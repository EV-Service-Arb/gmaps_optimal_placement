//! The study document. Not settings: there is no sane environment-variable spelling of `[[layer]]`,
//! so it is plain `Deserialize` from a path, with a JSON schema for the editor.
//!
//! A study is a category applied to an area. The area's base — a country file, or a location file
//! over one, with the area handed in on top — states the frame, the statistics office behind it, the
//! columns and layers every trade there shares, and the addresses being considered; the category
//! states everything else. A base may not name a trade: see the invariant in `docs/ARCHITECTURE.md`.
//!
//! An area on its own is also a study, the identity of the pairing. What is left is what the
//! statistical archive publishes, which is the whole of the free half: a city can be looked at before
//! a single Places call is spent on it.
pub use gmaps_optimal_placement_core::payload::Candidate;
use gmaps_optimal_placement_sources::GridSource;
use sa_core::{
	gbp::Country,
	placement::{Area, Category, Column, Layer},
};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Study {
	/// The definition's: `<trade>·<CC>·<slug>`. Not written down anywhere, and what the pin file is
	/// keyed by.
	#[serde(skip)]
	pub name: String,
	/// What the study was asked over; `None` for an area on its own.
	#[serde(skip)]
	pub category: Option<Category>,
	/// The definition's, laid over the base.
	pub area: Area,
	/// The country the base is for.
	pub country: Country,
	pub grid: GridSpec,
	/// Columns the source does not publish, evaluated in order over the ones it does; the category's
	/// demand columns follow the base's. Later entries may read earlier ones, so this is a list: an
	/// attribute set would be evaluated in whatever order its names happen to sort in.
	#[serde(rename = "column")]
	pub columns: Vec<Column>,
	/// The base's heat layers, then the category's.
	#[serde(rename = "layer")]
	pub layers: Vec<Layer>,
	/// Addresses the study is actually about. The usual question is "is *this* one good", not only
	/// "where is best". A premises is a location's, not a trade's: the same unit is worth asking
	/// about for whichever trade would move into it.
	#[serde(rename = "candidate")]
	pub candidates: Vec<Candidate>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GridSpec {
	pub source: GridSource,
	pub vintage: u16,
}
