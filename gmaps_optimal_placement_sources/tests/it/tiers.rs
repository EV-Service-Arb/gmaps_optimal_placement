//! A competitor's tier is which search found it: the primary's searches (its name, each keyword) file
//! under tier 1, then each additional category's name search under its own, the first that found it
//! winning; Excluded is checked before any of them.
use std::collections::BTreeMap;

use gmaps_optimal_placement_sources::{
	Frame, Work,
	poi::{self, Tiling},
	work::{Age, QueriesCache},
};
use sa_core::{
	gbp::Country,
	geo::LatLng,
	placement::{Additional, Area, Category, Excluded, Gcid, Keyword},
	queries_cache::gop,
};
use sha2::{Digest, Sha256};

const AT: LatLng = LatLng { lat: 45.7797, lng: 3.0863 };

fn gcid(id: &str, name: &str) -> Gcid {
	Gcid { id: id.into(), name: name.into() }
}

fn place(id: &str, name: &str, primary: &str) -> serde_json::Value {
	serde_json::json!({
		"id": id, "displayName": {"text": name}, "primaryType": primary, "types": [primary],
		"location": {"latitude": AT.lat, "longitude": AT.lng},
	})
}

#[test]
fn a_competitor_is_tiered_by_the_search_that_found_it() {
	let dir = std::env::temp_dir().join(format!("gmaps_optimal_placement_test_tiers_{}", std::process::id()));
	if dir.exists() {
		std::fs::remove_dir_all(&dir).unwrap(); // a killed run's
	}
	let category = Category {
		trade: 1,
		name: "Plumbing".into(),
		country: Country::FR,
		primary: gcid("gcid:plumber", "Plombier"),
		additional: vec![
			Additional {
				google: gcid("gcid:general_contractor", "Entreprise générale"),
				weight: 0.3,
			},
			Additional {
				google: gcid("gcid:electrician", "Électricien"),
				weight: 0.2,
			},
		],
		keywords: vec![
			Keyword {
				text: "plombier".into(),
				weight: Some(1.),
			},
			Keyword {
				text: "chauffagiste".into(),
				weight: Some(0.5),
			},
			Keyword {
				text: "fuite d'eau".into(),
				weight: None,
			},
		],
		excluded: Excluded {
			words: vec!["magasin".into()],
			types: vec!["hardware_store".into()],
		},
		demand: None,
	};
	let area = Area::Circle {
		slug: "tiers".into(),
		country: Country::FR,
		center: AT,
		radius_m: 300.,
		margin_m: 0.,
	};
	let work = Work::at(&dir, QueriesCache::Local).policy(Age::default(), Tiling { zoom: 12, depth: 0 });

	let asked = poi::cached(&category, Frame::from(&area), &work).unwrap().asks;
	let mut queries: Vec<String> = Vec::new();
	for a in &asked {
		let q = serde_json::from_str::<serde_json::Value>(&a.body).unwrap()["textQuery"].as_str().unwrap().to_owned();
		if !queries.contains(&q) {
			queries.push(q);
		}
	}
	assert_eq!(
		queries,
		["Plombier", "chauffagiste", "fuite d'eau", "Entreprise générale", "Électricien"],
		"the primary's name, its keywords but the one that is its name, then each additional's name"
	);

	let answers = BTreeMap::from([
		(
			"Plombier",
			vec![
				place("a", "Dupont SARL", "general_contractor"),
				place("e", "Magasin Dupont", "plumber"),
				place("f", "Plomberie Shop", "hardware_store"),
			],
		),
		("chauffagiste", vec![place("b", "Martin", "electrician")]),
		("fuite d'eau", vec![place("i", "Express", "")]),
		("Entreprise générale", vec![place("b", "Martin", "electrician"), place("c", "Bati Pro", "plumber")]),
		("Électricien", vec![place("c", "Bati Pro", "plumber"), place("d", "Plombier Volt", "plumber")]),
	]);
	for a in &asked {
		let q = serde_json::from_str::<serde_json::Value>(&a.body).unwrap()["textQuery"].as_str().unwrap().to_owned();
		let key: String = Sha256::digest(gop(&a.body)).iter().map(|b| format!("{b:02x}")).collect();
		let at = dir.join("data/places_cache");
		std::fs::create_dir_all(&at).unwrap();
		std::fs::write(at.join(format!("{key}.json")), serde_json::json!({ "places": answers[q.as_str()] }).to_string()).unwrap();
	}

	let inv = poi::cached(&category, Frame::from(&area), &work).unwrap();
	assert!(inv.asks.is_empty(), "every search is answered: {:?}", inv.asks);
	let tiers: BTreeMap<&str, &str> = inv.pois.iter().map(|p| (p.id.as_str(), p.tier.as_str())).collect();
	assert_eq!(
		tiers,
		BTreeMap::from([
			("a", "Plombier"),            // the primary's name found it; its type is no tier's
			("b", "Plombier"),            // a keyword found it before an additional did
			("i", "Plombier"),            // an unweighted keyword is still a primary search word
			("c", "Entreprise générale"), // the first additional to find it
			("d", "Électricien"),         // its name and type say plumber, but only this search found it
		]),
		"e is excluded by a word in its name and f by its primary type, before any tier"
	);
	std::fs::remove_dir_all(&dir).unwrap();
}
