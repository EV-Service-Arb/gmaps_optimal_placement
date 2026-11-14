//! How many times a month a query is asked. Unlike grids and POI sources, the set of volume
//! providers is open — see `docs/ARCHITECTURE.md` — so this is a trait with one registry `match`.
mod dataforseo;
mod google_ads;

use eyre::{Result, bail};
use serde::{Deserialize, Serialize};

use crate::work::Work;

pub trait SearchVolume {
	/// Keywords semantically around `seeds`, each with its own trailing-12-month series.
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

/// The one place that knows the set.
pub fn provider(name: &str) -> Result<Box<dyn SearchVolume>> {
	match name {
		"google_ads" => Ok(Box::new(google_ads::GoogleAds::from_env()?)),
		"dataforseo" => Ok(Box::new(dataforseo::DataForSeo::from_env()?)),
		_ => bail!("unknown searches.provider {name:?} — known: google_ads, dataforseo"),
	}
}

/// The Google canonical name of a country, which both providers resolve volume against. A row per
/// country a GBP can be in (`sa_core::gbp::Country`).
pub fn country_place(iso: &str) -> Result<&'static str> {
	const PLACES: &[(&str, &str)] = &[("FR", "France"), ("US", "United States"), ("CA", "Canada"), ("GB", "United Kingdom")];
	PLACES
		.iter()
		.find(|(c, _)| *c == iso)
		.map(|(_, p)| *p)
		.ok_or_else(|| eyre::eyre!("no canonical place known for country {iso:?} — add it to PLACES in searches/mod.rs"))
}

fn env(var: &str) -> Result<String> {
	std::env::var(var).map_err(|_| eyre::eyre!("{var} is not set"))
}
