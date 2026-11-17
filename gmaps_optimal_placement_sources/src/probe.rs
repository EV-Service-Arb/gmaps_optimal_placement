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
	work::{Kind, Work},
};

const MASK: &str = "places.id,nextPageToken";
/// One page of 20. The partial likelihood scores the top 5 and the page's own tail is the
/// denominator, so a second page doubles the call count to lengthen a list nobody's choice turns
/// on — and `SearchTextRequestPerDayPerProject` defaults to 100, which one page per node fits.
const PAGES: usize = 1;

/// The request plan: one search per (node, term). Pure, so the fit can look each one up in the cache
/// without a key.
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

/// What of the plan is already on disk. A study that has never been probed yields nothing, which is
/// how the first fit runs on the inventory sweep alone.
pub fn cached(work: &Work, plan: &[(Ranking, serde_json::Value)]) -> Result<Vec<Ranking>> {
	let mut out = Vec::new();
	for (r, body) in plan {
		// what is lacking stays so: an ordering is never bought again
		let ids: Vec<String> = poi::search_text(work, body, MASK, PAGES, Kind::Ordering)?.0.into_iter().map(|(id, ..)| id).collect();
		if !ids.is_empty() {
			out.push(Ranking { ids, ..r.clone() });
		}
	}
	Ok(out)
}
