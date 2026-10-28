//! What the browser is handed: the study, already evaluated. Everything a slider cannot move.
//!
//! The shape lives here rather than beside the CLI because the map reads it back, and the map is
//! wasm — it may not link a single line of I/O.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Payload {
	pub name: String,
	pub center: [f64; 2],
	pub zoom: u8,
	/// 8 numbers per cell: (lon, lat) SW, SE, NE, NW.
	pub ring: Vec<f64>,
	pub place: Vec<String>,
	pub imputed: Vec<u8>,
	pub layers: Vec<LayerOut>,
	pub candidates: Vec<Candidate>,
	/// Absent when a location was opened without one. Everything above is the location's, comes out
	/// of the statistical archive, and costs nothing to ask for.
	pub trade: Option<Trade>,
	/// Why the map is less than what was asked for — a spent day of Places quota above all. Set, the
	/// trade half is missing for a reason the page can state, rather than for the reason `trade: None`
	/// usually means.
	#[serde(default)]
	pub notice: Option<String>,
}

/// The half of a payload a trade decides, and the whole of the half that is billed. One `Option`
/// rather than eight, because a demand surface with no competitors under it is not a state this
/// tool has anything to say about.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Trade {
	/// Catchment decay, metres. The map's opening slider position.
	pub lambda_m: f64,
	/// The demand expression itself — the map has no other honest caption for it.
	pub demand_note: String,
	pub demand: Vec<f64>,
	pub tiers: Vec<TierOut>,
	/// The study's queries and what each is worth. `w` is already scored against them; the what-if
	/// needs them because the business it scores does not exist yet.
	pub terms: Vec<TermOut>,
	/// The inventory sweep's queries, most competitors on this map first.
	pub queries: Vec<QueryOut>,
	/// Where the probe stood, in the order `PoiOut::seen` is indexed by. Empty until a study has been
	/// probed, which is what decides whether the map offers an observed coverage layer at all.
	pub nodes: Vec<[f64; 2]>,
	pub pois: Vec<PoiOut>,
	/// Places searches the inventory sweep was answered with — what re-asking it buys again.
	pub searches: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TermOut {
	pub text: String,
	pub weight: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QueryOut {
	pub text: String,
	/// Distinct competitors on the map it returned, over every tile. The queries overlap by design.
	pub hits: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LayerOut {
	pub name: String,
	pub note: String,
	pub scale: Scale,
	pub values: Vec<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TierOut {
	pub name: String,
	pub weight: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PoiOut {
	#[serde(flatten)]
	pub poi: Poi,
	/// How much this one counts, from `rank`, normalised so the median of its tier is 1.
	pub w: f64,
	/// What the probe saw of this business from each of `Payload::nodes`, and nothing modelled: a
	/// competitor's real reach is lumpy and one radial decay cannot say so. Empty alongside `nodes`.
	pub seen: Vec<f64>,
}

/// Whoever is already selling the service. The inventory that produces one is `gmaps_optimal_placement_sources`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Poi {
	pub id: String,
	pub name: String,
	pub addr: String,
	pub lat: f64,
	pub lng: f64,
	pub rating: Option<f64>,
	pub n_rev: f64,
	pub kind: String,
	pub kind_label: String,
	pub web: String,
	pub tel: String,
	pub tier: String,
	/// Unix seconds the freshest answer naming it was written. Provenance, like `imputed`: nothing is
	/// evicted, so without it a map drawn from two-year-old competitors looks like one drawn this morning.
	pub fetched_s: u64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Scale {
	/// Rank, not magnitude — these quantities are heavy-tailed and a linear ramp shows one hot pixel.
	#[default]
	Percentile,
	Linear,
}

/// An address the study is actually about. The usual question is "is *this* one good", not only
/// "where is best".
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
	pub name: String,
	/// [lat, lon]
	pub at: [f64; 2],
}
