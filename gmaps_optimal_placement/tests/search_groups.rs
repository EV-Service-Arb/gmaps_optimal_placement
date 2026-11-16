//! The group filter and its sum, over a canned expansion. No provider is reached: [`fold`] is the
//! half of `searches` that is ours, and the half a wrong number would come out of.
use gmaps_optimal_placement::{config::Group, fold};
use gmaps_optimal_placement_sources::{Keyword, Month};

const IDEAS: &str = include_str!("fixtures/detailing_ideas.json");

fn ideas() -> Vec<Keyword> {
	serde_json::from_str(IDEAS).unwrap()
}

fn group() -> Group {
	Group {
		name: "detailing".to_owned(),
		pattern: Some("detail|esthetique|esthétique|polissage|céramique".to_owned()),
		drop: Some("emploi|formation|stage".to_owned()),
	}
}

#[test]
fn filter_and_sum() {
	let (months, g) = fold(&group(), ideas()).unwrap();
	assert_eq!(months.len(), 12);
	assert_eq!(months[0], "2024-09");

	let kept: Vec<&str> = g.members.iter().map(|m| m.text.as_str()).collect();
	assert_eq!(kept, ["car detailing", "esthétique automobile"], "`drop` beats `match`, and a non-matching keyword stays out");
	assert_eq!(g.no_data, ["polissage céramique"]);

	// the sum is exactly the members, so the 500/mo formation keyword and the no-data one are absent
	assert_eq!(g.total, [40, 30, 20, 10, 30, 40, 70, 90, 120, 90, 70, 50]);
}

#[test]
fn a_disagreeing_month_axis_is_an_error() {
	let mut ideas = ideas();
	ideas[1].monthly.as_mut().unwrap()[0] = Month { year: 2023, month: 9, searches: 10 };
	let err = fold(&group(), ideas).unwrap_err().to_string();
	assert!(err.contains("2023-09"), "{err}");
}

#[test]
fn a_group_that_keeps_nothing_is_an_error() {
	let g = Group {
		pattern: Some("nothing whatsoever".to_owned()),
		..group()
	};
	assert!(fold(&g, ideas()).is_err());
}
