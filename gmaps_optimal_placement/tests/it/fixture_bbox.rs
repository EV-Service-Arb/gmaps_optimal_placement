//! A few hundred cells of the real INSEE archive, through the real study's expressions. Snapshots
//! the numbers a model change would move, so it moves them visibly.
//!
//! Reads the bulk archive out of the real work dir, downloading it once if absent; the study is
//! kept in a work dir of its own. Nothing is held, so nothing billed is read.
use std::path::PathBuf;
use gmaps_optimal_placement::{Study, Work};
use gmaps_optimal_placement_core::{Area, Category, Country, Definition, Demand, Excluded, Gcid, Keyword, Model, Payload, Viewport};

use crate::{Answers, Dir};

/// The same few hundred cells every time, so two studies over them are comparable.
fn narrowed(category: Option<Category>) -> Study {
	let def = Definition {
		area: Area::City {
			slug: "clermont-ferrand".into(),
			country: Country::FR,
			viewport: Viewport {
				south: 45.770,
				west: 3.050,
				north: 45.790,
				east: 3.090,
			},
			margin_m: 0.,
		},
		category,
	};
	gmaps_optimal_placement::load(&def, Vec::new()).unwrap()
}

/// Built in `dir`, over the real work dir's archives.
fn built(study: &Study, dir: &Dir) -> Payload {
	let real = std::env::var_os("XDG_CACHE_HOME")
		.map(PathBuf::from)
		.unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").expect("a home to cache under")).join(".cache"))
		.join("gmaps_optimal_placement/data");
	std::fs::create_dir_all(&real).unwrap();
	std::os::unix::fs::symlink(&real, dir.0.join("data")).unwrap();
	study.build(&Work::at(&dir.0, Box::new(Answers::default()))).unwrap()
}

#[test]
fn clermont_centre() {
	let study = narrowed(Some(Category {
		trade: 3,
		name: "car_detailing".into(),
		country: Country::FR,
		primary: Gcid {
			id: "gcid:car_detailing_service".into(),
			name: "Service d'esthétique automobile".into(),
		},
		additional: Vec::new(),
		keywords: vec![
			Keyword {
				text: "service d'esthétique automobile".into(),
				weight: Some(1.),
			},
			Keyword {
				text: "lavage auto".into(),
				weight: Some(0.6),
			},
		],
		excluded: Excluded::default(),
		demand: Some(Demand {
			expr: "cars * (max(nv, 4000) / 22000) ^ 1.6".into(),
			columns: Vec::new(),
			layers: Vec::new(),
		}),
	}));
	let p = built(&study, &Dir::new("clermont_centre"));
	assert_eq!(p.name, "car_detailing·FR·clermont-ferrand", "the study names itself off its trade, country and area");
	assert!(p.trade.is_none(), "nothing is held, so no competitor is on it");
	let sum = |v: &[f64]| v.iter().sum::<f64>();
	let mut lines = vec![
		format!("cells           {}", p.place.len()),
		format!("imputed         {}", p.imputed.iter().filter(|&&i| i == 1).count()),
	];
	lines.extend(p.layers.iter().map(|l| format!("{:<15} {:.1}", l.name, sum(&l.values))));
	insta::assert_snapshot!(lines.join("\n"), @"
	cells           166
	imputed         6
	Demand          32330.6
	Population      43697.0
	Standard of living (€/yr) 4748488.4
	Estimated cars  23892.5
	Households      25879.9
	");
}

/// The claim this whole path exists for: a city can be looked at before a Places call is spent on
/// it. An area on its own asks nothing, and opens with no notice of anything lacking.
#[test]
fn a_location_alone_builds_off_the_archive() {
	let p = built(&narrowed(None), &Dir::new("location_alone"));
	assert_eq!(p.name, "clermont-ferrand", "with no category to apply, a study is named after the area");
	assert!(p.trade.is_none(), "a location carries no demand surface and no competitors");
	assert!(p.notice.is_none(), "and lacks nothing");
	let m = Model::try_new(p).unwrap();
	assert!(m.traded().is_none(), "and so no view onto anything a slider moves");
	insta::assert_snapshot!(m.layers().into_iter().map(|l| l.name).collect::<Vec<_>>().join("\n"), @"
	Population
	Standard of living (€/yr)
	Estimated cars
	Households
	");
}
