//! The server half. It holds a set of trades and a set of locations and serves the map over their
//! product — and over each location on its own, which is the same product with the trade axis
//! extended by the identity.
//!
//! `LeptosOptions` is built here rather than read from `Cargo.toml`: this ships as a CLI that runs
//! from wherever the studies are, and `--port` has to win over a manifest it may never see.
//!
//! Building a study is the CLI's job, so it arrives as a closure: the web crate never learns what a
//! `.nix` file is, nor that a trade is a function of a location.
use std::{collections::HashMap, net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{
	Router,
	extract::{Path as UrlPath, Query},
	http::StatusCode,
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

/// `prebuilt` is `(trade stem, location stem, payload)` — whatever the CLI already paid for.
pub fn serve(trades: Vec<PathBuf>, locations: Vec<PathBuf>, prebuilt: Vec<(Option<String>, String, String)>, build: Build, find: Find, addr: SocketAddr, open: bool) -> Result<()> {
	// leptos spawns the SSR stream through `any_spawner`, which has no executor until it is told
	any_spawner::Executor::init_tokio().map_err(|e| eyre::eyre!("{e}"))?;
	let key = std::env::var("GOOGLE_MAPS_KEY").wrap_err("GOOGLE_MAPS_KEY is not set")?;
	let root = site_root();
	let pkg = root.join("pkg");
	eyre::ensure!(
		pkg.join("gmaps_optimal_placement_web.js").exists(),
		"no client build under {} — run `cargo leptos build` or nix's own",
		pkg.display()
	);

	let options = LeptosOptions::builder()
		.output_name("gmaps_optimal_placement_web")
		.site_root(root.to_string_lossy().into_owned())
		.site_addr(addr)
		.build();

	eyre::ensure!(
		!trades.is_empty() && !locations.is_empty(),
		"the product of {} trades and {} locations is empty",
		trades.len(),
		locations.len()
	);
	let stems = |v: &[PathBuf]| -> Vec<String> { v.iter().map(|p| p.file_stem().expect("a *.nix path has a stem").to_string_lossy().into_owned()).collect() };
	let (trade_stems, location_stems) = (stems(&trades), stems(&locations));

	// every location twice over: crossed with each trade, and on its own
	let mut pool: HashMap<Key, _> = locations
		.iter()
		.zip(&location_stems)
		.flat_map(|(l, ls)| {
			let bare = std::iter::once(((None, ls.clone()), (None, l.clone(), Arc::new(tokio::sync::Mutex::new(None)))));
			bare.chain(
				trades
					.iter()
					.zip(&trade_stems)
					.map(move |(t, ts)| ((Some(ts.clone()), ls.clone()), (Some(t.clone()), l.clone(), Arc::new(tokio::sync::Mutex::new(None))))),
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
	let shell = {
		let (options, title, key) = (options.clone(), title.clone(), key);
		move || crate::shell(options.clone(), title.clone(), key.clone())
	};

	let studies = serde_json::json!({ "trades": trade_stems, "locations": location_stems, "open": opened }).to_string();
	let (bare, again) = ((pool.clone(), build.clone()), (pool.clone(), build.clone()));
	let places = pool.clone();
	let mut app = Router::new()
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
		.nest_service("/pkg", tower_http::services::ServeDir::new(pkg));
	for (path, method) in leptos::server_fn::axum::server_fn_paths() {
		app = app.route(
			path,
			axum::routing::MethodRouter::new().on(
				axum::routing::MethodFilter::try_from(method).expect("leptos only registers routable methods"),
				leptos_axum::handle_server_fns,
			),
		);
	}
	let app = app
		.fallback(get(leptos_axum::render_app_to_stream(shell)))
		.layer(tower_http::compression::CompressionLayer::new())
		.with_state(options);

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
