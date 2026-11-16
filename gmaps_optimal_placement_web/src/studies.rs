//! What the conductor's map reads: one table of operations behind the conductor's assertion, the
//! router and the document both derived from it (`ev_lib::api`). A definition is kept on disk once
//! handed in, so a restart serves it again and `fit` can read what it collected; its payload is
//! kept in memory once built.
use std::{
	collections::HashMap,
	path::PathBuf,
	sync::{Arc, Mutex},
	time::{SystemTime, UNIX_EPOCH},
};

use axum::{
	Json, Router,
	body::Bytes,
	extract::{FromRef, State},
	http::{HeaderName, Method, StatusCode, header},
	response::{IntoResponse, NoContent, Response},
};
use ev_lib::api::{
	Body, ErrorCode, GenContext, Ids, OperationOutput, Params, Refusal, Refuses, Route,
	openapi::{self, OpenApi, Operation},
};
use eyre::WrapErr;
use gmaps_optimal_placement_core::{Payload, Unaffordable};
use sa_auth::{Gate, Study, User, Verifier};
use sa_core::{
	Invalid,
	gbp::Country,
	placement::Definition,
	prices::{AdsRequest, DataForSeoTask, PlacementRequest, Spends},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A definition, to the serialised `Payload` of building it, and the Places searches that bought —
/// counted whether it built or not, because they are paid either way. A pull's budget buys what the
/// work dir lacks; without one nothing is bought.
pub type Built = Arc<dyn Fn(&Definition, Option<Pull>) -> (usize, eyre::Result<String>) + Send + Sync>;

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

/// What a category may be studied by, and where.
#[derive(Clone, Debug, JsonSchema, Serialize)]
pub struct Served {
	/// The trade files a category's `study` names one of.
	pub trades: Vec<String>,
	/// The countries a base is served for.
	pub countries: Vec<Country>,
}

#[derive(Clone)]
pub struct Studies {
	verifier: Verifier,
	served: Arc<Served>,
	/// Every definition handed in, `<sha>.json`.
	dir: PathBuf,
	holds: Holds,
	build: Built,
	// shortcut: unbounded, one payload per area × category any operator opened; an LRU when the process grows
	built: Arc<Mutex<HashMap<String, Slot>>>,
}

impl Studies {
	/// `keys` are the ones the conductor signs every request it forwards with.
	pub fn new(keys: sa_auth::Keys, served: Served, dir: PathBuf, holds: Holds, build: Built) -> Self {
		Self {
			verifier: Verifier::new(keys, sa_auth::Service::Placement),
			served: Arc::new(served),
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
	/// No study is defined under that id: PUT it first.
	NotFound,
	/// The definition is not one the conductor could have handed in, or not under that id.
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
			Self::NotFound => StatusCode::NOT_FOUND,
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
pub struct At {
	/// The definition's id: the SHA-256 of its canonical form, hex.
	sha: String,
}

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

/// An answer and, on it whatever it is, `x-billed`: the Places searches it bought, which is what
/// the conductor charges the puller.
pub struct Billed<T> {
	searches: usize,
	answer: T,
}

impl<T: IntoResponse> IntoResponse for Billed<T> {
	fn into_response(self) -> Response {
		([(HeaderName::from_static("x-billed"), self.searches.to_string())], self.answer).into_response()
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
async fn trades(State(s): State<Studies>) -> Json<Served> {
	Json(Served::clone(&s.served))
}

/// Defines a study.
///
/// Refused unless `sha` is the definition's id and the definition holds: its trade is served, its
/// country has a base, the trade takes it. Buys nothing.
async fn define(State(s): State<Studies>, Ids(At { sha }): Ids<At>, Body(def): Body<Definition>) -> Answer<NoContent> {
	def.check()?;
	let id = def.id();
	if id != sha {
		return Err(Refusal::new(Code::BadRequest, format!("the definition's id is {id}, not {sha}")));
	}
	let canonical = serde_json::to_vec(&def.clone().canonical()).expect("a definition serialises");
	let path = s.dir.join(format!("{sha}.json"));
	let holds = s.holds.clone();
	tokio::task::spawn_blocking(move || {
		holds(&def).map_err(|e| Refusal::new(Code::Unstudied, format!("{e:#}")))?;
		// renamed into place: a reader never sees half a definition
		let part = path.with_extension("part");
		std::fs::write(&part, &canonical)
			.and_then(|()| std::fs::rename(&part, &path))
			.map_err(|e| Refusal::new(Code::Internal, format!("keeping {}: {e}", path.display())))
	})
	.await
	.unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()))?;
	Ok(NoContent)
}

/// A study's payload.
///
/// Built on the first read and kept; never spends. A study whose answers are not all in the work
/// dir opens on what it holds, and its notice says how many are missing.
async fn study(State(s): State<Studies>, Ids(At { sha }): Ids<At>) -> Answer<Studied> {
	let (billed, built) = studied(&s, &sha, None).await;
	assert_eq!(billed, 0, "a read is built with no budget");
	built.map(Studied)
}

/// Builds a study again, buying at most `budget` Places searches.
///
/// Refused `unaffordable` before the first search when the sweep counts past the budget, and
/// stopped with it when it outgrows its count. Every answer carries `x-billed`, the searches it
/// bought. After a pull that did not end whole, re-asks only what was written before that one began.
async fn pull(
	State(s): State<Studies>,
	Spends(..): Spends<(PlacementRequest, (AdsRequest, DataForSeoTask)), User>,
	Ids(At { sha }): Ids<At>,
	Params(Budget { budget }): Params<Budget>,
) -> Billed<Answer<Studied>> {
	let (searches, built) = studied(&s, &sha, Some(budget)).await;
	Billed {
		searches,
		answer: built.map(Studied),
	}
}

/// Built on the first read and kept. A pull re-asks Google everything it is built from, and that
/// answer goes to the puller alone: one stopped part-way by the day's quota is a map with no
/// competitors, while the work dir now holds every answer it paid for, so the next read builds from
/// that, and the next pull buys only what this one did not. Beside it, the Places searches bought,
/// which is all a pull is charged on.
async fn studied(s: &Studies, sha: &str, budget: Option<usize>) -> (usize, Answer<Bytes>) {
	let path = s.dir.join(format!("{sha}.json"));
	let slot = s.slot(sha);
	let mut slot = slot.lock().await;
	if budget.is_some() {
		*slot = None;
	}
	if let Some(built) = &*slot {
		return (0, Ok(built.clone()));
	}
	let def: Definition = match std::fs::read(&path) {
		Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| panic!("{} is not a definition: {e}", path.display())),
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (0, Err(Refusal::new(Code::NotFound, format!("no study {sha} was defined here: PUT it first")))),
		Err(e) => return (0, Err(Refusal::new(Code::Internal, format!("reading {}: {e}", path.display())))),
	};
	let resume = s.dir.join(format!("{sha}.resume"));
	let pull = match budget.map(|budget| began(&resume).map(|since| Pull { budget, since })).transpose() {
		Ok(pull) => pull,
		Err(e) => return (0, Err(Refusal::new(Code::Internal, format!("{}: {e}", resume.display())))),
	};
	let build = s.build.clone();
	// a panicked build took what it bought with it, and no answer is better than an x-billed that guesses
	let (billed, built) = tokio::task::spawn_blocking(move || build(&def, pull))
		.await
		.unwrap_or_else(|e| std::panic::resume_unwind(e.into_panic()));
	let built = match built {
		Ok(b) => Bytes::from(b),
		Err(e) => return (billed, Err(e.into())),
	};
	match budget {
		None => *slot = Some(built.clone()),
		Some(_) =>
			if let Err(e) = std::fs::remove_file(&resume) {
				return (billed, Err(Refusal::new(Code::Internal, format!("{} outlived a whole pull: {e}", resume.display()))));
			},
	}
	(billed, Ok(built))
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
		Route::new(Method::PUT, "/studies/{sha}", read, define),
		Route::new(Method::GET, "/studies/{sha}", read, study),
		Route::new(Method::POST, "/studies/{sha}/pull", spend, pull),
	]
}

/// Placement's document, as the conductor's catalog pulls it.
pub fn document() -> OpenApi {
	ev_lib::api::document("placement", env!("CARGO_PKG_VERSION"), "/", &routes(), ev_lib::api_docs!(), None)
}

/// The table, and `/openapi.json` beside it for the conductor and for a person who may read studies.
pub fn router(studies: Studies) -> eyre::Result<Router> {
	std::fs::create_dir_all(&studies.dir).wrap_err_with(|| format!("creating {}", studies.dir.display()))?;
	Ok(routes()
		.into_iter()
		.fold(Router::new(), |r, route| r.route(route.path, route.handler()))
		.route("/openapi.json", ev_lib::api::serve(&document(), Gate::USER.or_conductor()))
		.with_state(studies))
}
