//! Two areas that overlap share every answer either bought: the sweep asks tiles of one quadtree,
//! never tiles of the area that asked, so a second area over the same ground pays only for the
//! ground the first did not cover.
use gmaps_optimal_placement_core::{
	define::{Area, LatLng},
	grid::Bbox,
};
use gmaps_optimal_placement_sources::{
	Work,
	poi::{self, PoiConfig, SEARCH_TEXT, Tile, Tiling},
	work::{Age, QueriesCache},
};
use sha2::{Digest, Sha256};

const ZOOM: u32 = 12;

/// Where `Work` keeps the answer to the first page of "plombier" over `tile`.
fn answer(dir: &std::path::Path, tile: Tile, places: serde_json::Value) {
	let b: Bbox = tile.bbox();
	let rect = serde_json::json!({
		"low":  {"latitude": b.lat[0], "longitude": b.lon[0]},
		"high": {"latitude": b.lat[1], "longitude": b.lon[1]},
	});
	let body = serde_json::json!({"textQuery": "plombier", "pageSize": 20, "locationRestriction": {"rectangle": rect}});
	let key: String = Sha256::digest(format!("{SEARCH_TEXT}\n{}", serde_json::to_string(&body).unwrap()).as_bytes())
		.iter()
		.map(|b| format!("{b:02x}"))
		.collect();
	let at = dir.join("data/places_cache");
	std::fs::create_dir_all(&at).unwrap();
	std::fs::write(at.join(format!("{key}.json")), serde_json::json!({ "places": places }).to_string()).unwrap();
}

fn circle(slug: &str, lat: f64, lng: f64) -> Area {
	Area::Circle {
		slug: slug.into(),
		country: "FR".into(),
		center: LatLng { lat, lng },
		radius_m: 4000.,
		margin_m: 0.,
	}
}

#[test]
fn an_overlapping_area_pays_only_for_new_ground() {
	let dir = std::env::temp_dir().join(format!("gmaps_optimal_placement_test_lattice_{}", std::process::id()));
	if dir.exists() {
		std::fs::remove_dir_all(&dir).unwrap(); // a killed run's
	}
	let mut cfg: PoiConfig = serde_json::from_value(serde_json::json!({
		"source": "google_places",
		"tier": [{"name": "plumber", "weight": 1.0, "match": "plomb"}],
	}))
	.unwrap();
	cfg.queries = vec!["plombier".into()];
	let work = Work::at(&dir, QueriesCache::Local).policy(Age::default(), Tiling { zoom: ZOOM, depth: 3 }).block_spend(true);

	let (first, second) = (circle("a", 45.77, 3.08), circle("b", 45.77, 3.16));
	let (a, b) = (Tile::covering(first.frame().bbox, ZOOM), Tile::covering(second.frame().bbox, ZOOM));
	let shared = b.iter().filter(|t| a.contains(t)).count();
	assert!(shared > 0 && shared < b.len(), "the two circles overlap on part of the lattice: {a:?} vs {b:?}");

	assert_eq!(poi::cached(&cfg, first.frame(), &work).unwrap().missing, a.len(), "nothing is on disk yet");
	for &t in &a {
		let places = match t == a[0] {
			true =>
				serde_json::json!([{"id": "far", "displayName": {"text": "Plomberie du Bord"}, "location": t.bbox().lat.iter().zip(t.bbox().lon).map(|(lat, lon)| serde_json::json!({"latitude": lat, "longitude": lon})).next().unwrap()}]),
			false => serde_json::json!([]),
		};
		answer(&dir, t, places);
	}
	assert_eq!(poi::cached(&cfg, first.frame(), &work).unwrap().missing, 0, "the first area is answered in full");
	assert_eq!(
		poi::cached(&cfg, second.frame(), &work).unwrap().missing,
		b.len() - shared,
		"the second pays only for what the first did not ask"
	);
	assert!(
		poi::cached(&cfg, first.frame(), &work).unwrap().pois.is_empty(),
		"a competitor in a corner of a tile, outside the circle, is not on the circle's map"
	);
	std::fs::remove_dir_all(&dir).unwrap();
}
