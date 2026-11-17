//! What the conductor asks: one table of operations behind its assertion, the router and the
//! document both derived from it (`ev_lib::api`). A definition comes in with every ask and its
//! payload goes out; it is kept on disk under its id, so `fit` can read what it collected, and its
//! payload in memory once built. Nothing here buys: a pull is the conductor asking what a study
//! lacks, buying it into the queries cache, until it lacks nothing, then asking it built again.
use std::{
	collections::HashMap,
	net::SocketAddr,
	path::PathBuf,
	sync::{Arc, Mutex},
	time::{SystemTime, UNIX_EPOCH},
};

use axum::{
	Json, Router,
	body::Bytes,
	extract::{FromRef, State},
	http::{Method, StatusCode, header},
	response::{IntoResponse, Response},
};
use ev_lib::api::{
	Body, ErrorCode, GenContext, OperationOutput, Refusal, Refuses, Route,
	openapi::{self, Operation},
};
use eyre::WrapErr;
use gmaps_optimal_placement_core::Payload;
use sa_auth::{Gate, Study, Verifier};
use sa_core::{
	Invalid,
	placement::{Ask, Asks, Catalog, Definition},
};
use serde::Serialize;

/// A definition, to the serialised `Payload` of building it.
pub type Built = Arc<dyn Fn(&Definition) -> eyre::Result<String> + Send + Sync>;

/// A definition, to the Places searches it lacks: none fetched before `since` counts.
pub type Asked = Arc<dyn Fn(&Definition, SystemTime) -> eyre::Result<Vec<Ask>> + Send + Sync>;

/// Whether a definition holds — its trade is served, its country has a base, the trade takes it —
/// without building it.
pub type Holds = Arc<dyn Fn(&Definition) -> eyre::Result<()> + Send + Sync>;

type Slot = Arc<tokio::sync::Mutex<Option<Bytes>>>;

#[derive(Clone)]
pub struct Studies {
	verifier: Verifier,
	catalog: Arc<Catalog>,
	/// Every definition handed in, `<sha>.json`.
	dir: PathBuf,
	holds: Holds,
	asks: Asked,
	build: Built,
	// shortcut: unbounded, one payload per area × category any operator opened; an LRU when the process grows
	built: Arc<Mutex<HashMap<String, Slot>>>,
}

impl Studies {
	/// `keys` are the ones the conductor signs every request it forwards with.
	pub fn new(keys: sa_auth::Keys, catalog: Catalog, dir: PathBuf, holds: Holds, asks: Asked, build: Built) -> Self {
		Self {
			verifier: Verifier::new(keys, sa_auth::Service::Placement),
			catalog: Arc::new(catalog),
			dir,
			holds,
			asks,
			build,
			built: Arc::default(),
		}
	}

	fn slot(&self, sha: &str) -> Slot {
		self.built.lock().expect("no holder panics").entry(sha.to_owned()).or_default().clone()
	}
}

impl FromRef<Studies> for Verifier {
	fn from_ref(s: &Studies) -> Self {
		s.verifier.clone()
	}
}

#[derive(Clone, Copy, Debug, Serialize, strum::VariantArray)]
#[serde(rename_all = "snake_case")]
pub enum Code {
	/// The definition is not one the conductor could have handed in.
	BadRequest,
	/// The definition holds, but nothing served here studies it: no such trade, no base for its
	/// country, or the trade refuses it.
	Unstudied,
	Internal,
}

impl ErrorCode for Code {
	fn status(self) -> StatusCode {
		match self {
			Self::BadRequest => StatusCode::BAD_REQUEST,
			Self::Unstudied => StatusCode::UNPROCESSABLE_ENTITY,
			Self::Internal => StatusCode::INTERNAL_SERVER_ERROR,
		}
	}
}

impl Refuses<Invalid> for Code {
	fn refusal(e: Invalid) -> Refusal<Self> {
		Refusal::new(Self::BadRequest, e.to_string())
	}
}

impl Refuses<eyre::Report> for Code {
	fn refusal(e: eyre::Report) -> Refusal<Self> {
		eprintln!("{e:?}");
		Refusal::new(Self::Internal, format!("{e:#}"))
	}
}

type Answer<T> = Result<T, Refusal<Code>>;

/// A built `Payload`, as the bytes it is kept in.
pub struct Studied(Bytes);

impl IntoResponse for Studied {
	fn into_response(self) -> Response {
		([(header::CONTENT_TYPE, "application/json")], self.0).into_response()
	}
}

impl OperationOutput for Studied {
	type Inner = Payload;

	fn operation_response(ctx: &mut GenContext, op: &mut Operation) -> Option<openapi::Response> {
		Json::<Payload>::operation_response(ctx, op)
	}

	fn inferred_responses(ctx: &mut GenContext, op: &mut Operation) -> Vec<(Option<openapi::StatusCode>, openapi::Response)> {
		Json::<Payload>::inferred_responses(ctx, op)
	}
}

/// What a category may be studied by, and where: the trade files and the country bases served.
async fn trades(State(s): State<Studies>) -> Json<Catalog> {
	Json(Catalog::clone(&s.catalog))
}

/// A study's payload.
///
/// Refused unless the definition holds: its trade is served, its country has a base, the trade
/// takes it. Built on the first ask and kept, out of what is held: a study whose answers are not
/// all there opens on what is, and its notice says how many it lacks.
async fn study(State(s): State<Studies>, Body(def): Body<Definition>) -> Answer<Studied> {
	studied(&s, def, false).await.map(Studied)
}

/// The Places searches a pull of the study still lacks, the first down each branch of its sweep:
/// what lies behind one is known once it is answered, so a pull asks again until none is lacking.
///
/// Since the first pull after its last whole one began, every answer is lacking until asked again:
/// a pull that stopped part-way has paid for what it bought.
async fn asks(State(s): State<Studies>, Body(def): Body<Definition>) -> Answer<Json<Asks>> {
	def.check().map_err(Code::refusal)?;
	let (dir, holds, asked) = (s.dir.clone(), s.holds.clone(), s.asks.clone());
	tokio::task::spawn_blocking(move || {
		holds(&def).map_err(|e| Refusal::new(Code::Unstudied, format!("{e:#}")))?;
		let sha = def.id();
		keep(&dir.join(format!("{sha}.json")), &def).map_err(|e| Refusal::new(Code::Internal, e))?;
		let resume = dir.join(format!("{sha}.resume"));
		let since = began(&resume).map_err(|e| Refusal::new(Code::Internal, format!("{}: {e}", resume.display())))?;
		let asks = asked(&def, since).map_err(Code::refusal)?;
		let since = jiff::Timestamp::try_from(since).map_err(|e| Refusal::new(Code::Internal, format!("a pull begun at {since:?}: {e}")))?;
		Ok(Json(Asks { since, asks }))
	})
	.await
	.unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()))
}

/// Builds a study again, out of what its pull bought: a whole pull, so the next one asks everything
/// again.
async fn pull(State(s): State<Studies>, Body(def): Body<Definition>) -> Answer<Studied> {
	let built = studied(&s, def.clone(), true).await?;
	let resume = s.dir.join(format!("{}.resume", def.id()));
	match std::fs::remove_file(&resume) {
		Ok(()) => {}
		// a pull of the same study, alongside this one, ended whole first
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
		Err(e) => return Err(Refusal::new(Code::Internal, format!("{} outlived a whole pull: {e}", resume.display()))),
	}
	Ok(Studied(built))
}

/// Built on the first read and kept; `again`, built again and kept.
async fn studied(s: &Studies, def: Definition, again: bool) -> Answer<Bytes> {
	def.check().map_err(Code::refusal)?;
	let sha = def.id();
	let slot = s.slot(&sha);
	let mut slot = slot.lock().await;
	if !again && let Some(built) = &*slot {
		return Ok(built.clone());
	}
	let (dir, holds, build) = (s.dir.clone(), s.holds.clone(), s.build.clone());
	let built = tokio::task::spawn_blocking(move || {
		holds(&def).map_err(|e| Refusal::new(Code::Unstudied, format!("{e:#}")))?;
		keep(&dir.join(format!("{sha}.json")), &def).map_err(|e| Refusal::new(Code::Internal, e))?;
		build(&def).map_err(Code::refusal)
	})
	.await
	.unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()))?;
	let built = Bytes::from(built);
	*slot = Some(built.clone());
	Ok(built)
}

/// Renamed into place: a reader never sees half a definition.
fn keep(path: &std::path::Path, def: &Definition) -> Result<(), String> {
	let canonical = serde_json::to_vec(&def.clone().canonical()).expect("a definition serialises");
	let part = path.with_extension("part");
	std::fs::write(&part, &canonical)
		.and_then(|()| std::fs::rename(&part, path))
		.map_err(|e| format!("keeping {}: {e}", path.display()))
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

fn routes() -> Vec<Route<Studies, Gate>> {
	let (read, spend) = (Gate::USER.needs(Study::Read), Gate::USER.needs(Study::Spend));
	vec![
		Route::new(Method::GET, "/trades.json", read, trades),
		Route::new(Method::POST, "/studies", read, study),
		Route::new(Method::POST, "/studies/asks", spend, asks),
		Route::new(Method::POST, "/studies/pull", spend, pull),
	]
}

fn router(studies: Studies) -> eyre::Result<Router> {
	std::fs::create_dir_all(&studies.dir).wrap_err_with(|| format!("creating {}", studies.dir.display()))?;
	Ok(routes()
		.into_iter()
		.fold(Router::new(), |r, route| r.route(route.path, route.handler()))
		.route("/health", axum::routing::get(|| async { "ok" }))
		.with_state(studies))
}

pub fn serve(addr: SocketAddr, studies: Studies) -> eyre::Result<()> {
	let app = router(studies)?.layer(tower_http::compression::CompressionLayer::new());
	let rt = tokio::runtime::Runtime::new().wrap_err("starting the server runtime")?;
	rt.block_on(async move {
		let listener = tokio::net::TcpListener::bind(&addr).await.wrap_err_with(|| format!("binding {addr}"))?;
		eprintln!("placement is at http://{}", listener.local_addr()?);
		axum::serve(listener, app.into_make_service()).await.wrap_err("serving")
	})
}
