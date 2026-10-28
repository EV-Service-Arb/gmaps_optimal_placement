//! A few hundred cells of the real INSEE archive, through the real study's expressions. Snapshots
//! the numbers a model change would move, so it moves them visibly.
//!
//! Reads the bulk archive out of the work dir, downloading it once if absent. No POI call
//! is made: [`Study::cells`] stops before the billed half.
use std::path::{Path, PathBuf};

use gmaps_optimal_placement::{Study, config::Area};
use gmaps_optimal_placement_core::grid::Bbox;
use gmaps_optimal_placement_sources::Work;

fn root() -> &'static Path {
	Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("crate lives in the workspace")
}

/// The same few hundred cells every time, so two studies over them are comparable.
fn narrowed(trade: Option<PathBuf>) -> Study {
	let mut study = gmaps_optimal_placement::load(trade.as_deref(), &root().join("examples/locations/Clermont-Ferrand.nix")).unwrap();
	study.area = Area {
		bbox: Bbox {
			lat: [45.770, 45.790],
			lon: [3.050, 3.090],
		},
		..study.area
	};
	study
}

#[test]
fn clermont_centre() {
	let study = narrowed(Some(root().join("examples/trades/car_detailing.nix")));
	assert_eq!(study.name, "car_detailing_-_Clermont-Ferrand", "the pairing names itself off the two stems");
	let c = study.cells(&Work::at(v_utils::xdg_cache_dir!(""))).unwrap();
	let sum = |v: &[f64]| v.iter().sum::<f64>();
	let mut lines = vec![
		format!("cells           {}", c.grid.len()),
		format!("imputed         {}", c.grid.cells.iter().filter(|x| x.imputed).count()),
		format!("columns         {}", c.grid.columns.len()),
		format!("demand total    {:.1}", sum(c.demand.as_ref().expect("a trade writes a demand model"))),
	];
	lines.extend(c.layers.iter().map(|l| format!("{:<15} {:.1}", l.name, sum(&l.values))));
	insta::assert_snapshot!(lines.join("\n"), @"
	cells           196
	imputed         9
	columns         32
	demand total    36501.2
	Population      49327.0
	Households      29006.8
	Standard of living (€/yr) 5571857.5
	Households in houses 3466.1
	Households in flats 25540.7
	Estimated cars  27082.0
	");
}

/// The claim this whole path exists for: a city can be looked at before a Places call is spent on
/// it. `build` is the only thing that ever buys an inventory, and there is no key in this process —
/// so if a location on its own ever reached `poi::load`, this would not return.
#[test]
fn a_location_alone_builds_off_the_archive() {
	let study = narrowed(None);
	assert_eq!(study.name, "Clermont-Ferrand", "with no trade to apply, a study is named after the city");

	let p = study.build(&Work::at(v_utils::xdg_cache_dir!(""))).unwrap();
	assert!(p.trade.is_none(), "a location carries no demand surface and no competitors");
	let m = gmaps_optimal_placement_core::Model::try_new(p).unwrap();
	assert!(m.traded().is_none(), "and so no view onto anything a slider moves");
	insta::assert_snapshot!(m.layers().into_iter().map(|l| l.name).collect::<Vec<_>>().join("\n"), @r"
	Population
	Households
	Standard of living (€/yr)
	Households in houses
	Households in flats
	");
}
