//! The numbers the map used to compute in the browser, pinned against the Clermont study.
//!
//! `clermont_payload.json` is one `car_detailing.nix` x `Clermont-Ferrand.nix` build — the
//! billed half of the pipeline, frozen, so this runs with no network and no key.
use gmaps_optimal_placement_core::{
	Model, Payload,
	model::{LayerRef, Report, TierState, colorise, field, fmt},
};

fn model() -> Model {
	let payload: Payload = serde_json::from_str(include_str!("clermont_payload.json")).unwrap();
	Model::try_new(payload).unwrap()
}

/// The fixture is a trade over a location, so the view every slider reads is always there.
fn trade(m: &Model) -> gmaps_optimal_placement_core::model::Traded<'_> {
	m.traded().expect("the Clermont fixture is a trade over a location")
}

fn everywhere(m: &Model) -> Vec<bool> {
	vec![true; m.n()]
}

fn sum(v: &[f64]) -> f64 {
	v.iter().sum()
}

fn nonzero(v: &[f64]) -> usize {
	v.iter().filter(|&&x| x > 0.).count()
}

fn render(r: &Report) -> String {
	let mut s = format!("{}\n", r.title);
	for row in &r.rows {
		s.push_str(&format!("  {:<28} {}\n", row.label, row.value));
		if let Some(sub) = &row.sub {
			s.push_str(&format!("    {sub}\n"));
		}
	}
	s
}

#[test]
fn state_matches_the_study() {
	let m = model();
	let tiers = m.tier_states();
	let press = trade(&m).pressure(trade(&m).trade.lambda_m, &tiers);
	let unmet = trade(&m).unmet(&press);
	let c = colorise(&unmet, &m.payload.imputed, false, false, &everywhere(&m));

	insta::assert_snapshot!(format!(
		"cells {}\ncompetitors {}\nimputed {}\nshown {}\npressure nonzero {}\ndemand nonzero {}\ndemand total {:.1}\nunmet total {:.1}\npressure total {:.6}",
		m.n(),
		trade(&m).trade.pois.len(),
		m.payload.imputed.iter().filter(|&&i| i == 1).count(),
		c.count,
		nonzero(&press),
		nonzero(&trade(&m).trade.demand),
		sum(&trade(&m).trade.demand),
		sum(&unmet),
		sum(&press),
	), @"
	cells 10446
	competitors 140
	imputed 6532
	shown 10446
	pressure nonzero 8576
	demand nonzero 10446
	demand total 346002.4
	unmet total 117654.5
	pressure total 27340.837444
	");
}

/// The three sensitivities `smoke.js` drove the page for: dropping a tier lowers pressure, a wider
/// catchment raises it, and recompute is a function of its inputs alone.
#[test]
fn pressure_moves_with_the_controls() {
	let m = model();
	let base = m.tier_states();
	let mut no_wash = base.clone();
	no_wash.iter_mut().find(|t| t.name == "wash").unwrap().show = false;

	let p_base = sum(&trade(&m).pressure(trade(&m).trade.lambda_m, &base));
	let p_no_wash = sum(&trade(&m).pressure(trade(&m).trade.lambda_m, &no_wash));
	let p_wide = sum(&trade(&m).pressure(4000., &base));
	let p_again = sum(&trade(&m).pressure(trade(&m).trade.lambda_m, &base));

	assert!(p_no_wash < p_base, "dropping washes must lower pressure");
	assert!(p_wide > p_base, "wider λ must raise pressure");
	assert_eq!(p_base, p_again, "recompute is deterministic");

	insta::assert_snapshot!(format!("base {p_base:.6}\nno wash {p_no_wash:.6}\nlambda 4000 {p_wide:.6}"), @"
	base 27340.837444
	no wash 12574.450215
	lambda 4000 87845.339268
	");
}

#[test]
fn top_ten_sites() {
	let m = model();
	let tiers = m.tier_states();
	let press = trade(&m).pressure(trade(&m).trade.lambda_m, &tiers);
	let (picked, pool) = trade(&m).rank_sites(trade(&m).trade.lambda_m, &press, 10);

	let mut s = format!("{pool} candidate cells\n");
	for (k, r) in picked.iter().enumerate() {
		s.push_str(&format!("{:>2}. {:<28} {:>6}  {:.5}, {:.5}\n", k + 1, m.payload.place[r.cell], fmt(r.score), r.lat, r.lng));
	}
	insta::assert_snapshot!(s, @"
	4617 candidate cells
	 1. Châtel-Guyon                   7.7k  45.91457, 3.07977
	 2. Saint-Bonnet-près-Riom         7.4k  45.92767, 3.11433
	 3. Châtel-Guyon                   7.2k  45.92432, 3.06300
	 4. Yssac-la-Tourette              7.0k  45.92975, 3.09077
	 5. Riom                           6.7k  45.90678, 3.09888
	 6. Davayat                        6.6k  45.94347, 3.10714
	 7. Châtel-Guyon                   6.6k  45.90931, 3.05457
	 8. Mozac                          6.1k  45.89973, 3.07391
	 9. Châtel-Guyon                   6.1k  45.93917, 3.06886
	10. Chambaron sur Morge            6.0k  45.93976, 3.13349
	");
}

#[test]
fn site_report_downtown() {
	let m = model();
	let tiers = m.tier_states();
	let press = trade(&m).pressure(trade(&m).trade.lambda_m, &tiers);
	insta::assert_snapshot!(render(&trade(&m).site_report([45.7797, 3.0863], None, Some("Nettoyage Auto Clermont"), trade(&m).trade.lambda_m, &press, &tiers)), @r#"
	Clermont-Ferrand
	  Capture score                3.9k
	  Demand ≤1 / ≤3 km            16k / 81k
	  Nearest detail               0.19 km
	    Clermont Nettoyage Auto et P
	  detail ≤2 / ≤5 km            6 / 17
	  Nearest wash                 0.59 km
	    Lavage auto EXEPXION
	  wash ≤2 / ≤5 km              7 / 29
	  Nearest competitor, any      0.19 km
	  All competitors ≤2 km        13
	  Coordinates                  45.77970, 3.08630
	  Open as "Nettoyage Auto Clermont", no reviews 0.18 of a median rival
	    reviews are associated with rank, not a lever on it
	  Outranks, of the ≤2 km field 1 of 13
	"#);
}

#[test]
fn candidates_compare() {
	let m = model();
	let tiers = m.tier_states();
	let press = trade(&m).pressure(trade(&m).trade.lambda_m, &tiers);
	insta::assert_snapshot!(render(&trade(&m).compare(&m.payload.candidates, trade(&m).trade.lambda_m, &press)), @"
	Candidates · λ=2.0 km
	  Capture score                
	  VifNet                       4.8k · 100%
	  Demand ≤1 / ≤3 km            
	  VifNet                       15k / 70k
	  Nearest competitor · ≤2 km   
	  VifNet                       1.01 km · 9
	  Opening weight, under this name, no reviews 
	  VifNet                       0.11 of a median rival
	");
}

/// Every layer colours cells, and `hide imputed` takes exactly the imputed ones out.
#[test]
fn every_layer_colours() {
	let m = model();
	let tiers = m.tier_states();
	let press = trade(&m).pressure(trade(&m).trade.lambda_m, &tiers);
	let unmet = trade(&m).unmet(&press);

	let mut s = String::new();
	for spec in m.layers() {
		let v = match spec.source {
			LayerRef::Study(i) => std::borrow::Cow::Borrowed(m.layer(i)),
			LayerRef::Trade(t) => trade(&m).values(t, 0, trade(&m).trade.lambda_m, &tiers, &unmet),
		};
		let shown = colorise(&v, &m.payload.imputed, false, spec.linear, &everywhere(&m));
		let hidden = colorise(&v, &m.payload.imputed, true, spec.linear, &everywhere(&m));
		s.push_str(&format!(
			"{:<28} {:>5} shown, {:>5} without imputed, ticks {}\n",
			spec.name,
			shown.count,
			hidden.count,
			shown.ticks.map(fmt).join(" · ")
		));
	}
	insta::assert_snapshot!(s, @"
	Underserved demand  ★        10446 shown,  3914 without imputed, ticks 0.01 · 2.14 · 6.63 · 15 · 162
	Demand                       10446 shown,  3914 without imputed, ticks 0.23 · 3.65 · 13 · 45 · 786
	Coverage · modelled          10446 shown,  3914 without imputed, ticks 0.00 · 0.02 · 0.03 · 0.05 · 0.06
	Population                   10446 shown,  3914 without imputed, ticks 1.00 · 4.00 · 15 · 50 · 1.3k
	Households                   10446 shown,  3914 without imputed, ticks 0.30 · 1.90 · 6.20 · 21 · 828
	Standard of living (€/yr)    10446 shown,  3914 without imputed, ticks 11k · 19k · 27k · 35k · 43k
	Households in houses         10398 shown,  3869 without imputed, ticks 0.10 · 1.70 · 5.70 · 17 · 175
	Estimated cars               10446 shown,  3914 without imputed, ticks 0.33 · 2.79 · 9.46 · 31 · 704
	");
}

/// The tooltip is the only place the per-cell numbers are ever read next to each other.
#[test]
fn tooltip_over_a_cell() {
	let m = model();
	let tiers: Vec<TierState> = m.tier_states();
	let press = trade(&m).pressure(trade(&m).trade.lambda_m, &tiers);
	let unmet = trade(&m).unmet(&press);
	let shown = colorise(&unmet, &m.payload.imputed, false, false, &everywhere(&m)).shown;
	let hit = m.cell_at(45.7797, 3.0863, &shown).expect("the study centre is on the grid");
	insta::assert_snapshot!(m.tooltip(hit, &press, &unmet), @"
	Clermont-Ferrand
	Population 424
	Households 256
	Standard of living (€/yr) 29k
	Households in houses 2.00
	Estimated cars 219
	demand 339 · pressure 16.61 · unmet 19
	");
}

/// The view is what the colours are relative to: the heat ramp and the pressure field both span the
/// cells in view, so the centre of a city zoomed into is read against itself and not against its
/// countryside. The field's haze rises with pressure and is absent where there is none, and its
/// lines run at the levels it reports.
#[test]
fn the_view_sets_the_scale() {
	let m = model();
	let tiers = m.tier_states();
	let press = trade(&m).pressure(trade(&m).trade.lambda_m, &tiers);
	let centre: Vec<bool> = (0..m.n()).map(|i| (m.c_lat[i] - 45.7797).abs() < 0.02 && (m.c_lng[i] - 3.0863).abs() < 0.03).collect();

	let whole = colorise(&press, &m.payload.imputed, false, false, &everywhere(&m));
	let zoomed = colorise(&press, &m.payload.imputed, false, false, &centre);
	assert!(zoomed.count < whole.count && zoomed.ticks[0] > whole.ticks[0], "the centre is a narrower, hotter range");

	let f = field(&press, &centre).expect("the centre is under pressure");
	let mut by: Vec<(f64, u8)> = press.iter().copied().zip(f.alpha.iter().copied()).collect();
	by.sort_by(|a, b| a.0.total_cmp(&b.0));
	assert!(by.windows(2).all(|w| w[0].1 <= w[1].1), "haze never falls as pressure rises");
	assert!(by.iter().all(|(p, a)| (*p > 0.) || *a == 0), "no haze where nothing presses");

	let segs = m.contours(&press, &f.levels);
	let per: Vec<usize> = (0..3).map(|l| segs.chunks(5).filter(|s| s[4] == l as f64).count()).collect();
	insta::assert_snapshot!(format!("levels {}\nsegments {per:?}\nalpha max {}", f.levels.map(fmt).join(" · "), f.alpha.iter().max().unwrap()), @"
	levels 14 · 17 · 19
	segments [55, 46, 28]
	alpha max 77
	");
}
