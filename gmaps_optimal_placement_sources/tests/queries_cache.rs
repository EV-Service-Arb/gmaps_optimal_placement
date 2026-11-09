//! With the conductor's queries cache out of reach, a run goes on from what the work dir mirrors
//! and stops at the first answer it would have to buy: never bought behind the cache's back.

use gmaps_optimal_placement_sources::{
	Work,
	poi::SEARCH_TEXT,
	work::{Blocked, QueriesCache},
};
use sha2::{Digest, Sha256};

#[test]
fn an_unreachable_cache_serves_the_mirror_and_buys_nothing() {
	let dir = std::env::temp_dir().join(format!("gmaps_queries_cache_{}", std::process::id()));
	let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap(); // dropped: nothing listens there
	let work = Work::at(&dir, QueriesCache::Conductor(format!("http://{closed}/api/queries_cache"))).keys(vec!["k".into()]);
	let (kept, missing) = (serde_json::json!({"textQuery": "kept"}), serde_json::json!({"textQuery": "missing"}));
	let key: String = Sha256::digest(format!("{SEARCH_TEXT}\n{}", serde_json::to_string(&kept).unwrap()))
		.iter()
		.map(|b| format!("{b:02x}"))
		.collect();
	let mirror = dir.join("data/places_cache");
	std::fs::create_dir_all(&mirror).unwrap();
	std::fs::write(mirror.join(format!("{key}.json")), r#"{"places":[]}"#).unwrap();

	assert_eq!(work.cached_post(SEARCH_TEXT, &kept, &[], &|_| None).unwrap().0, serde_json::json!({"places": []}));
	let e = work.cached_post(SEARCH_TEXT, &missing, &[], &|_| None).unwrap_err();
	assert!(e.downcast_ref::<Blocked>().is_none() && format!("{e:#}").contains("queries cache"), "{e:#}");
	assert_eq!(work.billed(), 0);
	std::fs::remove_dir_all(&dir).unwrap();
}
