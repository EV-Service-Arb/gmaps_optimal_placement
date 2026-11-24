//! A pull, as its host drives it: ask what a study lacks, buy it, ask again until nothing is lacking,
//! then build. Over the `testing` fixture, so it runs offline on real Filosofi rows.
use std::time::{Duration, SystemTime};

use gmaps_optimal_placement::{Work, testing};
use gmaps_optimal_placement_core::{Area, Ask, Category, Country, Definition, Demand, Excluded, Gcid, Keyword, Model, Payload};

use crate::{Answers, Dir};

struct Pull {
	dir: Dir,
	held: Answers,
	def: Definition,
}
impl Pull {
	fn new(name: &str) -> Self {
		let dir = Dir::new(name);
		testing::fixture(&dir.0).unwrap();
		let def = Definition {
			area: Area::Circle {
				slug: "clermont-centre".into(),
				country: Country::FR,
				center: testing::CENTER,
				radius_m: testing::RADIUS_M,
				margin_m: 0.,
			},
			category: Some(Category {
				trade: Gcid {
					id: "gcid:plumber".into(),
					name: "Plombier".into(),
				},
				country: Country::FR,
				additional: Vec::new(),
				keywords: vec![Keyword {
					text: "plombier".into(),
					weight: Some(1.),
				}],
				excluded: Excluded::default(),
				demand: Some(Demand {
					expr: "ind".into(),
					columns: Vec::new(),
					layers: Vec::new(),
				}),
			}),
		};
		Self { dir, held: Answers::default(), def }
	}

	fn work(&self, since: Option<SystemTime>) -> Work {
		Work::at(&self.dir.0, Box::new(self.held.clone())).since(since)
	}

	fn asks(&self, since: Option<SystemTime>) -> Vec<Ask> {
		gmaps_optimal_placement::load(&self.def).unwrap().asks(&self.work(since)).unwrap()
	}

	fn build(&self) -> Payload {
		gmaps_optimal_placement::load(&self.def).unwrap().build(&self.work(None)).unwrap()
	}
}

fn plumber() -> serde_json::Value {
	serde_json::json!({
		"id": "stub", "displayName": {"text": "Stub plombier"}, "rating": 4.5, "userRatingCount": 10,
		"location": {"latitude": testing::CENTER.lat, "longitude": testing::CENTER.lng},
	})
}

/// Each round names what lies behind the last one's answers, until none is lacking; the study then
/// opens whole, and is kept where `fit` harvests it.
#[test]
fn a_pull_is_asked_what_lies_behind_each_answer_until_none_is_lacking() {
	let p = Pull::new("pull_rounds");
	let now = SystemTime::now();

	let first = p.asks(None);
	assert!(!first.is_empty(), "nothing is held yet");
	assert!(first.iter().all(|a| !a.body.contains("pageToken")), "a first page down each branch: {first:?}");
	let lacking = p.build().notice.expect("a study lacking searches says so");
	assert_eq!(lacking.missing, Some(first.len()), "{}", lacking.text);

	for (i, a) in first.iter().enumerate() {
		let mut answer = serde_json::json!({ "places": [plumber()] });
		if i == 0 {
			answer["nextPageToken"] = "more".into();
		}
		p.held.answer(a, answer, now);
	}
	let [behind] = p
		.asks(None)
		.try_into()
		.unwrap_or_else(|a: Vec<Ask>| panic!("only the second page is behind what was answered: {a:?}"));
	assert!(behind.body.contains(r#""pageToken":"more""#), "{behind:?}");
	p.held.answer(&behind, serde_json::json!({ "places": [] }), now);
	assert!(p.asks(None).is_empty(), "the sweep is held whole");

	let built = p.build();
	assert!(built.notice.is_none(), "built of what was answered: {:?}", built.notice);
	assert_eq!(built.trade.as_ref().expect("the sweep found a competitor").pois.len(), 1);
	let opened = Model::try_new(built).expect("a page opens what was built");
	let pressed = opened.traded().expect("on its trade").trade.pressure.iter().filter(|p| **p > 0.).count();
	assert_eq!(pressed, opened.n(), "its one competitor reaches every cell");
	assert!(p.dir.0.join(format!("studies/{}.json", p.def.id())).is_file(), "the definition is kept for `fit`");
}

/// A pull re-asks what was fetched before it began, so a retry pays for none of what a stopped one bought.
#[test]
fn an_answer_fetched_before_since_is_lacking() {
	let p = Pull::new("pull_since");
	let (then, now) = (SystemTime::now() - Duration::from_secs(3600), SystemTime::now());
	let first = p.asks(None);
	for a in &first {
		p.held.answer(a, serde_json::json!({ "places": [plumber()] }), then);
	}
	assert!(p.asks(None).is_empty(), "held whole");
	assert_eq!(p.asks(Some(now)), first, "a pull since now re-asks everything fetched before it");
	assert!(p.asks(Some(then)).is_empty(), "and nothing fetched since");
}
