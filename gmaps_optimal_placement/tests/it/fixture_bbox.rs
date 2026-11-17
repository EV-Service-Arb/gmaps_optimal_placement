//! A few hundred cells of the real INSEE archive, through the real study's expressions. Snapshots
//! the numbers a model change would move, so it moves them visibly.
//!
//! Reads the bulk archive out of the work dir, downloading it once if absent. No POI call
//! is made: [`Study::cells`] stops before the billed half.
use std::path::Path;

use gmaps_optimal_placement::{Base, Study};
use gmaps_optimal_placement_sources::{Work, work::QueriesCache};
use sa_core::{
	gbp::Country,
	geo::Viewport,
	placement::{Area, Category, Definition, Gcid},
};

fn root() -> &'static Path {
	Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("crate lives in the workspace")
}

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
	gmaps_optimal_placement::load(&def, Base::Location(&root().join("examples/locations/Clermont-Ferrand.nix")), &root().join("examples/trades")).unwrap()
}

#[test]
fn clermont_centre() {
	let study = narrowed(Some(Category {
		trade: 3,
		country: Country::FR,
		study: "car_detailing".into(),
		keywords: vec!["lavage auto".into(), "nettoyage voiture".into()],
		google_primary: Gcid {
			id: "gcid:car_detailing_service".into(),
			name: "Service d'esthétique automobile".into(),
		},
		google_additional: Vec::new(),
	}));
	assert_eq!(study.name, "car_detailing·FR·clermont-ferrand", "the study names itself off its trade, country and area");
	let c = study.cells(&Work::at(v_utils::xdg_cache_dir!(""), QueriesCache::Local)).unwrap();
	let sum = |v: &[f64]| v.iter().sum::<f64>();
	let mut lines = vec![
		format!("cells           {}", c.grid.len()),
		format!("imputed         {}", c.grid.cells.iter().filter(|x| x.imputed).count()),
		format!("columns         {}", c.grid.columns.len()),
		format!("demand total    {:.1}", sum(c.demand.as_ref().expect("a trade writes a demand model"))),
	];
	lines.extend(c.layers.iter().map(|l| format!("{:<15} {:.1}", l.name, sum(&l.values))));
	insta::assert_snapshot!(lines.join("\n"), @"
	cells           166
	imputed         6
	columns         32
	demand total    32330.6
	Population      43697.0
	Standard of living (€/yr) 4748488.4
	Estimated cars  23892.5
	Households      25879.9
	");
}

/// The claim this whole path exists for: a city can be looked at before a Places call is spent on
/// it. `build` is the only thing that ever buys an inventory, and there is no key in this process —
/// so if a location on its own ever reached `poi::load`, this would not return.
#[test]
fn a_location_alone_builds_off_the_archive() {
	let study = narrowed(None);
	assert_eq!(study.name, "clermont-ferrand", "with no category to apply, a study is named after the area");

	let p = study.build(&Work::at(v_utils::xdg_cache_dir!(""), QueriesCache::Local)).unwrap();
	assert!(p.trade.is_none(), "a location carries no demand surface and no competitors");
	let m = gmaps_optimal_placement_core::Model::try_new(p).unwrap();
	assert!(m.traded().is_none(), "and so no view onto anything a slider moves");
	insta::assert_snapshot!(m.layers().into_iter().map(|l| l.name).collect::<Vec<_>>().join("\n"), @"
	Population
	Standard of living (€/yr)
	Estimated cars
	Households
	");
}
