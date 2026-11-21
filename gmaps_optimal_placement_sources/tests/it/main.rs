mod geostat_fixture;
mod lattice;
mod tiers;

use std::{cell::RefCell, collections::HashMap, rc::Rc, time::SystemTime};

use gmaps_optimal_placement_core::Ask;
use gmaps_optimal_placement_sources::Held;

/// Answers a test hands in as the host would have bought them, fetched now.
#[derive(Clone, Default)]
struct Answers(Rc<RefCell<HashMap<String, serde_json::Value>>>);
impl Answers {
	fn answer(&self, ask: &Ask, answer: serde_json::Value) {
		self.0.borrow_mut().insert(ask.body.clone(), answer);
	}
}
impl Held for Answers {
	fn held(&self, ask: &Ask) -> eyre::Result<Option<(serde_json::Value, SystemTime)>> {
		Ok(self.0.borrow().get(&ask.body).map(|a| (a.clone(), SystemTime::now())))
	}
}
