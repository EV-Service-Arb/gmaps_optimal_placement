//! Keyword Planner through the Google Ads API. Read `docs/ARCHITECTURE.md` on what the numbers it
//! returns are and are not: they are bucketed, and an account with no spend gets the coarsest
//! buckets.
use std::cell::OnceCell;

use eyre::{Result, WrapErr, bail, ensure};

use super::{Keyword, Month, SearchVolume, env};
use crate::work::Work;

const API: &str = "https://googleads.googleapis.com/v21";

/// ISO-639-1 → the id in `languageConstants/{id}`. Google publishes ~50; adding one is a row.
const LANGUAGES: &[(&str, u32)] = &[("en", 1000), ("de", 1001), ("es", 1003), ("fr", 1040), ("it", 1004)];

pub struct GoogleAds {
	developer_token: String,
	client_id: String,
	client_secret: String,
	refresh_token: String,
	customer_id: String,
	/// One exchange per run, shared across groups.
	access: OnceCell<String>,
}

impl GoogleAds {
	pub fn from_env() -> Result<Self> {
		Ok(Self {
			developer_token: env("GOOGLE_ADS_DEVELOPER_TOKEN")?,
			client_id: env("GOOGLE_ADS_CLIENT_ID")?,
			client_secret: env("GOOGLE_ADS_CLIENT_SECRET")?,
			refresh_token: env("GOOGLE_ADS_REFRESH_TOKEN")?,
			// Google writes it with dashes and the API path wants it without
			customer_id: env("GOOGLE_ADS_CUSTOMER_ID")?.replace('-', ""),
			access: OnceCell::new(),
		})
	}

	/// Plain `ureq`, never [`Work::cached_post`] — that helper writes the response to disk and this
	/// one is a secret.
	fn access_token(&self) -> Result<&str> {
		if let Some(t) = self.access.get() {
			return Ok(t);
		}
		let body = ureq::post("https://oauth2.googleapis.com/token")
			.send_form([
				("client_id", self.client_id.as_str()),
				("client_secret", self.client_secret.as_str()),
				("refresh_token", self.refresh_token.as_str()),
				("grant_type", "refresh_token"),
			])
			.wrap_err("refreshing the Google Ads access token")?
			.into_body()
			.read_to_string()?;
		let json: serde_json::Value = serde_json::from_str(&body).wrap_err_with(|| format!("token endpoint returned non-JSON: {}", &body[..body.len().min(400)]))?;
		let Some(t) = json["access_token"].as_str() else {
			bail!("token endpoint returned no access_token: {json}")
		};
		Ok(self.access.get_or_init(|| t.to_owned()))
	}

	fn headers(&self) -> Result<[(&str, &str); 2]> {
		Ok([("Authorization", self.access_token()?), ("developer-token", self.developer_token.as_str())])
	}

	/// No closest-match fallback: a study asking about Clermont-Ferrand must not silently be answered
	/// about Auvergne.
	fn geo_target(&self, place: &str, lang: &str, work: &Work) -> Result<String> {
		let city = place
			.split(',')
			.next()
			.filter(|s| !s.is_empty())
			.ok_or_else(|| eyre::eyre!("searches.place {place:?} is empty"))?;
		let body = serde_json::json!({"locationNames": {"names": [city]}, "locale": lang});
		let auth = format!("Bearer {}", self.access_token()?);
		let res = work
			.cached_post(
				&format!("{API}/geoTargetConstants:suggest"),
				&body,
				&[("Authorization", &auth), ("developer-token", &self.developer_token)],
				&|res| res.get("error").map(|e| format!("geoTargetConstants:suggest for {city:?}: {e}")),
			)?
			.0;
		let mut seen = Vec::new();
		for s in res["geoTargetConstantSuggestions"].as_array().into_iter().flatten() {
			let g = &s["geoTargetConstant"];
			let (Some(canonical), Some(name)) = (g["canonicalName"].as_str(), g["resourceName"].as_str()) else {
				bail!("suggestion without a canonicalName/resourceName: {g}");
			};
			if canonical == place {
				return Ok(name.to_owned());
			}
			seen.push(canonical.to_owned());
		}
		bail!("no geo target has canonicalName {place:?}. Candidates for {city:?}:\n  {}", seen.join("\n  "))
	}
}

impl SearchVolume for GoogleAds {
	fn ideas(&self, seeds: &[String], place: &str, lang: &str, work: &Work) -> Result<Vec<Keyword>> {
		ensure!(!seeds.is_empty(), "a group needs at least one seed");
		let language = LANGUAGES
			.iter()
			.find(|(c, _)| *c == lang)
			.map(|(_, id)| format!("languageConstants/{id}"))
			.ok_or_else(|| eyre::eyre!("no languageConstants id known for {lang:?} — add it to LANGUAGES in searches/google_ads.rs"))?;
		let geo = self.geo_target(place, lang, work)?;
		let body = serde_json::json!({
			"keywordSeed": {"keywords": seeds},
			"geoTargetConstants": [geo],
			"language": language,
			"keywordPlanNetwork": "GOOGLE_SEARCH",
		});
		let url = format!("{API}/customers/{}:generateKeywordIdeas", self.customer_id);
		let refused = |res: &serde_json::Value| res.get("error").map(|e| format!("generateKeywordIdeas for {seeds:?}: {e}"));
		let res = work.cached_post(&url, &body, &self.headers()?, &refused)?.0;

		let mut out = Vec::new();
		for r in res["results"].as_array().into_iter().flatten() {
			let Some(text) = r["text"].as_str() else { bail!("keyword idea without text: {r}") };
			let volumes = r["keywordIdeaMetrics"]["monthlySearchVolumes"].as_array();
			let monthly = volumes
				.map(|v| {
					v.iter()
						.map(|m| {
							let (Some(year), Some(name)) = (m["year"].as_str(), m["month"].as_str()) else {
								bail!("monthly volume without year/month: {m}")
							};
							Ok(Month {
								year: year.parse().wrap_err_with(|| format!("year {year:?}"))?,
								month: month_number(name)?,
								// the field is absent, not zero, when nobody searched
								searches: m["monthlySearches"].as_str().map(str::parse).transpose()?.unwrap_or(0),
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

fn month_number(name: &str) -> Result<u8> {
	const NAMES: [&str; 12] = [
		"JANUARY", "FEBRUARY", "MARCH", "APRIL", "MAY", "JUNE", "JULY", "AUGUST", "SEPTEMBER", "OCTOBER", "NOVEMBER", "DECEMBER",
	];
	NAMES.iter().position(|n| *n == name).map(|i| i as u8 + 1).ok_or_else(|| eyre::eyre!("unknown month {name:?}"))
}
