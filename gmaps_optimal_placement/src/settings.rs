//! What the tool does, rather than what a study is about. A study document is the two axes of the
//! product and nothing else; anything that would be the same for every study — how old an answer may
//! get before the map says so, how a frame is tiled — lives here.
use gmaps_optimal_placement_sources::{poi::Tiling, work::Age as Ages};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use smart_default::SmartDefault;
use v_utils::{
	Timeframe,
	macros::{ConfigJsonSchema, MyConfigPrimitives, Settings, SettingsNested},
};

#[derive(Clone, ConfigJsonSchema, Debug, MyConfigPrimitives, Settings)]
pub struct AppConfig {
	#[settings(flatten)]
	pub age: Age,
	#[settings(flatten)]
	pub sweep: Sweep,
}

/// How the competitor sweep tiles a study's frame. Every answer is cached under the exact tile it
/// was asked for. A query already on disk keeps the root it was swept from until `--refresh`; a
/// deeper `depth` asks the finer tiles under whatever came back at the cap.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize, SettingsNested, SmartDefault)]
#[serde(default)]
pub struct Sweep {
	/// Tiles a side the frame starts as, before any query is asked.
	#[default(Tiling::ROOT)]
	pub root: u32,
	/// How many times a tile that came back at Google's 60-result cap may be quartered.
	#[default(Tiling::DEPTH)]
	pub depth: u32,
}

impl From<&Sweep> for Tiling {
	fn from(s: &Sweep) -> Self {
		Self { root: s.root, depth: s.depth }
	}
}

/// How old an answer may get before the map says so. Nothing is evicted and nothing is refetched:
/// a refetch spends a day of Google's quota to learn what is mostly the same thing, so age is
/// reported and `--refresh` is the only thing that re-asks.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize, SettingsNested, SmartDefault)]
#[serde(default)]
pub struct Age {
	/// The inventory sweep: shop names, ratings, addresses. Shops open and close.
	#[default(Ages::INVENTORY.into())]
	pub inventory: Timeframe,
	/// Probe orderings — who ranks where from each stratum. What `core::rank::COEF` is fitted on.
	#[default(Ages::ORDERING.into())]
	pub ordering: Timeframe,
	/// The commune list from geo.api.gouv.fr. Drifts only on mergers.
	#[default(Ages::COMMUNES.into())]
	pub communes: Timeframe,
}

impl From<&Age> for Ages {
	fn from(a: &Age) -> Self {
		Self {
			inventory: a.inventory.duration(),
			ordering: a.ordering.duration(),
			communes: a.communes.duration(),
		}
	}
}
