mod fixture_bbox;
mod pull;

use std::path::PathBuf;
use std::{cell::RefCell, collections::HashMap, rc::Rc, time::SystemTime};

use gmaps_optimal_placement::Held;
use gmaps_optimal_placement_core::Ask;

/// Answers a test hands in as its host would have bought them, each as of when it was fetched.
#[derive(Clone, Default)]
struct Answers(Rc<RefCell<HashMap<String, (serde_json::Value, SystemTime)>>>);
impl Answers {
	fn answer(&self, ask: &Ask, answer: serde_json::Value, at: SystemTime) {
		self.0.borrow_mut().insert(ask.body.clone(), (answer, at));
	}
}
impl Held for Answers {
	fn held(&self, ask: &Ask) -> eyre::Result<Option<(serde_json::Value, SystemTime)>> {
		Ok(self.0.borrow().get(&ask.body).cloned())
	}
}

/// A work dir of its own, gone with the test.
struct Dir(PathBuf);
impl Dir {
	fn new(name: &str) -> Self {
		let p = std::env::temp_dir().join(format!("gmaps_optimal_placement_test_{name}_{}", std::process::id()));
		if p.exists() {
			std::fs::remove_dir_all(&p).unwrap(); // a killed run's
		}
		std::fs::create_dir_all(&p).unwrap();
		Self(p)
	}
}
impl Drop for Dir {
	fn drop(&mut self) {
		std::fs::remove_dir_all(&self.0).unwrap();
	}
}
