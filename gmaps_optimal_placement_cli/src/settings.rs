//! What the tool does, rather than what a study is about. A study document is the two axes of the
//! product and nothing else; anything that would be the same for every study — how old an answer may
//! get before the map says so — lives here.
use std::path::PathBuf;

use gmaps_optimal_placement_sources::work::Age as Ages;
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
	/// A copy of prod's `panel.db`, what the conductor bought read off it: site_conductor's
	/// `nix run .#prod-restore -- panel-data/panel.db=<this path>` takes one.
	pub queries_cache: PathBuf,
	/// A Google Maps JavaScript API key, for the GUI's map.
	pub maps_key: Option<String>,
}

/// How old an answer may get before the map says so. Nothing is evicted and nothing is refetched:
/// a refetch spends a day of Google's quota to learn what is mostly the same thing, so age is
/// reported and a pull is the only thing that re-asks.
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
