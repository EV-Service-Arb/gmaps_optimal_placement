//! The harness, against a corpus with a known answer.
//!
//! A cross-validation harness that silently trains on its own test set still produces a plausible
//! table, and nothing downstream would notice. `FLAT`'s held-out likelihood has a closed form — the
//! uniform answer — so it pins the harness rather than restating it.
use gmaps_optimal_placement_core::{
	payload::Poi,
	rank::{K, N},
};
use gmaps_optimal_placement_rank::{Linear, Observed, Strategy, cv, fit};
use gmaps_optimal_placement_sources::{Ranking, Region};

/// What the corpus was drawn under. Distance is per km.
const TRUE: [f64; N] = [0.5, 1.2, 1.5, -0.3, -0.4];
const TERMS: [&str; 4] = ["plombier", "chauffagiste", "depannage plomberie", "sanitaire"];
const HEADS: [&str; 8] = ["Plomberie", "Chauffage", "Depannage", "Sanitaire", "Renovation", "Artisan", "Installation", "Entretien"];
const BIZ: usize = 12;
const NODES: usize = 60;

fn places() -> Vec<String> {
	vec!["Clermont-Ferrand".to_owned(), "Riom".to_owned()]
}

fn pois() -> Vec<Poi> {
	(0..BIZ)
		.map(|i| Poi {
			id: format!("p{i}"),
			// every third carries the city in its name, which is the only thing `place_token` reads
			name: format!("{} {} {i}", HEADS[i % HEADS.len()], if i % 3 == 0 { "Clermont" } else { "Sarl" }),
			addr: String::new(),
			lat: 45.75 + (i % 4) as f64 * 0.012,
			lng: 3.08 + (i / 4) as f64 * 0.015,
			rating: (i % 7 != 0).then_some(3.2 + (i % 9) as f64 * 0.2),
			n_rev: (i * 7 % 40) as f64 * 3.,
			kind: String::new(),
			kind_label: String::new(),
			web: String::new(),
			tel: String::new(),
			tier: "t".to_owned(),
			fetched_s: 0,
		})
		.collect()
}

/// Plackett–Luce by the Gumbel-max trick: one draw per business, the whole list sorted by
/// `score + gumbel`, which is exactly sampling the ordering without replacement.
fn corpus(pois: &[Poi]) -> Vec<Ranking> {
	// the same shrinkage target `Observed` will settle on, or the rating feature is not the one drawn
	let rated: Vec<f64> = pois.iter().filter_map(|p| p.rating).collect();
	let feats = gmaps_optimal_placement_core::rank::Feats::try_new(rated.iter().sum::<f64>() / rated.len() as f64, &places()).unwrap();
	let mut seed = 0x2545_f491_4f6c_dd1du64;
	let mut next = || {
		seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
		// away from both ends: `ln(0)` would poison the whole ordering
		((seed >> 11) as f64 / (1u64 << 53) as f64).clamp(1e-12, 1. - 1e-12)
	};

	let mut out = Vec::new();
	for n in 0..NODES {
		let at = [45.74 + (n % 9) as f64 * 0.008, 3.06 + (n / 9) as f64 * 0.008];
		for term in TERMS {
			let mut keyed: Vec<(f64, &Poi)> = pois
				.iter()
				.map(|p| {
					let x = feats.at(&(p.into()), term, Some(at));
					let s: f64 = (0..N).map(|i| TRUE[i] * x[i]).sum();
					(s - (-next().ln()).ln(), p)
				})
				.collect();
			keyed.sort_by(|a, b| b.0.total_cmp(&a.0));
			out.push(Ranking {
				from: Region::Node(at, 3000.),
				term: term.to_owned(),
				ids: keyed.iter().map(|(_, p)| p.id.clone()).collect(),
			});
		}
	}
	out
}

#[test]
fn the_table_earns_its_numbers() {
	let pois = pois();
	let observed = Observed::new(pois.clone(), places(), &corpus(&pois)).unwrap();
	let all = observed.orderings();
	assert_eq!(all.len(), NODES * TERMS.len());

	let (coef, _) = fit::adam(&fit::obs(&observed.feats, &all), [true; N]);
	for i in 0..N {
		assert!((coef[i] - TRUE[i]).abs() < 0.2, "coefficient {i} came back {:+.3} against {:+.3}", coef[i], TRUE[i]);
	}

	let flat = cv(&observed.feats, &all, &Linear::flat()).unwrap();
	// every score is zero, so the risk set contributes log of its own size and nothing else
	let uniform: f64 = all.iter().map(|o| (0..o.ranked.min(K)).map(|k| ((o.biz.len() - k) as f64).ln()).sum::<f64>()).sum::<f64>() / all.len() as f64;
	assert!((flat.nll - uniform).abs() < 1e-9, "FLAT scored {} where the uniform answer is {uniform}", flat.nll);

	let fitted = cv(&observed.feats, &all, &Linear::fitted()).unwrap();
	assert!(fitted.nll < flat.nll, "FITTED scored {:.3} held out against FLAT's {:.3}", fitted.nll, flat.nll);
	assert!(
		fitted.top3 > flat.top3,
		"FITTED agreed with the draw on {:.2} of the top 3 against FLAT's {:.2}",
		fitted.top3,
		flat.top3
	);
	assert_eq!(Linear::fitted().name(), "FITTED");
}
