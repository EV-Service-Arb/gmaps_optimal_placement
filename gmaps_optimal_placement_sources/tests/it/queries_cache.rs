//! With the conductor's queries cache out of reach, a run goes on from what the work dir mirrors
//! and stops at the first answer it does not hold.

use gmaps_optimal_placement_sources::{Work, poi::SEARCH_TEXT, work::QueriesCache};
use sha2::{Digest, Sha256};

#[test]
fn an_unreachable_cache_serves_the_mirror_and_stops_at_a_miss() {
	let dir = std::env::temp_dir().join(format!("gmaps_queries_cache_{}", std::process::id()));
	let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap(); // dropped: nothing listens there
	let work = Work::at(&dir, QueriesCache::Conductor(format!("http://{closed}/api/queries_cache")));
	let (kept, missing) = (serde_json::json!({"textQuery": "kept"}), serde_json::json!({"textQuery": "missing"}));
	let key: String = Sha256::digest(format!("{SEARCH_TEXT}\n{}", serde_json::to_string(&kept).unwrap()))
		.iter()
		.map(|b| format!("{b:02x}"))
		.collect();
	let mirror = dir.join("data/places_cache");
	std::fs::create_dir_all(&mirror).unwrap();
	std::fs::write(mirror.join(format!("{key}.json")), r#"{"places":[]}"#).unwrap();

	assert_eq!(work.cached(&kept).unwrap().unwrap().0, serde_json::json!({"places": []}));
	let e = work.cached(&missing).unwrap_err();
	assert!(format!("{e:#}").contains("queries cache"), "{e:#}");
	std::fs::remove_dir_all(&dir).unwrap();
}
