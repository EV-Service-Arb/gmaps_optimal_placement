//! What the conductor's catalog pulls of placement: the committed `docs/openapi.json` is the one the
//! binary prints, every operation says what it refuses and who it opens for, and the one that buys
//! says what.
use std::process::Command;

fn printed() -> serde_json::Value {
	let out = Command::new(env!("CARGO_BIN_EXE_gmaps_optimal_placement")).arg("openapi").output().unwrap();
	assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
	serde_json::from_slice(&out.stdout).unwrap()
}

#[test]
fn the_committed_document_is_the_printed_one() {
	let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/openapi.json");
	let committed: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
	assert!(committed == printed(), "{} drifted from the table: `nix run .#gen`", path.display());
}

#[test]
fn every_operation_refuses_and_opens_by_name_and_only_the_pull_spends() {
	let doc = printed();
	let mut spending = Vec::new();
	for (path, item) in doc["paths"].as_object().unwrap() {
		for (method, op) in item.as_object().unwrap() {
			let at = format!("{method} {path}");
			assert!(op["x-errors"].as_object().is_some_and(|e| e.contains_key("unauthenticated")), "{at}: x-errors");
			assert!(op["x-permission"].as_str().is_some_and(|p| p.starts_with("sa:placement:")), "{at}: x-permission");
			if let Some(items) = op.get("x-spends") {
				spending.push((at, items.clone()));
			}
		}
	}
	let [(at, items)] = spending.as_slice() else {
		panic!("one operation spends, not {spending:?}")
	};
	assert_eq!(at, "post /studies/{sha}/pull");
	assert_eq!(doc["paths"]["/studies/{sha}/pull"]["post"]["x-permission"], "sa:placement:study:spend");
	let items: Vec<&str> = items.as_array().unwrap().iter().map(|i| i.as_str().unwrap()).collect();
	assert!(items.contains(&"placement.search"), "the person is charged the pull's searches: {items:?}");
	assert!(
		!items.contains(&"places.search"),
		"a Places search is what placement pays, not what the person is charged: {items:?}"
	);
}
