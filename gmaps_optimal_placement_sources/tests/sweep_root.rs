//! A query swept under one root tiling is served from disk under any other, because `sweep.root` says
//! how a new sweep is bought and never re-buys one already answered — even where a run under the new
//! root began and was stopped.
use gmaps_optimal_placement_core::grid::Bbox;
use gmaps_optimal_placement_sources::{
	Work,
	poi::{self, PoiConfig, SEARCH_TEXT, Tiling},
	work::Age,
};
use sha2::{Digest, Sha256};

const BBOX: Bbox = Bbox {
	lat: [45.70, 45.85],
	lon: [3.00, 3.20],
};

/// The body `poi` asks with for tile `(i, j)` of `root` a side, which is its cache key.
fn tile(root: u32, i: u32, j: u32) -> serde_json::Value {
	let (dlat, dlon) = ((BBOX.lat[1] - BBOX.lat[0]) / f64::from(root), (BBOX.lon[1] - BBOX.lon[0]) / f64::from(root));
	let rect = serde_json::json!({
		"low":  {"latitude": BBOX.lat[0] + f64::from(i) * dlat, "longitude": BBOX.lon[0] + f64::from(j) * dlon},
		"high": {"latitude": BBOX.lat[0] + f64::from(i + 1) * dlat, "longitude": BBOX.lon[0] + f64::from(j + 1) * dlon},
	});
	serde_json::json!({"textQuery": "plombier", "pageSize": 20, "locationRestriction": {"rectangle": rect}})
}

/// Where `Work` keeps the answer to this exact request.
fn answer(dir: &std::path::Path, body: &serde_json::Value, places: serde_json::Value) {
	let key: String = Sha256::digest(format!("{SEARCH_TEXT}\n{}", serde_json::to_string(body).unwrap()).as_bytes())
		.iter()
		.map(|b| format!("{b:02x}"))
		.collect();
	let at = dir.join("data/places_cache");
	std::fs::create_dir_all(&at).unwrap();
	std::fs::write(at.join(format!("{key}.json")), serde_json::json!({ "places": places }).to_string()).unwrap();
}

#[test]
fn a_changed_root_serves_what_the_old_one_paid_for() {
	let dir = std::env::temp_dir().join(format!("gmaps_optimal_placement_test_sweep_root_{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&dir);
	for i in 0..3 {
		for j in 0..3 {
			let places = match (i, j) {
				(0, 0) => serde_json::json!([{
					"id": "a",
					"displayName": {"text": "Plomberie Aubière"},
					"location": {"latitude": 45.72, "longitude": 3.02},
				}]),
				_ => serde_json::json!([]),
			};
			answer(&dir, &tile(3, i, j), places);
		}
	}
	// a root-2 run stopped by the quota after its first tile
	answer(&dir, &tile(2, 0, 0), serde_json::json!([]));
	let cfg: PoiConfig = serde_json::from_value(serde_json::json!({
		"source": "google_places",
		"queries": ["plombier"],
		"tier": [{"name": "plumber", "weight": 1.0, "match": "plomb"}],
	}))
	.unwrap();
	let work = Work::at(&dir).policy(Age::default(), Tiling { root: 2, depth: 3 }).block_spend(true);

	let inv = poi::cached(&cfg, BBOX, &work).unwrap();
	assert_eq!(inv.missing, 0, "every root-3 tile is on disk, so nothing is left to buy");
	assert_eq!(inv.pois.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Plomberie Aubière"]);
	std::fs::remove_dir_all(&dir).unwrap();
}
