//! What the tool does, rather than what a study is about. A study document is the two axes of the
//! product and nothing else; anything that would be the same for every study — how old an answer may
//! get before the map says so, how a frame is tiled — lives here.
use gmaps_optimal_placement_sources::{poi::Tiling, searches, work::Age as Ages};
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
	/// accounting's google.nix `accounts`, imported whole: every Google key is a slot of one of them.
	#[serde(default)]
	#[settings(skip)]
	pub google: Vec<Account>,
	/// Where what is bought is cached for everyone: the conductor's `<origin>/api/queries_cache`,
	/// so no machine buys the same query twice; or `local`, this machine's work dir alone.
	pub queries_cache: String,
}

#[derive(Clone, ConfigJsonSchema, Debug, MyConfigPrimitives)]
pub struct Account {
	/// What a search bought on its keys is counted against.
	pub name: String,
	pub provides: Provides,
}

#[derive(Clone, ConfigJsonSchema, Debug, MyConfigPrimitives)]
pub struct Provides {
	/// Places searches.
	pub places: Option<Key>,
	/// The map's Maps JS.
	pub browser: Option<Key>,
	/// Keyword Planner.
	pub ads: Option<Ads>,
}

#[derive(Clone, ConfigJsonSchema, Debug, MyConfigPrimitives)]
pub struct Key {
	pub key: String,
}

#[derive(Clone, ConfigJsonSchema, Debug, MyConfigPrimitives)]
pub struct Ads {
	pub developer_token: String,
	pub client_id: String,
	pub client_secret: String,
	pub refresh_token: String,
	pub customer_id: String,
}

impl From<&Ads> for searches::Ads {
	fn from(a: &Ads) -> Self {
		Self {
			developer_token: a.developer_token.clone(),
			client_id: a.client_id.clone(),
			client_secret: a.client_secret.clone(),
			refresh_token: a.refresh_token.clone(),
			customer_id: a.customer_id.clone(),
		}
	}
}

impl From<&AppConfig> for Vec<(String, String)> {
	fn from(cfg: &AppConfig) -> Self {
		cfg.google.iter().filter_map(|a| a.provides.places.as_ref().map(|p| (a.name.clone(), p.key.clone()))).collect()
	}
}

/// How the competitor sweep tiles the ground. Every answer is cached under the exact quadtree tile it
/// was asked for, so a changed `zoom` asks a new set of tiles; a deeper `depth` asks the finer tiles
/// under whatever came back at the cap.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize, SettingsNested, SmartDefault)]
#[serde(default)]
pub struct Sweep {
	/// The Web-Mercator level a query is first asked at.
	#[default(Tiling::ZOOM)]
	pub zoom: u32,
	/// How many times a tile that came back at Google's 60-result cap may be quartered.
	#[default(Tiling::DEPTH)]
	pub depth: u32,
}

impl From<&Sweep> for Tiling {
	fn from(s: &Sweep) -> Self {
		Self { zoom: s.zoom, depth: s.depth }
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
