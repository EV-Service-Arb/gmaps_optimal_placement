//! `view`'s page on 127.0.0.1: google.maps, the overlay, and a thin vanilla host over one payload.
//! The host renders what the overlay tells it; every number is the overlay's.
use std::{path::PathBuf, sync::Arc};

use axum::{
	Router,
	extract::{Path, State},
	http::{StatusCode, header},
	response::{Html, IntoResponse, Response},
	routing::get,
};
use eyre::{Result, WrapErr};

pub fn serve(payload: Vec<u8>, overlay: &std::path::Path, key: &str, port: u16) -> Result<()> {
	eyre::ensure!(
		overlay.join("overlay.js").is_file(),
		"{} holds no overlay.js: --overlay is wasm-bindgen's output (`nix build .#overlay`)",
		overlay.display()
	);
	let page = Page {
		html: Arc::new(include_str!("view.html").replace("{{MAPS_KEY}}", &serde_json::to_string(key)?)),
		payload: Arc::new(payload),
		overlay: Arc::new(overlay.to_owned()),
	};
	let app = Router::new()
		.route("/", get(|State(p): State<Page>| async move { Html(p.html.as_str().to_owned()) }))
		.route(
			"/payload.json",
			get(|State(p): State<Page>| async move { ([(header::CONTENT_TYPE, "application/json")], p.payload.as_slice().to_vec()) }),
		)
		.route("/overlay/{*file}", get(file))
		.with_state(page);
	tokio::runtime::Runtime::new().wrap_err("starting the viewer's runtime")?.block_on(async move {
		let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.wrap_err_with(|| format!("binding 127.0.0.1:{port}"))?;
		eprintln!("open http://127.0.0.1:{port}");
		axum::serve(listener, app).await.wrap_err("serving the viewer")
	})
}

#[derive(Clone)]
struct Page {
	html: Arc<String>,
	payload: Arc<Vec<u8>>,
	/// wasm-bindgen's output for `gmaps_optimal_placement_overlay`.
	overlay: Arc<PathBuf>,
}

/// One of the overlay's files, never one above its directory.
async fn file(State(p): State<Page>, Path(file): Path<String>) -> Response {
	if file.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
		return StatusCode::NOT_FOUND.into_response();
	}
	let kind = match file.rsplit('.').next() {
		Some("js") => "text/javascript",
		Some("wasm") => "application/wasm",
		_ => "application/octet-stream",
	};
	match std::fs::read(p.overlay.join(&file)) {
		Ok(bytes) => ([(header::CONTENT_TYPE, kind)], bytes).into_response(),
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => StatusCode::NOT_FOUND.into_response(),
		Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("reading {file}: {e}")).into_response(),
	}
}
