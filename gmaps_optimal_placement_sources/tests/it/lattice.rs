//! Two areas that overlap share every answer either bought: the sweep asks tiles of one quadtree,
//! never tiles of the area that asked, so a second area over the same ground pays only for the
//! ground the first did not cover.
use gmaps_optimal_placement_core::{Area, Ask, Category, Country, Excluded, Gcid, Keyword, LatLng, grid::Bbox};
use gmaps_optimal_placement_sources::{
	Frame, Work,
	poi::{self, Tile, Tiling},
	work::Age,
};

use crate::Answers;

const ZOOM: u32 = 12;

/// The first page of "plombier" over `tile`, answered.
fn answer(held: &Answers, tile: Tile, places: serde_json::Value) {
	let b: Bbox = tile.bbox();
	let rect = serde_json::json!({
		"low":  {"latitude": b.lat[0], "longitude": b.lon[0]},
		"high": {"latitude": b.lat[1], "longitude": b.lon[1]},
	});
	let body = serde_json::json!({"textQuery": "plombier", "pageSize": 20, "locationRestriction": {"rectangle": rect}});
	let ask = Ask {
		mask: String::new(),
		body: serde_json::to_string(&body).unwrap(),
	};
	held.answer(&ask, serde_json::json!({ "places": places }));
}

fn circle(slug: &str, lat: f64, lng: f64) -> Area {
	Area::Circle {
		slug: slug.into(),
		country: Country::FR,
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
	let cfg = Category {
		trade: 1,
		name: "Plumbing".into(),
		country: Country::FR,
		primary: Gcid {
			id: "gcid:plumber".into(),
			name: "plombier".into(),
		},
		additional: Vec::new(),
		keywords: vec![Keyword {
			text: "plombier".into(),
			weight: Some(1.),
		}],
		excluded: Excluded::default(),
		demand: None,
	};
	let held = Answers::default();
	let work = Work::at(&dir, Box::new(held.clone())).policy(Age::default(), Tiling { zoom: ZOOM, depth: 3 });

	let (first, second) = (circle("a", 45.77, 3.08), circle("b", 45.77, 3.16));
	let (a, b) = (Tile::covering(Frame::from(&first).bbox, ZOOM), Tile::covering(Frame::from(&second).bbox, ZOOM));
	let shared = b.iter().filter(|t| a.contains(t)).count();
	assert!(shared > 0, "the two circles overlap on part of the lattice: {a:?} vs {b:?}");
	assert!(shared < b.len(), "the two circles overlap on part of the lattice: {a:?} vs {b:?}");

	assert_eq!(poi::cached(&cfg, Frame::from(&first), &work).unwrap().asks.len(), a.len(), "nothing is on disk yet");
	for &t in &a {
		let places = match t == a[0] {
			true =>
				serde_json::json!([{"id": "far", "displayName": {"text": "Plomberie du Bord"}, "location": t.bbox().lat.iter().zip(t.bbox().lon).map(|(lat, lon)| serde_json::json!({"latitude": lat, "longitude": lon})).next().unwrap()}]),
			false => serde_json::json!([]),
		};
		answer(&held, t, places);
	}
	assert_eq!(poi::cached(&cfg, Frame::from(&first), &work).unwrap().asks.len(), 0, "the first area is answered in full");
	assert_eq!(
		poi::cached(&cfg, Frame::from(&second), &work).unwrap().asks.len(),
		b.len() - shared,
		"the second pays only for what the first did not ask"
	);
	assert!(
		poi::cached(&cfg, Frame::from(&first), &work).unwrap().pois.is_empty(),
		"a competitor in a corner of a tile, outside the circle, is not on the circle's map"
	);
	if dir.exists() {
		std::fs::remove_dir_all(&dir).unwrap();
	}
}
