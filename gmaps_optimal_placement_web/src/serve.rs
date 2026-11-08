//! The server half. It holds a set of trades and a set of locations and serves the map over their
//! product — and over each location on its own, which is the same product with the trade axis
//! extended by the identity.
//!
//! `LeptosOptions` is built here rather than read from `Cargo.toml`: this ships as a CLI that runs
//! from wherever the studies are, and `--port` has to win over a manifest it may never see.
//!
//! Building a study is the CLI's job, so it arrives as a closure: the web crate never learns what a
//! `.nix` file is, nor that a trade is a function of a location.
//!
//! Behind the conductor there is no page: the conductor's own map imports `/mfe/overlay.js`, which is
//! open, and every other request carries the conductor's assertion for exactly itself.
use std::{collections::HashMap, net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{
	Router,
	extract::{Path as UrlPath, Query, Request, State},
	http::{Method, StatusCode},
	middleware::Next,
	response::{IntoResponse, Response},
	routing::get,
};
use eyre::{Result, WrapErr};
use leptos::prelude::*;

/// A trade and a location, to the serialised `Payload` of pairing them, re-asking every answer it is
/// built from when told to. No trade is a location on its own, which is the half of the product that
/// costs nothing to build.
pub type Build = Arc<dyn Fn(Option<&std::path::Path>, &std::path::Path, bool) -> Result<String> + Send + Sync>;

/// A location and a search, to the one place it names there: `[lat, lon]` and Google's address. The
/// CLI's for the same reason `Build` is, and because what it may spend is the CLI's to say.
pub type Find = Arc<dyn Fn(&std::path::Path, &str) -> Result<Option<([f64; 2], String)>> + Send + Sync>;

/// One slot per pairing, so two tabs asking at once read the 87 MB archive once between them. Keyed
/// by the two stems rather than by the study's name: what a pairing is called is the CLI's to say,
/// and a second spelling of it here is a second thing to keep in step with the pin file.
type Key = (Option<String>, String);
type Pool = Arc<HashMap<Key, (Option<PathBuf>, PathBuf, Arc<tokio::sync::Mutex<Option<axum::body::Bytes>>>)>>;

/// One served location: its file, and what the conductor places it on a map by.
pub struct Location {
	pub path: PathBuf,
	/// West, south, east, north.
	pub bbox: [f64; 4],
	/// ISO 3166-1 alpha-2.
	pub country: String,
}

pub enum Mode {
	/// gop's own page. `key` is the one the browser loads Google's script with.
	Page { key: String, open: bool },
	/// Behind the conductor, which signs every request it forwards with one of these.
	Conductor(sa_auth::Keys),
}

/// `prebuilt` is `(trade stem, location stem, payload)` — whatever the CLI already paid for.
pub fn serve(trades: Vec<PathBuf>, locations: Vec<Location>, prebuilt: Vec<(Option<String>, String, String)>, build: Build, find: Find, addr: SocketAddr, mode: Mode) -> Result<()> {
	let root = site_root();
	let (pkg, mfe) = (root.join("pkg"), root.join("mfe"));
	let built = match &mode {
		Mode::Page { .. } => pkg.join("gmaps_optimal_placement_web.js"),
		Mode::Conductor(_) => mfe.join("overlay.js"),
	};
	eyre::ensure!(built.exists(), "no client build at {} — nix's `open`/`conductor` apps build it", built.display());

	eyre::ensure!(
		!trades.is_empty() && !locations.is_empty(),
		"the product of {} trades and {} locations is empty",
		trades.len(),
		locations.len()
	);
	let stem = |p: &PathBuf| -> String { p.file_stem().expect("a *.nix path has a stem").to_string_lossy().into_owned() };
	let trade_stems: Vec<String> = trades.iter().map(stem).collect();
	let location_stems: Vec<String> = locations.iter().map(|l| stem(&l.path)).collect();

	// every location twice over: crossed with each trade, and on its own
	let mut pool: HashMap<Key, _> = locations
		.iter()
		.zip(&location_stems)
		.flat_map(|(l, ls)| {
			let bare = std::iter::once(((None, ls.clone()), (None, l.path.clone(), Arc::new(tokio::sync::Mutex::new(None)))));
			bare.chain(
				trades
					.iter()
					.zip(&trade_stems)
					.map(move |(t, ts)| ((Some(ts.clone()), ls.clone()), (Some(t.clone()), l.path.clone(), Arc::new(tokio::sync::Mutex::new(None))))),
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

	let title = match (trade_stems.as_slice(), location_stems.as_slice()) {
		([t], [l]) => format!("{t} in {l}"),
		(t, l) => format!("{} trades over {} locations", t.len(), l.len()),
	};

	let served: Vec<serde_json::Value> = locations
		.iter()
		.zip(&location_stems)
		.map(|(l, name)| serde_json::json!({"name": name, "bbox": l.bbox, "country": l.country}))
		.collect();
	let studies = serde_json::json!({ "trades": trade_stems, "locations": served, "open": opened }).to_string();
	let (bare, again) = ((pool.clone(), build.clone()), (pool.clone(), build.clone()));
	let places = pool.clone();
	let api = Router::new()
		.route("/studies.json", get(move || std::future::ready(json(studies.clone()))))
		.route(
			"/payload/{trade}/{location}",
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
		)
		.nest_service("/mfe", tower_http::services::ServeDir::new(mfe));

	let (app, open) = match mode {
		Mode::Page { key, open } => {
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
			(api.merge(page), open)
		}
		Mode::Conductor(keys) => (api.layer(axum::middleware::from_fn_with_state(Arc::new(keys), guard)), false),
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

/// Every request but the overlay build carries the conductor's assertion for exactly its method and
/// path, made for a person who may read studies — and, for anything but a GET, spend on them.
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
	let need = match *req.method() == Method::GET {
		true => sa_auth::Study::Read,
		false => sa_auth::Study::Spend,
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

/// `null` when Google names nothing. A refusal — `--block-spend` above all — is the eyre chain, which
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
