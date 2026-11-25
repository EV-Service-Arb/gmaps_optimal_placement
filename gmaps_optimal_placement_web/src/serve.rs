//! The server half: the page, the two axes, and a payload per pairing — each built by the library
//! from what the host's `Work` holds, as the conductor builds its own.
//!
//! `LeptosOptions` is built here rather than read from `Cargo.toml`: this ships inside the CLI,
//! which runs from wherever it is, and its `--port` has to win over a manifest it may never see.
use std::{collections::HashMap, convert::Infallible, net::SocketAddr, path::PathBuf, process::Stdio, sync::Arc, time::Duration};

use axum::{
	Json, Router,
	extract::Path as UrlPath,
	http::{HeaderMap, StatusCode, header},
	response::sse::{Event, Sse},
	routing::{get, post},
};
use eyre::{Result, WrapErr};
use futures_util::{Stream, stream};
use gmaps_optimal_placement::Work;
use gmaps_optimal_placement_core::{Area, Category, Definition};
use leptos::prelude::*;
use tokio::{
	sync::{Notify, watch},
	time::{sleep, timeout},
};
use v_utils::Timeframe;

use crate::run::{Again, Buyer};

/// A `Work` per build: the CLI's holds a read-only sqlite connection, which does not cross threads.
pub type Works = Arc<dyn Fn() -> Result<Work> + Send + Sync>;

type Slot = Arc<tokio::sync::Mutex<Option<axum::body::Bytes>>>;

/// Keyed by the two labels the picker shows. One slot per pairing, so two tabs asking at once read
/// the 87 MB archive once between them.
type Pool = Arc<HashMap<(Option<String>, String), (Definition, Slot)>>;

/// `categories` and `locations` are the two axes, each by the label the picker shows. `site` is
/// where `wasm-bindgen` put the client, as `pkg/`. `open` opens the page in the default browser, and
/// the server and the pages on it then end together. `certain`: a run is not confirmed, and the panel
/// may pull. `old`: how far back is, until the page says otherwise.
#[expect(clippy::too_many_arguments, reason = "the CLI's one call")]
pub fn serve(
	addr: SocketAddr,
	site: PathBuf,
	maps_key: String,
	categories: Vec<(String, Category)>,
	locations: Vec<(String, Area)>,
	works: Works,
	buyer: Arc<dyn Buyer>,
	certain: bool,
	old: Timeframe,
	open: bool,
) -> Result<()> {
	let pkg = site.join("pkg");
	let built = pkg.join("gmaps_optimal_placement_web.js");
	eyre::ensure!(built.exists(), "no client build at {} — the flake's `dev` app builds it", built.display());
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
		"certain": certain,
		"old": old,
	})
	.to_string();
	let ours = format!("http://{addr}");
	let (quoted, ran) = ((pool.clone(), buyer.clone(), ours.clone()), (pool.clone(), works.clone(), buyer, ours));
	let (traded, bare) = ((pool.clone(), works.clone()), (pool, works));
	let pages = open.then(Pages::default);
	let mut api = Router::new()
		.route("/studies.json", get(move || std::future::ready(json(axes.clone()))))
		.route(
			"/payload/{category}/{location}",
			get(move |UrlPath((c, l)): UrlPath<(String, String)>| payload(traded.0.clone(), traded.1.clone(), (Some(c), l))),
		)
		.route(
			"/quote/{category}/{location}",
			post(move |UrlPath((c, l)): UrlPath<(String, String)>, headers: HeaderMap| {
				let (pool, buyer, ours) = quoted.clone();
				async move {
					let (def, _) = spending(&pool, &ours, &headers, (Some(c), l))?;
					let quote = blocking(format!("pricing {}", def.name()), move || Ok(serde_json::to_vec(&buyer.quote(&def)?)?)).await?;
					Ok::<_, (StatusCode, String)>(json(quote))
				}
			}),
		)
		.route(
			"/run/{category}/{location}",
			post(move |UrlPath((c, l)): UrlPath<(String, String)>, headers: HeaderMap, Json(again): Json<Again>| {
				let (pool, works, buyer, ours) = ran.clone();
				async move {
					let (def, slot) = spending(&pool, &ours, &headers, (Some(c), l))?;
					let mut slot = slot.lock_owned().await;
					// the slot goes with it: a page that left does not stop a run
					let payload = blocking(format!("running {}", def.name()), move || {
						buyer.run(&def, again)?;
						let payload = build(&def, &works)?;
						// any other may share a tile with it; one locked is being built or run
						for (_, other) in pool.values() {
							if let Ok(mut other) = other.try_lock() {
								*other = None;
							}
						}
						*slot = Some(payload.clone());
						Ok(payload)
					})
					.await?;
					Ok::<_, (StatusCode, String)>(json(payload))
				}
			}),
		)
		.route("/place/{location}", get(move |UrlPath(l): UrlPath<String>| payload(bare.0.clone(), bare.1.clone(), (None, l))));
	if let Some(pages) = pages.clone() {
		api = api.route("/page", get(move || std::future::ready(pages.held())));
	}

	// leptos spawns the SSR stream through `any_spawner`, which has no executor until it is told
	any_spawner::Executor::init_tokio().map_err(|e| eyre::eyre!("{e}"))?;
	let options = LeptosOptions::builder()
		.output_name("gmaps_optimal_placement_web")
		.site_root(site.to_string_lossy().into_owned())
		.site_addr(addr)
		.build();
	let shell = {
		let options = options.clone();
		move || shell(options.clone(), maps_key.clone(), open)
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
	let ended = rt.block_on(async move {
		let listener = tokio::net::TcpListener::bind(&addr).await.wrap_err_with(|| format!("binding {addr}"))?;
		let url = format!("http://{}", listener.local_addr()?);
		eprintln!("{} categories over {} locations are at {url}", categories.len(), locations.len());
		let served = async { axum::serve(listener, app.into_make_service()).await.wrap_err("serving") };
		match pages {
			None => served.await,
			Some(pages) => tokio::select! {
				r = served => r,
				r = pages.ended(&url) => r,
			},
		}
	});
	rt.shutdown_background(); // dropping it would wait for a build under way
	ended
}

/// The pages open on a server that opened the first of them, each one a receiver.
#[derive(Clone, Default)]
struct Pages {
	close: watch::Sender<()>,
	opened: Arc<Notify>,
}

impl Pages {
	/// How long a reload may leave the server with no page.
	const RELOAD: Duration = Duration::from_secs(3);

	/// Held by a page for as long as it is open; an event on it is the word to close.
	fn held(&self) -> Sse<impl Stream<Item = Result<Event, Infallible>> + use<>> {
		let page = self.close.subscribe();
		self.opened.notify_one();
		Sse::new(stream::unfold(page, |mut page| async {
			page.changed().await.expect("the route holds a sender");
			Some((Ok(Event::default().data("close")), page))
		}))
	}

	/// Opens `url`, and returns once every page is closed, by the user or on SIGINT.
	async fn ended(&self, url: &str) -> Result<()> {
		let mut opener = tokio::process::Command::new("xdg-open")
			.arg(url)
			.stdin(Stdio::null())
			.process_group(0) // it may become the browser itself, which the terminal's SIGINT would end whole
			.spawn()
			.wrap_err("running xdg-open")?;
		let unopened = async {
			let status = opener.wait().await.wrap_err("waiting for xdg-open")?;
			eyre::ensure!(status.success(), "xdg-open {url}: {status}");
			std::future::pending().await
		};
		let left = async {
			self.opened.notified().await;
			loop {
				self.close.closed().await;
				sleep(Self::RELOAD).await;
				if self.close.receiver_count() == 0 {
					break;
				}
			}
		};
		tokio::select! {
			r = unopened => r,
			() = left => {
				eprintln!("closed in the browser");
				Ok(())
			}
			r = tokio::signal::ctrl_c() => {
				r.wrap_err("listening for SIGINT")?;
				self.close.send_replace(());
				// a browser lets a script close only a page with nothing behind it in its history
				if timeout(Duration::from_secs(2), self.close.closed()).await.is_err() {
					eprintln!("the browser keeps a page open");
				}
				Ok(())
			}
		}
	}
}

/// Built on the first tab that asks for it, and kept. A failure is the eyre chain, which the page
/// banners.
async fn payload(pool: Pool, works: Works, at: (Option<String>, String)) -> Result<([(axum::http::HeaderName, &'static str); 1], axum::body::Bytes), (StatusCode, String)> {
	let (def, slot) = served(&pool, &at)?;
	let mut slot = slot.lock().await;
	if let Some(built) = &*slot {
		return Ok(json(built.clone()));
	}
	let built = blocking(format!("building {}", def.name()), move || build(&def, &works)).await?;
	*slot = Some(built.clone());
	Ok(json(built))
}

fn served(pool: &Pool, at: &(Option<String>, String)) -> Result<(Definition, Slot), (StatusCode, String)> {
	pool.get(at)
		.cloned()
		.ok_or_else(|| (StatusCode::NOT_FOUND, format!("{:?} is not a category over {:?}", at.0, at.1)))
}

/// [`served`], to a request that may spend: any page the browser holds can send one here, under any
/// name that resolves to this machine, so it is taken from this page alone.
fn spending(pool: &Pool, ours: &str, headers: &HeaderMap, at: (Option<String>, String)) -> Result<(Definition, Slot), (StatusCode, String)> {
	if headers.get(header::ORIGIN).is_none_or(|o| o != ours) {
		return Err((StatusCode::FORBIDDEN, format!("a study is run from {ours}")));
	}
	served(pool, &at)
}

fn build(def: &Definition, works: &Works) -> Result<axum::body::Bytes> {
	let payload = gmaps_optimal_placement::load(def)?.build(&works()?)?;
	Ok(serde_json::to_vec(&payload)?.into())
}

/// `f` on a blocking thread; a failure is the eyre chain.
async fn blocking<T: Send + 'static>(what: String, f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T, (StatusCode, String)> {
	tokio::task::spawn_blocking(f)
		.await
		.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("{what}: {e}")))?
		// the chain and not `{e:?}`: the backtrace belongs in this terminal, not on the map
		.map_err(|e| {
			eprintln!("{what}: {e:?}");
			(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
		})
}

fn json<T>(body: T) -> ([(axum::http::HeaderName, &'static str); 1], T) {
	([(axum::http::header::CONTENT_TYPE, "application/json")], body)
}

/// `map_core.js` awaits `__mapsReady` before it touches `google.maps`, so the callback the Maps
/// bootstrap fires is a promise resolver rather than an entry point — nothing races the wasm.
fn shell(options: LeptosOptions, key: String, open: bool) -> impl IntoView {
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
				{open
					.then(|| {
						leptos::html::script()
							.inner_html("new EventSource('/page').onmessage = () => window.close();")
					})}
				{leptos::html::script().attr("async", "").attr("src", maps)}
				<HydrationScripts options islands=true />
			</head>
			<body>
				<crate::map::MapView />
			</body>
		</html>
	}
}
