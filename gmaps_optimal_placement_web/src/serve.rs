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
//! Behind the conductor there is no page: the conductor's own map imports the overlay under `/mfe/`,
//! which is open, and every other request is an operation of [`crate::studies`]' table, behind the
//! conductor's assertion for exactly itself.
use std::{collections::HashMap, net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{
	Router,
	extract::{Path as UrlPath, Query},
	http::StatusCode,
	routing::get,
};
use eyre::{Result, WrapErr};
use leptos::prelude::*;
use sa_core::{gbp::Country, placement::Category};

/// A category and a location file, to the serialised `Payload` of pairing them, re-asking every
/// answer it is built from when told to. No category is a location on its own, which is the half of
/// the product that costs nothing to build.
pub type Build = Arc<dyn Fn(Option<&Category>, &std::path::Path, bool) -> Result<String> + Send + Sync>;

/// A location and a search, to the one place it names there: `[lat, lon]` and Google's address. The
/// CLI's for the same reason `Build` is, and because what it may spend is the CLI's to say.
pub type Find = Arc<dyn Fn(&std::path::Path, &str) -> Result<Option<([f64; 2], String)>> + Send + Sync>;

type Slot = Arc<tokio::sync::Mutex<Option<axum::body::Bytes>>>;

/// One slot per pairing, so two tabs asking at once read the 87 MB archive once between them. Keyed
/// by the category's key and the location's stem rather than by the study's name: what a pairing is
/// called is the CLI's to say, and a second spelling of it here is a second thing to keep in step
/// with the pin file.
type Key = (Option<String>, String);
type Pool = Arc<HashMap<Key, (Option<Category>, PathBuf, Slot)>>;

/// One served location: its file, and what the page places it on a map by.
pub struct Location {
	pub path: PathBuf,
	/// West, south, east, north.
	pub bbox: [f64; 4],
	pub country: Country,
}

pub enum Mode {
	/// gop's own page. `key` is the one the browser loads Google's script with; `categories` the
	/// conductor's, by the key the picker shows; `prebuilt` is `(category key, location stem,
	/// payload)` — whatever the CLI already paid for.
	Page {
		key: String,
		open: bool,
		categories: Vec<(String, Category)>,
		locations: Vec<Location>,
		prebuilt: Vec<(Option<String>, String, String)>,
		build: Build,
		find: Find,
	},
	/// Behind the conductor.
	Conductor(crate::studies::Studies),
}

pub fn serve(addr: SocketAddr, mode: Mode) -> Result<()> {
	let root = site_root();
	let (pkg, mfe) = (root.join("pkg"), root.join("mfe"));
	let built = match &mode {
		Mode::Page { .. } => pkg.join("gmaps_optimal_placement_web.js"),
		Mode::Conductor(_) => mfe.join("gmaps_optimal_placement_overlay.js"),
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
		Mode::Conductor(studies) => {
			let api = crate::studies::router(studies)?.nest_service("/mfe", tower_http::services::ServeDir::new(mfe));
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
fn page_api(categories: Vec<(String, Category)>, locations: Vec<Location>, prebuilt: Vec<(Option<String>, String, String)>, build: Build, find: Find) -> Result<(Router, String)> {
	eyre::ensure!(!locations.is_empty(), "no location to serve");
	let stem = |p: &PathBuf| -> String { p.file_stem().expect("a study file has a stem").to_string_lossy().into_owned() };
	let category_stems: Vec<String> = categories.iter().map(|(k, _)| k.clone()).collect();
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
					.map(move |(cs, c)| ((Some(cs.clone()), ls.clone()), (Some(c.clone()), l.path.clone(), Slot::default()))),
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
		Some(t) => format!("{}·{} x {}", t.study, t.country, location.display()),
		None => location.display().to_string(),
	};
	let built = tokio::task::spawn_blocking(move || build(trade.as_ref(), &location, refresh).map(axum::body::Bytes::from))
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
