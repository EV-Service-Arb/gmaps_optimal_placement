//! The numbers the browser reads off a payload, pinned against the Clermont study.
//!
//! `clermont_payload.json` is one `car_detailing.nix` x `Clermont-Ferrand.nix` build — the
//! billed half of the pipeline, frozen, so this runs with no network and no key.
use gmaps_optimal_placement_core::{
	Model, Payload, Scale,
	model::{LayerRef, Report, TradeLayer, colorise, field, fmt},
};

fn model() -> Model {
	let payload: Payload = serde_json::from_str(include_str!("clermont_payload.json")).unwrap();
	Model::try_new(payload).unwrap()
}

/// The fixture is a trade over a location.
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
	let unmet = trade(&m).values(TradeLayer::Unmet, 0);
	let c = colorise(&unmet, &m.payload.imputed, false, Scale::Percentile, &everywhere(&m));

	insta::assert_snapshot!(format!(
		"cells {}\ncompetitors {}\nimputed {}\nshown {}\npressure nonzero {}\ndemand nonzero {}\ndemand total {:.1}\nunmet total {:.1}\npressure total {:.6}",
		m.n(),
		trade(&m).trade.pois.len(),
		m.payload.imputed.iter().filter(|&&i| i == 1).count(),
		c.count,
		nonzero(&trade(&m).trade.pressure),
		nonzero(trade(&m).demand().unwrap()),
		sum(trade(&m).demand().unwrap()),
		sum(&unmet),
		sum(&trade(&m).trade.pressure),
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

/// A hidden tier or kind takes its competitors out of the marks, and a search deselected takes what
/// they scored on it out of their size.
#[test]
fn the_selection_filters_the_marks() {
	let m = model();
	let base = trade(&m).selection();
	let mut no_wash = base.clone();
	no_wash.tiers.retain(|t| t != "wash");
	let mut first_term = base.clone();
	first_term.terms.iter_mut().skip(1).for_each(|on| *on = false);
	let mut no_commonest_kind = base.clone();
	no_commonest_kind.kinds.remove(0);

	let n = trade(&m).trade.pois.len();
	let marks = |sel| ((0..n).filter(|&i| trade(&m).counts(sel, i)).count(), (0..n).map(|i| trade(&m).weight(sel, i)).sum::<f64>());
	let (all, no_wash, first_term, no_kind) = (marks(&base), marks(&no_wash), marks(&first_term), marks(&no_commonest_kind));
	assert!(no_wash.0 < all.0, "a hidden tier leaves the marks");
	assert!(no_wash.1 < all.1, "a hidden tier leaves the marks");
	assert!(no_kind.0 < all.0, "a hidden kind leaves the marks");
	assert!(no_kind.1 < all.1, "a hidden kind leaves the marks");
	assert!(first_term.0 == all.0, "a competitor counts only on the searches selected");
	assert!(first_term.1 < all.1, "a competitor counts only on the searches selected");
	let show = |(shown, size): (usize, f64)| format!("{shown} shown, {size:.4} in all");
	insta::assert_snapshot!(
		format!("all {}\nno wash {}\nfirst term only {}\nno commonest kind {}", show(all), show(no_wash), show(first_term), show(no_kind)),
		@"
	all 140 shown, 245.9710 in all
	no wash 46 shown, 62.4404 in all
	first term only 140 shown, 155.0776 in all
	no commonest kind 50 shown, 94.5828 in all
	");
}

/// Every competitor's share of a cell is of the one pressure the payload carries, so they add up to
/// it wherever any reaches.
#[test]
fn the_shares_of_a_cell_sum_to_one() {
	let m = model();
	let mut total = vec![0.; m.n()];
	for of in 0..trade(&m).trade.pois.len() {
		for (t, s) in total.iter_mut().zip(trade(&m).values(TradeLayer::Share, of).iter()) {
			*t += s;
		}
	}
	let reached = |i: usize| f64::from(u8::from(trade(&m).trade.pressure[i] > 0.));
	let worst = total.iter().enumerate().map(|(i, t)| (t - reached(i)).abs()).fold(0., f64::max);
	assert!(worst < 1e-9, "a cell's shares add up to {worst:e} off one, or off none where nothing reaches");
}

#[test]
fn a_payload_whose_pressure_is_not_per_cell_is_refused() {
	let mut payload: Payload = serde_json::from_str(include_str!("clermont_payload.json")).unwrap();
	payload.trade.as_mut().unwrap().pressure.pop();
	assert!(
		Model::try_new(payload)
			.err()
			.expect("one value short")
			.to_string()
			.contains("10445 pressure values for 10446 cells")
	);
}

#[test]
fn top_ten_sites() {
	let m = model();
	let (picked, pool) = trade(&m).rank_sites(10).expect("the fixture carries a demand");

	let mut s = format!("{pool} cells tried\n");
	for (k, r) in picked.iter().enumerate() {
		s.push_str(&format!("{:>2}. {:<28} {:>6}  {:.5}, {:.5}\n", k + 1, m.payload.place[r.cell], fmt(r.score), r.lat, r.lng));
	}
	insta::assert_snapshot!(s, @"
	4617 cells tried
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
	insta::assert_snapshot!(render(&trade(&m).site_report([45.7797, 3.0863], None, Some("Nettoyage Auto Clermont"))), @r#"
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

/// Every layer colours cells, and `hide imputed` takes exactly the imputed ones out.
#[test]
fn every_layer_colours() {
	let m = model();

	let mut s = String::new();
	for spec in m.layers() {
		let v = match spec.source {
			LayerRef::Study(i) => std::borrow::Cow::Borrowed(m.layer(i)),
			LayerRef::Trade(t) => trade(&m).values(t, 0),
		};
		let shown = colorise(&v, &m.payload.imputed, false, spec.scale, &everywhere(&m));
		let hidden = colorise(&v, &m.payload.imputed, true, spec.scale, &everywhere(&m));
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
	Coverage · modelled           1339 shown,   961 without imputed, ticks 0.01 · 0.02 · 0.04 · 0.05 · 0.07
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
	let hit = m.cell_at([45.7797, 3.0863], false).expect("the study centre is on the grid");
	let hidden = m.payload.imputed.iter().position(|&i| i == 1).expect("the fixture carries imputed cells");
	let over = [m.c_lat[hidden], m.c_lng[hidden]];
	assert!(m.cell_at(over, false).is_some());
	assert!(m.cell_at(over, true).is_none_or(|i| m.payload.imputed[i] == 0), "a hidden cell answers nothing");
	insta::assert_snapshot!(m.tooltip(hit), @"
	Clermont-Ferrand
	Population 424
	Households 256
	Standard of living (€/yr) 29k
	Households in houses 2.00
	Estimated cars 219
	demand 339 · pressure 16.61 · unmet 19
	at most 60 competitors per search in each 7 km tile
	");
}

/// The view is what the colours are relative to: the heat ramp and the pressure field both span the
/// cells in view, so the centre of a city zoomed into is read against itself and not against its
/// countryside. The field's haze rises with pressure and is absent where there is none, and its
/// lines run at the levels it reports.
#[test]
fn the_view_sets_the_scale() {
	let m = model();
	let press = &trade(&m).trade.pressure;
	let centre: Vec<bool> = (0..m.n()).map(|i| (m.c_lat[i] - 45.7797).abs() < 0.02 && (m.c_lng[i] - 3.0863).abs() < 0.03).collect();

	let whole = colorise(press, &m.payload.imputed, false, Scale::Percentile, &everywhere(&m));
	let zoomed = colorise(press, &m.payload.imputed, false, Scale::Percentile, &centre);
	assert!(zoomed.count < whole.count, "the centre is a narrower, hotter range");
	assert!(zoomed.ticks[0] > whole.ticks[0], "the centre is a narrower, hotter range");

	let f = field(press, &centre).expect("the centre is under pressure");
	let mut by: Vec<(f64, u8)> = press.iter().copied().zip(f.alpha.iter().copied()).collect();
	by.sort_by(|a, b| a.0.total_cmp(&b.0));
	assert!(by.windows(2).all(|w| w[0].1 <= w[1].1), "haze never falls as pressure rises");
	assert!(by.iter().all(|(p, a)| (*p > 0.) || *a == 0), "no haze where nothing presses");

	let segs = m.contours(press, &f.levels);
	let per: Vec<usize> = (0..3).map(|l| segs.chunks(5).filter(|s| s[4] == l as f64).count()).collect();
	insta::assert_snapshot!(format!("levels {}\nsegments {per:?}\nalpha max {}", f.levels.map(fmt).join(" · "), f.alpha.iter().max().unwrap()), @"
	levels 14 · 17 · 19
	segments [55, 46, 28]
	alpha max 77
	");
}

/// A category that says nothing about demand still shows its competitors, and offers nothing that
/// reads a demand.
#[test]
fn without_a_demand_the_competitors_stand_alone() {
	let mut payload: Payload = serde_json::from_str(include_str!("clermont_payload.json")).unwrap();
	payload.trade.as_mut().unwrap().demand = None;
	let m = Model::try_new(payload).unwrap();
	assert!(sum(&trade(&m).trade.pressure) > 0., "the competitors still press on the map");
	assert!(trade(&m).rank_sites(10).is_none());
	let layers: Vec<String> = m.layers().into_iter().map(|l| l.name).collect();
	assert!(!layers.iter().any(|l| l.contains("emand")), "{layers:?}");
	let report = trade(&m).site_report([45.7797, 3.0863], None, None);
	assert!(!report.rows.iter().any(|r| r.label.contains("Capture") || r.label.contains("Demand")), "{report:?}");
}
