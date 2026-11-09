//! DataForSEO resells the same Keyword Planner numbers, so `location_name` is verbatim the Google
//! canonical name and switching providers is a one-word study edit.
use base64::{Engine, engine::general_purpose::STANDARD};
use eyre::{Result, bail, ensure};

use super::{Keyword, Month, SearchVolume, env};
use crate::work::Work;

pub struct DataForSeo {
	auth: String,
}

impl DataForSeo {
	pub fn from_env() -> Result<Self> {
		let (login, password) = (env("DATAFORSEO_LOGIN")?, env("DATAFORSEO_PASSWORD")?);
		Ok(Self {
			auth: format!("Basic {}", STANDARD.encode(format!("{login}:{password}"))),
		})
	}
}

impl SearchVolume for DataForSeo {
	fn ideas(&self, seeds: &[String], place: &str, lang: &str, work: &Work) -> Result<Vec<Keyword>> {
		ensure!(!seeds.is_empty(), "a group needs at least one seed");
		let body = serde_json::json!([{"keywords": seeds, "location_name": place, "language_code": lang, "sort_by": "search_volume"}]);
		// a refused task still answers 200
		let refused = |res: &serde_json::Value| {
			let task = &res["tasks"][0];
			(task["status_code"].as_u64() != Some(20000)).then(|| format!("keywords_for_keywords for {seeds:?}: {} {}", task["status_code"], task["status_message"]))
		};
		let res = work
			.cached_post(
				"https://api.dataforseo.com/v3/keywords_data/google_ads/keywords_for_keywords/live",
				&body,
				&[("Authorization", self.auth.as_str())],
				&refused,
			)?
			.0;
		let task = &res["tasks"][0];

		let mut out = Vec::new();
		for r in task["result"].as_array().into_iter().flatten() {
			let Some(text) = r["keyword"].as_str() else { bail!("result without a keyword: {r}") };
			let monthly = r["monthly_searches"]
				.as_array()
				.map(|v| {
					v.iter()
						.map(|m| {
							let (Some(year), Some(month), Some(searches)) = (m["year"].as_u64(), m["month"].as_u64(), m["search_volume"].as_u64()) else {
								bail!("monthly volume for {text:?} is incomplete: {m}")
							};
							Ok(Month {
								year: year as u16,
								month: month as u8,
								searches,
							})
						})
						.collect::<Result<Vec<_>>>()
				})
				.transpose()?;
			out.push(Keyword { text: text.to_owned(), monthly });
		}
		Ok(out)
	}
}
