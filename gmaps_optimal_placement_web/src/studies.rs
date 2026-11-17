//! What the conductor asks: one table of operations behind its assertion, the router and the
//! document both derived from it (`ev_lib::api`). A definition comes in with every ask and its
//! payload goes out; it is kept on disk under its id, so `fit` can read what it collected, and its
//! payload in memory once built.
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
	http::{HeaderName, Method, StatusCode, header},
	response::{IntoResponse, Response},
};
use ev_lib::api::{
	Body, ErrorCode, GenContext, OperationOutput, Params, Refusal, Refuses, Route,
	openapi::{self, Operation},
};
use eyre::WrapErr;
use gmaps_optimal_placement_core::{Payload, Unaffordable};
use sa_auth::{Gate, Study, Verifier};
use sa_core::{
	Invalid,
	placement::{BILLED, Billed as Bought, Catalog, Definition},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A definition, to the serialised `Payload` of building it, and what that bought — counted whether
/// it built or not, because it is paid either way. A pull's budget buys what the work dir lacks;
/// without one nothing is bought.
pub type Built = Arc<dyn Fn(&Definition, Option<Pull>) -> (Bought, eyre::Result<String>) + Send + Sync>;

/// Whether a definition holds — its trade is served, its country has a base, the trade takes it —
/// without building it.
pub type Holds = Arc<dyn Fn(&Definition) -> eyre::Result<()> + Send + Sync>;

/// What a pull may buy, and the answers it re-asks: those written before `since`.
pub struct Pull {
	/// Places searches.
	pub budget: usize,
	pub since: SystemTime,
}

type Slot = Arc<tokio::sync::Mutex<Option<Bytes>>>;

#[derive(Clone)]
pub struct Studies {
	verifier: Verifier,
	catalog: Arc<Catalog>,
	/// Every definition handed in, `<sha>.json`.
	dir: PathBuf,
	holds: Holds,
	build: Built,
	// shortcut: unbounded, one payload per area × category any operator opened; an LRU when the process grows
	built: Arc<Mutex<HashMap<String, Slot>>>,
}

impl Studies {
	/// `keys` are the ones the conductor signs every request it forwards with.
	pub fn new(keys: sa_auth::Keys, catalog: Catalog, dir: PathBuf, holds: Holds, build: Built) -> Self {
		Self {
			verifier: Verifier::new(keys, sa_auth::Service::Placement),
			catalog: Arc::new(catalog),
			dir,
			holds,
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
	/// The pull needs more Places searches than its budget.
	Unaffordable,
	Internal,
}

impl ErrorCode for Code {
	fn status(self) -> StatusCode {
		match self {
			Self::BadRequest => StatusCode::BAD_REQUEST,
			Self::Unstudied => StatusCode::UNPROCESSABLE_ENTITY,
			Self::Unaffordable => StatusCode::PAYMENT_REQUIRED,
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
		if let Some(u) = e.downcast_ref::<Unaffordable>() {
			return Refusal::new(Self::Unaffordable, u.to_string());
		}
		eprintln!("{e:?}");
		Refusal::new(Self::Internal, format!("{e:#}"))
	}
}

type Answer<T> = Result<T, Refusal<Code>>;

#[derive(Deserialize, JsonSchema)]
pub struct Budget {
	/// How many Places searches the pull may buy.
	budget: usize,
}

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

/// An answer and, on it whatever it is, `x-billed`: what it bought, which is what the conductor
/// charges the puller.
pub struct Billed<T> {
	bought: Bought,
	answer: T,
}

impl<T: IntoResponse> IntoResponse for Billed<T> {
	fn into_response(self) -> Response {
		let bought = serde_json::to_string(&self.bought).expect("items and counts serialise");
		([(HeaderName::from_static(BILLED), bought)], self.answer).into_response()
	}
}

impl<T: OperationOutput> OperationOutput for Billed<T> {
	type Inner = T::Inner;

	fn operation_response(ctx: &mut GenContext, op: &mut Operation) -> Option<openapi::Response> {
		T::operation_response(ctx, op)
	}

	fn inferred_responses(ctx: &mut GenContext, op: &mut Operation) -> Vec<(Option<openapi::StatusCode>, openapi::Response)> {
		T::inferred_responses(ctx, op)
	}
}

/// What a category may be studied by, and where: the trade files and the country bases served.
async fn trades(State(s): State<Studies>) -> Json<Catalog> {
	Json(Catalog::clone(&s.catalog))
}

/// A study's payload.
///
/// Refused unless the definition holds: its trade is served, its country has a base, the trade
/// takes it. Built on the first ask and kept; never spends. A study whose answers are not all in
/// the work dir opens on what it holds, and its notice says how many are missing.
async fn study(State(s): State<Studies>, Body(def): Body<Definition>) -> Answer<Studied> {
	let (bought, built) = studied(&s, def, None).await;
	assert!(bought.is_empty(), "a study is built with no budget: {bought:?}");
	built.map(Studied)
}

/// Builds a study again, buying at most `budget` Places searches.
///
/// Refused `unaffordable` before the first search when the sweep counts past the budget, and
/// stopped with it when it outgrows its count. Every answer carries `x-billed`, what it bought.
/// After a pull that did not end whole, re-asks only what was written before that one began.
async fn pull(
	State(s): State<Studies>,
	Params(Budget { budget }): Params<Budget>,
	Body(def): Body<Definition>,
) -> Billed<Answer<Studied>> {
	let (bought, built) = studied(&s, def, Some(budget)).await;
	Billed {
		bought,
		answer: built.map(Studied),
	}
}

/// Built on the first read and kept. A pull re-asks Google everything it is built from, and that
/// answer goes to the puller alone: one stopped part-way by the day's quota is a map with no
/// competitors, while the work dir now holds every answer it paid for, so the next read builds from
/// that, and the next pull buys only what this one did not. Beside it, the Places searches bought,
/// which is all a pull is charged on.
async fn studied(s: &Studies, def: Definition, budget: Option<usize>) -> (Bought, Answer<Bytes>) {
	if let Err(e) = def.check() {
		return (Bought::new(), Err(Code::refusal(e)));
	}
	let sha = def.id();
	let slot = s.slot(&sha);
	let mut slot = slot.lock().await;
	if budget.is_some() {
		*slot = None;
	}
	if let Some(built) = &*slot {
		return (Bought::new(), Ok(built.clone()));
	}
	let (dir, holds, build) = (s.dir.clone(), s.holds.clone(), s.build.clone());
	// a panicked build took what it bought with it, and no answer is better than an x-billed that guesses
	let (bought, built) = tokio::task::spawn_blocking(move || {
		if let Err(e) = holds(&def) {
			return (Bought::new(), Err(Refusal::new(Code::Unstudied, format!("{e:#}"))));
		}
		if let Err(e) = keep(&dir.join(format!("{sha}.json")), &def) {
			return (Bought::new(), Err(Refusal::new(Code::Internal, e)));
		}
		let resume = dir.join(format!("{sha}.resume"));
		let pull = match budget.map(|budget| began(&resume).map(|since| Pull { budget, since })).transpose() {
			Ok(pull) => pull,
			Err(e) => return (Bought::new(), Err(Refusal::new(Code::Internal, format!("{}: {e}", resume.display())))),
		};
		let (bought, built) = build(&def, pull);
		let built = built.map_err(Code::refusal).and_then(|b| match budget {
			Some(_) => std::fs::remove_file(&resume)
				.map(|()| b)
				.map_err(|e| Refusal::new(Code::Internal, format!("{} outlived a whole pull: {e}", resume.display()))),
			None => Ok(b),
		});
		(bought, built)
	})
	.await
	.unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()));
	let built = built.map(Bytes::from);
	if let (None, Ok(b)) = (budget, &built) {
		*slot = Some(b.clone());
	}
	(bought, built)
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
		Route::new(Method::POST, "/studies/pull", spend, pull),
	]
}

fn router(studies: Studies) -> eyre::Result<Router> {
	std::fs::create_dir_all(&studies.dir).wrap_err_with(|| format!("creating {}", studies.dir.display()))?;
	Ok(routes()
		.into_iter()
		.fold(Router::new(), |r, route| r.route(route.path, route.handler()))
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
