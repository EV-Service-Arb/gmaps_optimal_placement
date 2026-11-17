//! The server half. gop's own page holds a set of categories and a set of locations and serves the
//! map over their product — and over each location on its own, which is the same product with the
//! category axis extended by the identity. Behind the conductor it holds the trades and the country
//! bases instead, and builds whatever definition the conductor's map hands in.
//!
//! `LeptosOptions` is built here rather than read from `Cargo.toml`: this ships as a CLI that runs
//! from wherever the studies are, and `--port` has to win over a manifest it may never see.
//!
//! Building a study is the CLI's job, so it arrives as a closure: the web crate never learns what a
//! `.nix` file is, nor that a trade is a function of an area.
//!
//! Behind the conductor there is no page: the conductor's own map imports `/mfe/overlay.js`, which is
//! open, and every other request carries the conductor's assertion for exactly itself.
//!
//! ```text
//!   GET  /trades.json            Read   {trades, countries}: what a category may be studied by, and where
//!   PUT  /studies/{sha}          Read   a Definition; 400 unless sha = its id, or it does not hold
//!   GET  /studies/{sha}          Read   the payload, built once; never spends
//!   POST /studies/{sha}/pull     Spend  ?budget=<n>: the payload, built again buying at most n Places
//!                                       searches; 402 {need, budget} when the sweep needs more.
//!                                       Every answer carries x-billed: what it bought, `{item: n}`.
//!                                       After one that did not end 2xx, buys only what it did not
//! ```
use std::{
	collections::HashMap,
	net::SocketAddr,
	path::PathBuf,
	sync::Arc,
	time::{SystemTime, UNIX_EPOCH},
};

use axum::{
	Router,
	extract::{Path as UrlPath, Query, Request, State},
	http::{Method, StatusCode},
	middleware::Next,
	response::{IntoResponse, Response},
	routing::{get, post},
};
use eyre::{Result, WrapErr};
use gmaps_optimal_placement_core::define::{Definition, Unaffordable};
use leptos::prelude::*;

/// A category file and a location, to the serialised `Payload` of pairing them, re-asking every
/// answer it is built from when told to. No category is a location on its own, which is the half of
/// the product that costs nothing to build.
pub type Build = Arc<dyn Fn(Option<&std::path::Path>, &std::path::Path, bool) -> Result<String> + Send + Sync>;

/// A location and a search, to the one place it names there: `[lat, lon]` and Google's address. The
/// CLI's for the same reason `Build` is, and because what it may spend is the CLI's to say.
pub type Find = Arc<dyn Fn(&std::path::Path, &str) -> Result<Option<([f64; 2], String)>> + Send + Sync>;

/// What a build bought, each priced item (`sa_core::prices`, `<tool>.<variant>`) with its count.
pub type Bought = std::collections::BTreeMap<String, usize>;

/// A definition, to the serialised `Payload` of building it, and what that bought — counted
/// whether it built or not, because it is paid either way. A pull's budget buys what the
/// work dir lacks; without one nothing is bought. `Err` before anything is read when the definition
/// does not hold.
pub type Built = Arc<dyn Fn(&Definition, Option<Pull>) -> (Bought, Result<String>) + Send + Sync>;

/// Whether a definition holds — its trade is served, its country has a base, the trade takes it —
/// without building it.
pub type Holds = Arc<dyn Fn(&Definition) -> Result<()> + Send + Sync>;

type Slot = Arc<tokio::sync::Mutex<Option<axum::body::Bytes>>>;

/// One slot per pairing, so two tabs asking at once read the 87 MB archive once between them. Keyed
/// by the two stems rather than by the study's name: what a pairing is called is the CLI's to say,
/// and a second spelling of it here is a second thing to keep in step with the pin file.
type Key = (Option<String>, String);
type Pool = Arc<HashMap<Key, (Option<PathBuf>, PathBuf, Slot)>>;

/// One served location: its file, and what the page places it on a map by.
pub struct Location {
	pub path: PathBuf,
	/// West, south, east, north.
	pub bbox: [f64; 4],
	/// ISO 3166-1 alpha-2.
	pub country: String,
}

pub enum Mode {
	/// gop's own page. `key` is the one the browser loads Google's script with; `prebuilt` is
	/// `(category stem, location stem, payload)` — whatever the CLI already paid for.
	Page {
		key: String,
		open: bool,
		categories: Vec<PathBuf>,
		locations: Vec<Location>,
		prebuilt: Vec<(Option<String>, String, String)>,
		build: Build,
		find: Find,
	},
	/// Behind the conductor, which signs every request it forwards with one of `keys`. `catalog` is
	/// `/trades.json`'s body; `studies` the directory every definition handed in is kept under.
	Conductor {
		keys: sa_auth::Keys,
		catalog: String,
		studies: PathBuf,
		holds: Holds,
		build: Built,
	},
}

pub fn serve(addr: SocketAddr, mode: Mode) -> Result<()> {
	let root = site_root();
	let (pkg, mfe) = (root.join("pkg"), root.join("mfe"));
	let built = match &mode {
		Mode::Page { .. } => pkg.join("gmaps_optimal_placement_web.js"),
		Mode::Conductor { .. } => mfe.join("overlay.js"),
	};
	eyre::ensure!(built.exists(), "no client build at {} — nix's `open`/`conductor` apps build it", built.display());

	let (app, open, title) = match mode {
		Mode::Page {
			key,
			open,
			categories,
			locations,
			prebuilt,
			build,
			find,
		} => {
			let (api, title) = page_api(categories, locations, prebuilt, build, find)?;
			// leptos spawns the SSR stream through `any_spawner`, which has no executor until it is told
			any_spawner::Executor::init_tokio().map_err(|e| eyre::eyre!("{e}"))?;
			let options = LeptosOptions::builder()
				.output_name("gmaps_optimal_placement_web")
				.site_root(root.to_string_lossy().into_owned())
				.site_addr(addr)
				.build();
			let shell = {
				let (options, title) = (options.clone(), title.clone());
				move || shell(options.clone(), title.clone(), key.clone())
			};
			let mut page = Router::new().nest_service("/pkg", tower_http::services::ServeDir::new(pkg));
			for (path, method) in leptos::server_fn::axum::server_fn_paths() {
				page = page.route(
					path,
					axum::routing::MethodRouter::new().on(
						axum::routing::MethodFilter::try_from(method).expect("leptos only registers routable methods"),
						leptos_axum::handle_server_fns,
					),
				);
			}
			let page = page.fallback(get(leptos_axum::render_app_to_stream(shell))).with_state(options);
			(api.nest_service("/mfe", tower_http::services::ServeDir::new(mfe)).merge(page), open, title)
		}
		Mode::Conductor {
			keys,
			catalog,
			studies,
			holds,
			build,
		} => {
			std::fs::create_dir_all(&studies).wrap_err_with(|| format!("creating {}", studies.display()))?;
			let api = conductor_api(catalog, studies, holds, build)
				.layer(axum::middleware::from_fn_with_state(Arc::new(keys), guard))
				.nest_service("/mfe", tower_http::services::ServeDir::new(mfe));
			(api, false, "studies behind the conductor".to_owned())
		}
	};
	let app = app.layer(tower_http::compression::CompressionLayer::new());

	let rt = tokio::runtime::Runtime::new().wrap_err("starting the server runtime")?;
	rt.block_on(async move {
		let listener = tokio::net::TcpListener::bind(&addr).await.wrap_err_with(|| format!("binding {addr}"))?;
		let url = format!("http://{}", listener.local_addr()?);
		eprintln!("{title} is at {url}");
		if open {
			// not being able to reach a browser is not a reason to stop serving
			if let Err(e) = std::process::Command::new("xdg-open").arg(&url).spawn() {
				eprintln!("xdg-open: {e}");
			}
		}
		axum::serve(listener, app.into_make_service()).await.wrap_err("serving")
	})
}

/// The page's product: `/studies.json`, `/payload/{category}/{location}`, `/place/{location}` and
/// `/find/{location}`.
fn page_api(categories: Vec<PathBuf>, locations: Vec<Location>, prebuilt: Vec<(Option<String>, String, String)>, build: Build, find: Find) -> Result<(Router, String)> {
	eyre::ensure!(
		!categories.is_empty() && !locations.is_empty(),
		"the product of {} categories and {} locations is empty",
		categories.len(),
		locations.len()
	);
	let stem = |p: &PathBuf| -> String { p.file_stem().expect("a study file has a stem").to_string_lossy().into_owned() };
	let category_stems: Vec<String> = categories.iter().map(stem).collect();
	let location_stems: Vec<String> = locations.iter().map(|l| stem(&l.path)).collect();

	// every location twice over: crossed with each category, and on its own
	let mut pool: HashMap<Key, _> = locations
		.iter()
		.zip(&location_stems)
		.flat_map(|(l, ls)| {
			let bare = std::iter::once(((None, ls.clone()), (None, l.path.clone(), Slot::default())));
			bare.chain(
				categories
					.iter()
					.zip(&category_stems)
					.map(move |(c, cs)| ((Some(cs.clone()), ls.clone()), (Some(c.clone()), l.path.clone(), Slot::default()))),
			)
		})
		.collect();
	let opened: Vec<Key> = prebuilt.iter().map(|(t, l, _)| (t.clone(), l.clone())).collect();
	for (t, l, json) in prebuilt {
		let (_, _, slot) = pool
			.get_mut(&(t.clone(), l.clone()))
			.ok_or_else(|| eyre::eyre!("{t:?} x {l:?} was built but is not in the product"))?;
		let slot = Arc::get_mut(slot).expect("nothing has seen the pool yet").get_mut();
		assert!(slot.replace(axum::body::Bytes::from(json)).is_none(), "each study is prebuilt at most once");
	}
	let pool: Pool = Arc::new(pool);

	let title = match (category_stems.as_slice(), location_stems.as_slice()) {
		([c], [l]) => format!("{c} in {l}"),
		(c, l) => format!("{} categories over {} locations", c.len(), l.len()),
	};

	let served: Vec<serde_json::Value> = locations
		.iter()
		.zip(&location_stems)
		.map(|(l, name)| serde_json::json!({"name": name, "bbox": l.bbox, "country": l.country}))
		.collect();
	let studies = serde_json::json!({ "categories": category_stems, "locations": served, "open": opened }).to_string();
	let (bare, again) = ((pool.clone(), build.clone()), (pool.clone(), build.clone()));
	let places = pool.clone();
	let api = Router::new()
		.route("/studies.json", get(move || std::future::ready(json(studies.clone()))))
		.route(
			"/payload/{category}/{location}",
			get(move |UrlPath((t, l)): UrlPath<(String, String)>| payload(pool.clone(), build.clone(), (Some(t), l), false))
				.post(move |UrlPath((t, l)): UrlPath<(String, String)>| payload(again.0.clone(), again.1.clone(), (Some(t), l), true)),
		)
		.route(
			"/place/{location}",
			get(move |UrlPath(l): UrlPath<String>| payload(bare.0.clone(), bare.1.clone(), (None, l), false)),
		)
		.route(
			"/find/{location}",
			get(move |UrlPath(l): UrlPath<String>, Query(q): Query<Search>| self::find(places.clone(), find.clone(), l, q.q)),
		);
	Ok((api, title))
}

/// What the conductor's map reads. A definition is kept on disk once handed in, so a restart serves
/// it again and `fit` can read what it collected; its payload is kept in memory once built.
fn conductor_api(catalog: String, studies: PathBuf, holds: Holds, build: Built) -> Router {
	// shortcut: unbounded, one payload per area × category any operator opened; an LRU when the process grows
	let built: Arc<std::sync::Mutex<HashMap<String, Slot>>> = Arc::default();
	let slot = move |sha: &str| -> Slot { built.lock().expect("no holder panics").entry(sha.to_owned()).or_default().clone() };
	let (put_dir, get_dir, pull_dir) = (studies.clone(), studies.clone(), studies);
	let (get_slot, pull_slot) = (slot.clone(), slot);
	let (get_build, pull_build) = (build.clone(), build);
	Router::new()
		.route("/trades.json", get(move || std::future::ready(json(catalog.clone()))))
		.route(
			"/studies/{sha}",
			get(move |UrlPath(sha): UrlPath<String>| {
				let (dir, slot, build) = (get_dir.clone(), get_slot(&sha), get_build.clone());
				async move {
					let (billed, built) = study(&dir, &sha, slot, build, None).await;
					assert!(billed.is_empty(), "a GET is built with no budget: {billed:?}");
					built.map(json)
				}
			})
			.put(move |UrlPath(sha): UrlPath<String>, body: axum::body::Bytes| define(put_dir.clone(), holds.clone(), sha, body)),
		)
		.route(
			"/studies/{sha}/pull",
			post(move |UrlPath(sha): UrlPath<String>, pull: Result<Query<Budget>, axum::extract::rejection::QueryRejection>| {
				let (dir, slot, build) = (pull_dir.clone(), pull_slot(&sha), pull_build.clone());
				async move {
					let (billed, built) = match pull {
						Ok(Query(Budget { budget })) => study(&dir, &sha, slot, build, Some(budget)).await,
						Err(e) => (Bought::new(), Err(e.into_response())),
					};
					let billed = serde_json::to_string(&billed).expect("names and counts serialise");
					([(axum::http::HeaderName::from_static("x-billed"), billed)], built.map(json))
				}
			}),
		)
}

/// A definition, checked against the key it was put under and against what is served, and kept.
async fn define(dir: PathBuf, holds: Holds, sha: String, body: axum::body::Bytes) -> Result<StatusCode, (StatusCode, String)> {
	let bad = |e: String| (StatusCode::BAD_REQUEST, e);
	let def: Definition = serde_json::from_slice(&body).map_err(|e| bad(format!("not a definition: {e}")))?;
	let id = def.id();
	if id != sha {
		return Err(bad(format!("the definition's id is {id}, not {sha}")));
	}
	let canonical = serde_json::to_vec(&def.clone().canonical()).expect("a definition serialises");
	let path = dir.join(format!("{sha}.json"));
	tokio::task::spawn_blocking(move || {
		holds(&def).map_err(|e| bad(format!("{e:#}")))?;
		// renamed into place: a reader never sees half a definition
		let part = path.with_extension("part");
		std::fs::write(&part, &canonical)
			.and_then(|()| std::fs::rename(&part, &path))
			.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("keeping {}: {e}", path.display())))
	})
	.await
	.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("checking {sha}: {e}")))??;
	Ok(StatusCode::NO_CONTENT)
}

#[derive(serde::Deserialize)]
struct Budget {
	budget: usize,
}

/// What a pull may buy, and the answers it re-asks: those written before `since`.
pub struct Pull {
	/// Places searches.
	pub budget: usize,
	pub since: SystemTime,
}

/// When the first pull of a study since its last whole one began, kept beside its definition until
/// one ends whole: a pull that stops part-way, or never ends, has paid for what it already bought.
fn began(resume: &std::path::Path) -> std::io::Result<SystemTime> {
	match std::fs::read_to_string(resume) {
		Ok(s) => s
			.parse()
			.map(|ns: u64| UNIX_EPOCH + std::time::Duration::from_nanos(ns))
			.map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{} is not nanoseconds: {e}", resume.display()))),
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
			let now = SystemTime::now();
			let part = resume.with_extension("resume.part");
			std::fs::write(&part, now.duration_since(UNIX_EPOCH).expect("the clock is past 1970").as_nanos().to_string())?;
			std::fs::rename(&part, resume)?;
			Ok(now)
		}
		Err(e) => Err(e),
	}
}

/// Built on the first GET and kept. A pull re-asks Google everything it is built from, and that
/// answer goes to the puller alone: one stopped part-way by the day's quota is a map with no
/// competitors, while the work dir now holds every answer it paid for, so the next GET builds from
/// that, and the next pull buys only what this one did not. Beside it, what it bought, which is what
/// a pull is charged.
async fn study(dir: &std::path::Path, sha: &str, slot: Slot, build: Built, budget: Option<usize>) -> (Bought, Result<axum::body::Bytes, Response>) {
	let path = dir.join(format!("{sha}.json"));
	let mut slot = slot.lock().await;
	if budget.is_some() {
		*slot = None;
	}
	if let Some(built) = &*slot {
		return (Bought::new(), Ok(built.clone()));
	}
	let def: Definition = match std::fs::read(&path).map(|b| serde_json::from_slice(&b)) {
		Ok(Ok(def)) => def,
		Ok(Err(e)) =>
			return (
				Bought::new(),
				Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{} is not a definition: {e}", path.display())).into_response()),
			),
		Err(e) if e.kind() == std::io::ErrorKind::NotFound =>
			return (
				Bought::new(),
				Err((StatusCode::NOT_FOUND, format!("no study {sha} was defined here: PUT it first")).into_response()),
			),
		Err(e) =>
			return (
				Bought::new(),
				Err((StatusCode::INTERNAL_SERVER_ERROR, format!("reading {}: {e}", path.display())).into_response()),
			),
	};
	let name = def.name();
	let resume = dir.join(format!("{sha}.resume"));
	let pull = match budget.map(|budget| began(&resume).map(|since| Pull { budget, since })).transpose() {
		Ok(pull) => pull,
		Err(e) => return (Bought::new(), Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{}: {e}", resume.display())).into_response())),
	};
	// a panicked build took what it bought with it, and no answer is better than an x-billed that guesses
	let (billed, built) = tokio::task::spawn_blocking(move || build(&def, pull))
		.await
		.unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()));
	let built = match built {
		Ok(b) => axum::body::Bytes::from(b),
		Err(e) => {
			eprintln!("building {name}: {e:?}");
			let refused = match e.downcast_ref::<Unaffordable>() {
				Some(u) => (StatusCode::PAYMENT_REQUIRED, json(serde_json::to_string(u).expect("two integers serialise"))).into_response(),
				None => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")).into_response(),
			};
			return (billed, Err(refused));
		}
	};
	match budget {
		None => *slot = Some(built.clone()),
		Some(_) =>
			if let Err(e) = std::fs::remove_file(&resume) {
				return (
					billed,
					Err((StatusCode::INTERNAL_SERVER_ERROR, format!("{} outlived a whole pull: {e}", resume.display())).into_response()),
				);
			},
	}
	(billed, Ok(built))
}

/// Every request but the overlay build carries the conductor's assertion for exactly its method and
/// path, made for a person who may read studies — and, for a pull, spend on them. A PUT defines a
/// study and buys nothing.
async fn guard(State(keys): State<Arc<sa_auth::Keys>>, req: Request, next: Next) -> Response {
	let path = req.uri().path();
	if path.starts_with("/mfe/") {
		return next.run(req).await;
	}
	let Some(token) = req.headers().get(sa_auth::HEADER) else {
		return (StatusCode::UNAUTHORIZED, format!("no {} on {path}", sa_auth::HEADER)).into_response();
	};
	let Ok(token) = token.to_str() else {
		return (StatusCode::UNAUTHORIZED, format!("{} is not ASCII", sa_auth::HEADER)).into_response();
	};
	let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("the clock is past 1970").as_secs() as i64;
	let assertion = match sa_auth::verify(&keys, token, sa_auth::Service::Placement, req.method().as_str(), path, None, now) {
		Ok(a) => a,
		Err(e) => return (StatusCode::UNAUTHORIZED, format!("{}: {e}", sa_auth::HEADER)).into_response(),
	};
	let need = match *req.method() == Method::POST && path.ends_with("/pull") {
		true => sa_auth::Study::Spend,
		false => sa_auth::Study::Read,
	};
	match assertion.principal {
		sa_auth::Principal::User(u) if u.permissions.may(need) => next.run(req).await,
		sa_auth::Principal::User(u) => (StatusCode::FORBIDDEN, format!("{} does not hold {}", u.email, need.as_str())).into_response(),
		other => (StatusCode::FORBIDDEN, format!("studies are a person's to read, not {other:?}'s")).into_response(),
	}
}

/// Built on the first tab that asks for it, and kept until one asks for it again, which re-asks Google
/// everything it is built from. That answer goes to the asking tab alone: a refresh stopped part-way
/// by the day's quota is a map with no competitors on it, while the work dir under it now holds
/// every answer either sweep paid for, so the next tab to ask builds from that. A failure is the eyre
/// chain, which the page banners.
async fn payload(pool: Pool, build: Build, at: Key, refresh: bool) -> Result<([(axum::http::HeaderName, &'static str); 1], axum::body::Bytes), (StatusCode, String)> {
	let Some((trade, location, slot)) = pool.get(&at) else {
		return Err((StatusCode::NOT_FOUND, format!("{:?} is not a trade over {:?}", at.0, at.1)));
	};
	let mut slot = slot.lock().await;
	if refresh {
		*slot = None;
	}
	if let Some(built) = &*slot {
		return Ok(json(built.clone()));
	}
	let (trade, location) = (trade.clone(), location.clone());
	let shown = match &trade {
		Some(t) => format!("{} x {}", t.display(), location.display()),
		None => location.display().to_string(),
	};
	let built = tokio::task::spawn_blocking(move || build(trade.as_deref(), &location, refresh).map(axum::body::Bytes::from))
		.await
		.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("building {shown}: {e}")))?
		// the chain and not `{e:?}`: the backtrace belongs in this terminal, and a page that hands
		// one to whoever opened a city has stopped being a map
		.map_err(|e| {
			eprintln!("building {shown}: {e:?}");
			(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
		})?;
	if !refresh {
		*slot = Some(built.clone());
	}
	Ok(json(built))
}

#[derive(serde::Deserialize)]
struct Search {
	q: String,
}

/// `null` when Google names nothing. A refusal — a server started without `--spend-real-quota-i-am-certain` above all — is the eyre chain, which
/// the place field shows.
async fn find(pool: Pool, find: Find, location: String, q: String) -> Result<([(axum::http::HeaderName, &'static str); 1], String), (StatusCode, String)> {
	let Some((_, path, _)) = pool.get(&(None, location.clone())) else {
		return Err((StatusCode::NOT_FOUND, format!("{location:?} is not a location served here")));
	};
	let path = path.clone();
	let found = tokio::task::spawn_blocking(move || find(&path, &q))
		.await
		.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("finding a place: {e}")))?
		.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}")))?;
	Ok(json(serde_json::json!(found.map(|(at, address)| serde_json::json!({"at": at, "address": address}))).to_string()))
}

fn json<T>(body: T) -> ([(axum::http::HeaderName, &'static str); 1], T) {
	([(axum::http::header::CONTENT_TYPE, "application/json")], body)
}

/// Where `wasm-bindgen` put the client. The nix wrapper sets it; a dev shell gets `cargo`'s own.
fn site_root() -> PathBuf {
	std::env::var_os("LEPTOS_SITE_ROOT").map_or_else(|| PathBuf::from("target/site"), PathBuf::from)
}

/// `map_core.js` awaits `__mapsReady` before it touches `google.maps`, so the callback the Maps
/// bootstrap fires is a promise resolver rather than an entry point — nothing races the wasm.
fn shell(options: LeptosOptions, study: String, key: String) -> impl IntoView {
	let maps = format!("https://maps.googleapis.com/maps/api/js?key={key}&callback=__gmapsCb&loading=async");
	view! {
		<!DOCTYPE html>
		<html lang="en">
			<head>
				<meta charset="utf-8" />
				<meta name="viewport" content="width=device-width, initial-scale=1" />
				<title>{study}</title>
				{leptos::html::style().inner_html(include_str!(concat!(env!("OUT_DIR"), "/style.css")))}
				{leptos::html::script()
					.inner_html("window.__mapsReady = new Promise(r => { window.__gmapsCb = r; });")}
				{leptos::html::script().attr("async", "").attr("src", maps)}
				<AutoReload options=options.clone() />
				<HydrationScripts options islands=true />
			</head>
			<body>
				<crate::map::MapView />
			</body>
		</html>
	}
}
