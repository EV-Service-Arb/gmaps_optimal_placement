//! The ranking model, against the frozen Clermont payload and against data made to a known answer.
//!
//! The gradient check is the one that matters: a wrong derivative produces plausible, confidently
//! wrong coefficients, and nothing downstream would notice.
use gmaps_optimal_placement_core::{
	Payload, Trade,
	payload::Poi,
	rank::{self, Biz, DIST, Feats, N, NAME, NAMES, Obs, nll_grad, nodes},
};

fn payload() -> Payload {
	serde_json::from_str(include_str!("clermont_payload.json")).unwrap()
}

/// The frozen payload is a built study, so it carries the half a trade decides.
fn trade() -> Trade {
	payload().trade.expect("the Clermont fixture is a trade over a location")
}

fn feats() -> Feats {
	let p = payload();
	let rated: Vec<f64> = trade().pois.iter().filter_map(|q| q.poi.rating).collect();
	Feats::try_new(rated.iter().sum::<f64>() / rated.len() as f64, &p.place).unwrap()
}

/// Deterministic and spread over every feature, so no direction is left untested.
fn synthetic(coef: [f64; N]) -> Vec<Obs> {
	let mut obs = Vec::new();
	for o in 0..24 {
		let mut x: Vec<[f64; N]> = (0..9)
			.map(|i| {
				let t = (o * 9 + i) as f64;
				[
					((t * 0.37) % 5.).ln_1p(),
					((t * 0.11) % 2.) - 1.,
					((t * 0.29) % 1.01).min(1.),
					f64::from((t as u32).is_multiple_of(3)),
					(t * 0.23) % 9.,
				]
			})
			.collect();
		// the ordering *is* the argmax under `coef`, and only the top 4 survive the page cap
		x.sort_by(|a, b| {
			let s = |v: &[f64; N]| (0..N).map(|i| coef[i] * v[i]).sum::<f64>();
			s(b).total_cmp(&s(a))
		});
		obs.push(Obs { x, ranked: 4 });
	}
	obs
}

#[test]
fn the_gradient_is_the_derivative_of_the_likelihood() {
	let obs = synthetic([0.5, 0.3, 1.2, -0.4, -0.2]);
	let at = [0.31, -0.22, 0.74, 0.15, -0.11];
	let (_, g) = nll_grad(&obs, &at);

	let h = 1e-6;
	for i in 0..N {
		let (mut lo, mut hi) = (at, at);
		lo[i] -= h;
		hi[i] += h;
		let central = (nll_grad(&obs, &hi).0 - nll_grad(&obs, &lo).0) / (2. * h);
		assert!((g[i] - central).abs() < 1e-6, "{}: analytic {} against finite {central}", NAMES[i], g[i]);
	}
}

/// Adam, censoring and the truncated sum together: the coefficients that generated the orderings
/// come back out of them, up to the scale Plackett–Luce cannot see.
#[test]
fn a_known_coefficient_set_comes_back() {
	let truth = [0.5, 0.3, 1.2, -0.4, -0.2];
	let obs = synthetic(truth);

	let (mut c, mut m, mut v) = ([0.; N], [0.; N], [0.; N]);
	for t in 1..=4000 {
		let (_, g) = nll_grad(&obs, &c);
		for i in 0..N {
			m[i] = 0.9 * m[i] + 0.1 * g[i];
			v[i] = 0.999 * v[i] + 0.001 * g[i] * g[i];
			c[i] -= 0.02 * (m[i] / (1. - 0.9f64.powi(t))) / ((v[i] / (1. - 0.999f64.powi(t))).sqrt() + 1e-8);
		}
	}
	// PL identifies the ratios, not the scale: the likelihood is unchanged by a common multiplier
	let k = c[NAME] / truth[NAME];
	assert!(k > 0., "the fit recovered the opposite sign: {c:?}");
	for i in 0..N {
		assert!((c[i] / k - truth[i]).abs() < 0.15, "{}: {} against {}, scale {k:.2}", NAMES[i], c[i] / k, truth[i]);
	}
}

/// An accent-stripping or containment change shows up here as a diff before it shows up as a map.
#[test]
fn feature_vectors_over_three_shops() {
	let t = trade();
	let f = feats();
	let node = [45.7797, 3.0863];
	let mut s = format!("{:<30} {}\n", "", NAMES.map(|n| format!("{n:>14}")).join(""));
	for name in ["Ecolavage Clermont", "American Car Wash - Clermont-Ferrand Aubière", "EDEO DETAILING"] {
		let q: &Poi = &t.pois.iter().find(|q| q.poi.name == name).unwrap().poi;
		let x = f.at(&Biz::from(q), "lavage auto", Some(node));
		s.push_str(&format!("{:<30} {}\n", name.chars().take(29).collect::<String>(), x.map(|v| format!("{v:>14.4}")).join("")));
	}
	insta::assert_snapshot!(s, @"
	                                log1p_reviews    rating_dev    name_match   place_token   distance_km
	Ecolavage Clermont                     5.4205        0.8156        0.5000        1.0000        1.5487
	American Car Wash - Clermont-          6.7604       -0.0116        0.0000        1.0000        5.0927
	EDEO DETAILING                         0.0000        0.0000        0.0000        0.0000        0.4096
	");
}

/// A rectangle has nobody standing in it, so its orderings carry no distance and can identify none.
#[test]
fn without_a_searcher_there_is_no_distance() {
	let q = &trade().pois[0].poi;
	assert_eq!(feats().at(&Biz::from(q), "lavage auto", None)[DIST], 0.);
	assert!(feats().at(&Biz::from(q), "lavage auto", Some([45.9, 3.3]))[DIST] > 0.);
}

/// More reviews may never make a shop count for less. The scoring path asserts this too; here it is
/// against the committed coefficients.
#[test]
fn strength_rises_with_reviews_and_with_the_name() {
	let rank = rank::Rank::try_new(feats(), rank::COEF).unwrap();
	let terms = [("lavage auto".to_owned(), 1.0)];
	let at = |name: &str, n_rev: f64| {
		rank.strength(
			&Biz {
				name,
				n_rev,
				rating: None,
				lat: 45.78,
				lng: 3.08,
			},
			&terms,
		)
	};
	assert!(at("Aquafix", 200.) > at("Aquafix", 20.), "reviews must not lower a competitor's weight");
	assert!(at("Lavage Auto Aquafix", 20.) > at("Aquafix", 20.), "matching the query must not lower it either");
}

/// Every stratum carries the same demand mass, so an unweighted average over nodes is already the
/// demand-weighted one. That the sampler cannot see the competitors needs no test: `nodes` does not
/// take them.
#[test]
fn the_split_hands_every_node_an_equal_share() {
	let p = payload();
	let at: Vec<[f64; 2]> = (0..p.place.len())
		.map(|i| {
			let r = &p.ring[i * 8..i * 8 + 8];
			[(r[1] + r[3] + r[5] + r[7]) / 4., (r[0] + r[2] + r[4] + r[6]) / 4.]
		})
		.collect();
	let n = 32;
	let out = nodes(&at, &trade().demand, n).unwrap();
	assert_eq!(out.len(), n);
	assert!((out.iter().map(|k| k.share).sum::<f64>() - 1.).abs() < 1e-9, "the strata do not partition the demand");

	let worst = out.iter().map(|k| (k.share * n as f64 - 1.).abs()).fold(0., f64::max);
	assert!(worst < 0.05, "a stratum carries {:.3} of an equal share", 1. + worst);

	let mut s = String::new();
	for (k, node) in out.iter().enumerate() {
		s.push_str(&format!("{:>2}  {:.5}, {:.5}   {:.4}\n", k + 1, node.at[0], node.at[1], node.share));
	}
	insta::assert_snapshot!(s, @"
	 1  45.62244, 3.05322   0.0309
	 2  45.69842, 3.07243   0.0313
	 3  45.74046, 3.05087   0.0311
	 4  45.74032, 3.10377   0.0311
	 5  45.76521, 3.04476   0.0309
	 6  45.76555, 3.07604   0.0310
	 7  45.76358, 3.09163   0.0312
	 8  45.76294, 3.11112   0.0320
	 9  45.61294, 3.18365   0.0311
	10  45.68717, 3.17263   0.0311
	11  45.55832, 3.27351   0.0312
	12  45.66418, 3.27610   0.0312
	13  45.72545, 3.17961   0.0309
	14  45.75218, 3.18345   0.0313
	15  45.74144, 3.21852   0.0313
	16  45.74151, 3.31407   0.0314
	17  45.78412, 3.04486   0.0310
	18  45.78322, 3.06996   0.0312
	19  45.77688, 3.08590   0.0307
	20  45.78932, 3.08647   0.0320
	21  45.83576, 3.02687   0.0311
	22  45.90897, 3.01567   0.0315
	23  45.83316, 3.08651   0.0314
	24  45.90598, 3.08546   0.0314
	25  45.78695, 3.11233   0.0309
	26  45.81549, 3.12516   0.0314
	27  45.85725, 3.12174   0.0313
	28  45.91005, 3.11696   0.0316
	29  45.79846, 3.21348   0.0313
	30  45.80184, 3.31580   0.0314
	31  45.89797, 3.20598   0.0312
	32  45.88573, 3.32755   0.0315
	");
}

#[test]
fn the_splitter_refuses_what_it_cannot_halve() {
	let at = [[45.0, 3.0], [45.1, 3.1], [45.2, 3.2]];
	assert!(nodes(&at, &[1., 1., 1.], 3).is_err(), "3 is not a power of two");
	assert!(nodes(&at, &[1., 1., 1.], 4).is_err(), "3 cells cannot carry 4 strata");
	assert!(nodes(&at, &[1., 0., 0.], 2).is_err(), "one cell with demand cannot carry 2 strata");
	assert!(nodes(&at, &[1., -1., 1.], 2).is_err(), "negative demand");
}
