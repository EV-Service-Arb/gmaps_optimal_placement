//! The server half: the page, the two axes, and a payload per pairing — each built by the library
//! from what the host's `Work` holds, as the conductor builds its own.
//!
//! `LeptosOptions` is built here rather than read from `Cargo.toml`: this ships inside the CLI,
//! which runs from wherever it is, and its `--port` has to win over a manifest it may never see.
use std::{collections::HashMap, net::SocketAddr, path::PathBuf, sync::Arc};

use axum::{Router, extract::Path as UrlPath, http::StatusCode, routing::get};
use eyre::{Result, WrapErr};
use gmaps_optimal_placement::Work;
use gmaps_optimal_placement_core::{Area, Category, Definition};
use leptos::prelude::*;

/// A `Work` per build: the CLI's holds a read-only sqlite connection, which does not cross threads.
pub type Works = Arc<dyn Fn() -> Result<Work> + Send + Sync>;

type Slot = Arc<tokio::sync::Mutex<Option<axum::body::Bytes>>>;

/// Keyed by the two labels the picker shows. One slot per pairing, so two tabs asking at once read
/// the 87 MB archive once between them.
type Pool = Arc<HashMap<(Option<String>, String), (Definition, Slot)>>;

/// `categories` and `locations` are the two axes, each by the label the picker shows. `site` is
/// where `wasm-bindgen` put the client, as `pkg/`.
pub fn serve(addr: SocketAddr, site: PathBuf, maps_key: String, categories: Vec<(String, Category)>, locations: Vec<(String, Area)>, works: Works) -> Result<()> {
	let pkg = site.join("pkg");
	let built = pkg.join("gmaps_optimal_placement_web.js");
	eyre::ensure!(built.exists(), "no client build at {} — the flake's default app builds it", built.display());
	eyre::ensure!(!locations.is_empty(), "no location to serve");

	let mut pool = HashMap::new();
	for (l, area) in &locations {
		for (c, category) in categories.iter().map(|(k, c)| (Some(k.clone()), Some(c.clone()))).chain([(None, None)]) {
			let def = Definition { area: area.clone(), category };
			eyre::ensure!(pool.insert((c, l.clone()), (def, Slot::default())).is_none(), "two different locations are both labelled {l:?}");
		}
	}
	let pool: Pool = Arc::new(pool);
	let axes = serde_json::json!({
		"categories": categories.iter().map(|(k, _)| k).collect::<Vec<_>>(),
		"locations": locations.iter().map(|(l, _)| l).collect::<Vec<_>>(),
	})
	.to_string();
	let (traded, bare) = ((pool.clone(), works.clone()), (pool, works));
	let api = Router::new()
		.route("/studies.json", get(move || std::future::ready(json(axes.clone()))))
		.route(
			"/payload/{category}/{location}",
			get(move |UrlPath((c, l)): UrlPath<(String, String)>| payload(traded.0.clone(), traded.1.clone(), (Some(c), l))),
		)
		.route("/place/{location}", get(move |UrlPath(l): UrlPath<String>| payload(bare.0.clone(), bare.1.clone(), (None, l))));

	// leptos spawns the SSR stream through `any_spawner`, which has no executor until it is told
	any_spawner::Executor::init_tokio().map_err(|e| eyre::eyre!("{e}"))?;
	let options = LeptosOptions::builder()
		.output_name("gmaps_optimal_placement_web")
		.site_root(site.to_string_lossy().into_owned())
		.site_addr(addr)
		.build();
	let shell = {
		let options = options.clone();
		move || shell(options.clone(), maps_key.clone())
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
	let app = api.merge(page).layer(tower_http::compression::CompressionLayer::new());

	let rt = tokio::runtime::Runtime::new().wrap_err("starting the server runtime")?;
	rt.block_on(async move {
		let listener = tokio::net::TcpListener::bind(&addr).await.wrap_err_with(|| format!("binding {addr}"))?;
		eprintln!("{} categories over {} locations are at http://{}", categories.len(), locations.len(), listener.local_addr()?);
		axum::serve(listener, app.into_make_service()).await.wrap_err("serving")
	})
}

/// Built on the first tab that asks for it, and kept. A failure is the eyre chain, which the page
/// banners.
async fn payload(pool: Pool, works: Works, at: (Option<String>, String)) -> Result<([(axum::http::HeaderName, &'static str); 1], axum::body::Bytes), (StatusCode, String)> {
	let Some((def, slot)) = pool.get(&at) else {
		return Err((StatusCode::NOT_FOUND, format!("{:?} is not a category over {:?}", at.0, at.1)));
	};
	let mut slot = slot.lock().await;
	if let Some(built) = &*slot {
		return Ok(json(built.clone()));
	}
	let def = def.clone();
	let shown = def.name();
	let built = tokio::task::spawn_blocking(move || -> Result<axum::body::Bytes> {
		let payload = gmaps_optimal_placement::load(&def, Vec::new())?.build(&works()?)?;
		Ok(serde_json::to_vec(&payload)?.into())
	})
	.await
	.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("building {shown}: {e}")))?
	// the chain and not `{e:?}`: the backtrace belongs in this terminal, not on the map
	.map_err(|e| {
		eprintln!("building {shown}: {e:?}");
		(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
	})?;
	*slot = Some(built.clone());
	Ok(json(built))
}

fn json<T>(body: T) -> ([(axum::http::HeaderName, &'static str); 1], T) {
	([(axum::http::header::CONTENT_TYPE, "application/json")], body)
}

/// `map_core.js` awaits `__mapsReady` before it touches `google.maps`, so the callback the Maps
/// bootstrap fires is a promise resolver rather than an entry point — nothing races the wasm.
fn shell(options: LeptosOptions, key: String) -> impl IntoView {
	let maps = format!("https://maps.googleapis.com/maps/api/js?key={key}&callback=__gmapsCb&loading=async");
	view! {
		<!DOCTYPE html>
		<html lang="en">
			<head>
				<meta charset="utf-8" />
				<meta name="viewport" content="width=device-width, initial-scale=1" />
				<title>"studies"</title>
				{leptos::html::style().inner_html(include_str!(concat!(env!("OUT_DIR"), "/style.css")))}
				{leptos::html::script()
					.inner_html("window.__mapsReady = new Promise(r => { window.__gmapsCb = r; });")}
				{leptos::html::script().attr("async", "").attr("src", maps)}
				<HydrationScripts options islands=true />
			</head>
			<body>
				<crate::map::MapView />
			</body>
		</html>
	}
}
