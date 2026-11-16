//! The study document. Not settings: there is no sane environment-variable spelling of `[[layer]]`,
//! so it is plain `Deserialize` from a path, with a JSON schema for the editor.
//!
//! A study is not one file. It is a trade applied to an area — `import trades/cleaning.nix
//! (import countries/FR.nix // { area = …; })` — because the two vary independently, and then the
//! category's own words added on top: its Google categories are what the sweep asks, its keywords
//! what the competitors are scored on. What the area decides is the frame, the statistics office
//! behind it, and the addresses being considered; everything else is the trade. Nothing here may
//! need both: see the invariant in `docs/ARCHITECTURE.md`.
//!
//! An area on its own is also a study, the identity of the pairing — and the three fields below
//! that a trade fills are `None` for it. What is left is what the statistical archive publishes,
//! which is the whole of the free half: a city can be looked at before a single Places call is spent
//! on it.
pub use gmaps_optimal_placement_core::payload::{Candidate, Scale};
use gmaps_optimal_placement_sources::{GridSource, PoiConfig};
use sa_core::{
	gbp::Country,
	placement::{Area, Category},
};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Study {
	/// The definition's: `<study>·<CC>·<slug>`. Not written down anywhere, and what the pin file and
	/// the searches chart are keyed by.
	#[serde(skip)]
	pub name: String,
	/// What the study was asked over; `None` with `poi`, `model` and `rank`.
	#[serde(skip)]
	pub category: Option<Category>,
	/// The definition's, handed through the trade untouched.
	pub area: Area,
	/// The country the frame is in, whose language its searches are asked in.
	pub country: Country,
	pub grid: GridSpec,
	/// The trade's three, absent together on a location opened without one.
	#[serde(default)]
	pub poi: Option<PoiConfig>,
	/// Columns the source does not publish, evaluated in order over the ones it does. Later entries
	/// may read earlier ones, so this is a list: an attribute set would be evaluated in whatever
	/// order its names happen to sort in.
	#[serde(default, rename = "column")]
	pub columns: Vec<Column>,
	#[serde(default)]
	pub model: Option<Model>,
	#[serde(default)]
	pub rank: Option<RankSpec>,
	#[serde(rename = "layer")]
	pub layers: Vec<Layer>,
	/// Optional: the map answers where the people are, this answers how many of them ask for the
	/// thing. A study without it simply has no `searches` command.
	#[serde(default)]
	pub searches: Option<Searches>,
	/// Addresses the study is actually about. The usual question is "is *this* one good", not only
	/// "where is best". A premises is a location's, not a trade's: the same unit is worth asking
	/// about for whichever trade would move into it.
	#[serde(default, rename = "candidate")]
	pub candidates: Vec<Candidate>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GridSpec {
	pub source: GridSource,
	pub vintage: u16,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Model {
	/// Per cell, over the grid's columns.
	pub demand: String,
	/// Catchment decay, metres. The map's opening slider position.
	pub lambda_m: f64,
}

/// How the probe samples the frame and what it asks there. The reviews→prominence curve is not here:
/// it is a property of how Google ranks, shared by every trade, and lives in
/// `gmaps_optimal_placement_core::rank`.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RankSpec {
	/// Power of two — a k-d split to depth log2(n), one equal share of demand per stratum. The
	/// `locationBias` circle the probe asks from is a stratum's worth of ground, so this sets it too.
	/// What it asks there is the category's keywords: what a customer types, not the sweep's
	/// queries, which are tuned to *find* competitors and so lean on words already in shop names.
	pub nodes: usize,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Column {
	pub name: String,
	pub expr: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Layer {
	pub name: String,
	pub expr: String,
	#[serde(default)]
	pub scale: Scale,
	#[serde(default)]
	pub note: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Searches {
	/// `google_ads` or `dataforseo`.
	pub provider: String,
	/// A Google canonical location name, e.g. `Clermont-Ferrand,Auvergne-Rhone-Alpes,France`, which
	/// the `searches` chart resolves volume against. A location file names one; an area handed in at
	/// runtime has none. DataForSEO's `location_name` format is the same string.
	#[serde(default)]
	pub place: Option<String>,
	/// The trade's, so a location states the provider and the place and nothing else. `searches`
	/// refuses an empty set rather than charting one. Each filters one expansion of the category's
	/// keywords: what a customer types is the category's to say, not a trade's.
	#[serde(default, rename = "group")]
	pub groups: Vec<Group>,
}

/// One line on the chart: an intent, asked for in however many words people ask for it in.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Group {
	pub name: String,
	/// Case-insensitive regex over the expansion. Omitted keeps all of it.
	#[serde(default, rename = "match")]
	pub pattern: Option<String>,
	/// Checked before `match` — expansion drags in intent that is not demand.
	#[serde(default)]
	pub drop: Option<String>,
}
