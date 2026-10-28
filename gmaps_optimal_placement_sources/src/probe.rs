//! Asking Google the study's own question, from where a searcher would stand.
//!
//! The inventory sweep restricts to a rectangle, which no searcher stands in — so it says nothing
//! about how far a customer will look. The probe biases a circle around a node instead, and that is
//! the only observation that identifies the distance coefficient.
//!
//! The mask is `places.id,nextPageToken`: both are Text Search Essentials (IDs only), which the
//! free monthly allowance covers. Every attribute joins by id from the inventory already fetched.
use eyre::Result;

use crate::{
	poi::{self, Ranking, Region},
	work::{Kind, Need, Work},
};

const MASK: &str = "places.id,nextPageToken";
/// One page of 20. The partial likelihood scores the top 5 and the page's own tail is the
/// denominator, so a second page doubles the call count to lengthen a list nobody's choice turns
/// on — and `SearchTextRequestPerDayPerProject` defaults to 100, which one page per node fits.
const PAGES: usize = 1;

/// The request plan: one search per (node, term). Pure, so `--dry-run` prints exactly what a run
/// would spend and the fit can look each one up in the cache without a key.
pub fn plan(nodes: &[[f64; 2]], terms: &[String], radius_m: f64) -> Vec<(Ranking, serde_json::Value)> {
	let mut out = Vec::with_capacity(nodes.len() * terms.len());
	for node in nodes {
		for term in terms {
			let body = serde_json::json!({
				"textQuery": term,
				"pageSize": 20,
				"locationBias": {"circle": {"center": {"latitude": node[0], "longitude": node[1]}, "radius": radius_m}},
			});
			out.push((
				Ranking {
					from: Region::Node(*node, radius_m),
					term: term.clone(),
					ids: Vec::new(),
				},
				body,
			));
		}
	}
	out
}

/// Calls at most `PAGES` per plan entry. Cached, so a rerun is free.
pub fn run(work: &Work, what: &str, plan: Vec<(Ranking, serde_json::Value)>) -> Result<Vec<Ranking>> {
	work.announce(what, Need::Exact(unanswered(work, &plan)?));
	plan.into_iter()
		.map(|(mut r, body)| {
			r.ids = poi::search_text(work, true, &body, MASK, PAGES, Kind::Ordering)?.into_iter().map(|(id, ..)| id).collect();
			Ok(r)
		})
		.collect()
}

/// What a run would spend: one call per plan entry the work dir has no answer for, and every entry
/// under `--refresh`. Exact — a probe is one page from one point, and nothing about the answer
/// changes how many more there are to make.
pub fn unanswered(work: &Work, plan: &[(Ranking, serde_json::Value)]) -> Result<usize> {
	if work.refreshing() {
		return Ok(plan.len());
	}
	let mut n = 0;
	for (_, body) in plan {
		n += usize::from(work.cached(poi::SEARCH_TEXT, body)?.is_none());
	}
	Ok(n)
}

/// What of the plan is already on disk. A study that has never been probed yields nothing, which is
/// how the first fit runs on the inventory sweep alone.
pub fn cached(work: &Work, plan: &[(Ranking, serde_json::Value)]) -> Result<Vec<Ranking>> {
	let mut out = Vec::new();
	for (r, body) in plan {
		let ids: Vec<String> = poi::search_text(work, false, body, MASK, PAGES, Kind::Ordering)?.into_iter().map(|(id, ..)| id).collect();
		if !ids.is_empty() {
			out.push(Ranking { ids, ..r.clone() });
		}
	}
	Ok(out)
}
