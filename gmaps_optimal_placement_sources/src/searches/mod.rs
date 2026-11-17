//! How many times a month a query is asked. Unlike grids and POI sources, the set of volume
//! providers is open — see `docs/ARCHITECTURE.md` — so this is a trait with one registry `match`.
mod dataforseo;
mod google_ads;

use eyre::{Result, bail};
use sa_core::gbp::Country;
use serde::{Deserialize, Serialize};

use crate::work::Work;

/// How many seeds one expansion takes: Keyword Planner's `keywordSeed` and DataForSEO's
/// `keywords_for_keywords` both cap it at 20.
pub const SEEDS: usize = 20;

pub trait SearchVolume {
	/// Keywords semantically around `seeds` (at most [`SEEDS`]), each with its own trailing-12-month
	/// series.
	/// `place` is a Google canonical location name; `lang` an ISO-639-1 code.
	fn ideas(&self, seeds: &[String], place: &str, lang: &str, work: &Work) -> Result<Vec<Keyword>>;
	/// `keywords` exactly, each with its own trailing-12-month series where it has one. What a study
	/// weighs its terms by.
	fn volumes(&self, keywords: &[String], place: &str, lang: &str, work: &Work) -> Result<Vec<Keyword>>;
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Month {
	pub year: u16,
	pub month: u8,
	pub searches: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Keyword {
	pub text: String,
	/// Absent when the provider has no data for this keyword at this place — distinct from zero,
	/// and never summed as zero.
	pub monthly: Option<Vec<Month>>,
}

/// A Google Ads account's Keyword Planner credentials.
#[derive(Clone)]
pub struct Ads {
	pub developer_token: String,
	pub client_id: String,
	pub client_secret: String,
	pub refresh_token: String,
	pub customer_id: String,
}

/// The one place that knows the set. `ads` is every configured account's, of which Keyword Planner
/// needs exactly one.
pub fn provider(name: &str, ads: &[Ads]) -> Result<Box<dyn SearchVolume>> {
	match name {
		"google_ads" => match ads {
			[one] => Ok(Box::new(google_ads::GoogleAds::new(one.clone()))),
			_ => bail!("Keyword Planner buys through the one account in the config's `google` that provides `ads`, and {} do", ads.len()),
		},
		"dataforseo" => Ok(Box::new(dataforseo::DataForSeo::from_env()?)),
		_ => bail!("unknown searches.provider {name:?} — known: google_ads, dataforseo"),
	}
}

/// The Google canonical name of a country, which both providers resolve volume against.
pub fn country_place(country: Country) -> &'static str {
	match country {
		Country::FR => "France",
		Country::US => "United States",
		Country::CA => "Canada",
		Country::GB => "United Kingdom",
	}
}

fn env(var: &str) -> Result<String> {
	std::env::var(var).map_err(|_| eyre::eyre!("{var} is not set"))
}
