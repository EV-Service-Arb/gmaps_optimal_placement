//! Estimating the ranking model from orderings already on disk.
//!
//! Adam over `gmaps_optimal_placement_core::rank`'s likelihood. Offline and slow-ish by design: it runs once,
//! its output is a table of five numbers, and those numbers are pasted into `rank::COEF` and
//! committed.
use eyre::{Result, ensure};
use gmaps_optimal_placement_core::rank::{self, DIST, Feats, K, N, NAMES, Obs};
use indexmap::IndexMap;

use crate::{Observed, Ordering};

const STEPS: usize = 2000;
const LR: f64 = 0.02;

pub struct Fit {
	pub coef: [f64; N],
	pub nll: f64,
	obs: Vec<Obs>,
	/// node→business over everything Places actually returned, km.
	reach: (f64, f64),
}
impl Fit {
	/// Each coefficient, and what the likelihood loses when that feature is refitted out.
	pub fn stats(&self) -> IndexMap<String, String> {
		let mut m = IndexMap::from([
			("orderings".to_owned(), self.obs.len().to_string()),
			(
				"choice_set".to_owned(),
				format!(
					"{}..{} places, top {K} scored",
					self.obs.iter().map(|o| o.x.len()).min().unwrap_or(0),
					self.obs.iter().map(|o| o.x.len()).max().unwrap_or(0)
				),
			),
			("nll".to_owned(), format!("{:.1}", self.nll)),
		]);
		for i in 0..N {
			let mut on = [true; N];
			on[i] = false;
			let (_, without) = adam(&self.obs, on);
			m.insert(NAMES[i].to_owned(), format!("{:+.4}   Δnll if dropped {:+.1}", self.coef[i], without - self.nll));
		}
		m.insert("implied_lambda_m".to_owned(), format!("{:.0} (linked: {:.0})", -1000. / self.coef[DIST], rank::LAMBDA_M));
		m.insert(
			"reach_km".to_owned(),
			match self.reach.0 <= self.reach.1 {
				true => format!("{:.2}..{:.2}", self.reach.0, self.reach.1),
				false => "none — no ordering was asked from a node".to_owned(),
			},
		);
		m.insert("top3_overlap".to_owned(), format!("{:.2}", self.top3()));
		m
	}

	/// A catchment the data never observed is extrapolation, and a coefficient that says distance
	/// helps is a data problem. Both are cheap to check and neither is recoverable.
	pub fn check(&self) -> Result<()> {
		ensure!(self.coef[DIST] < 0., "distance coefficient is {:+.4}: the fit says further away ranks higher", self.coef[DIST]);
		let lambda = -1. / self.coef[DIST];
		ensure!(
			(self.reach.0..=self.reach.1).contains(&lambda),
			"implied catchment {lambda:.2} km falls outside the {:.2}..{:.2} km the probe observed — nothing identifies it there",
			self.reach.0,
			self.reach.1
		);
		Ok(())
	}

	/// How often the model puts the same three businesses on top as Google did. In sample, like
	/// [`Fit::nll`] — held out is `crate::League`.
	fn top3(&self) -> f64 {
		let (mut sum, mut n) = (0., 0);
		for o in self.obs.iter().filter(|o| o.ranked >= 3) {
			let mut by_score: Vec<usize> = (0..o.x.len()).collect();
			let s = |j: usize| (0..N).map(|i| self.coef[i] * o.x[j][i]).sum::<f64>();
			by_score.sort_by(|&a, &b| s(b).total_cmp(&s(a)));
			sum += by_score[..3].iter().filter(|&&j| j < 3).count() as f64 / 3.;
			n += 1;
		}
		sum / n as f64
	}
}

/// How Google ranks is one mechanism, so the evidence pools: every study's orderings go into one
/// coefficient set, and each study's own term weights re-weight it afterwards. Features come from
/// each study's own context, because the shrinkage target and the place list are the study's.
impl<'a> FromIterator<&'a Observed> for Fit {
	fn from_iter<I: IntoIterator<Item = &'a Observed>>(it: I) -> Self {
		let (mut obs, mut reach) = (Vec::new(), (f64::MAX, 0f64));
		for o in it {
			obs.extend(self::obs(&o.feats, &o.orderings()));
			reach = (reach.0.min(o.reach.0), reach.1.max(o.reach.1));
		}
		let (coef, nll) = adam(&obs, [true; N]);
		Self { coef, nll, obs, reach }
	}
}

/// Choice sets, read through one feature context.
pub fn obs(ctx: &Feats, orderings: &[Ordering]) -> Vec<Obs> {
	orderings
		.iter()
		.map(|o| Obs {
			x: o.biz.iter().map(|b| ctx.at(b, o.term, o.node())).collect(),
			ranked: o.ranked,
		})
		.collect()
}

/// Full-batch Adam. Five parameters over a few hundred orderings — batching would buy noise, and
/// the whole run is seconds. `on` masks a feature out by leaving it at the zero init.
pub fn adam(obs: &[Obs], on: [bool; N]) -> ([f64; N], f64) {
	let (mut c, mut m, mut v) = ([0.; N], [0.; N], [0.; N]);
	let steps = match on.iter().any(|&b| b) {
		true => STEPS,
		false => 0,
	};
	for t in 1..=steps {
		let (_, g) = rank::nll_grad(obs, &c);
		for i in (0..N).filter(|&i| on[i]) {
			m[i] = 0.9 * m[i] + 0.1 * g[i];
			v[i] = 0.999 * v[i] + 0.001 * g[i] * g[i];
			let (mh, vh) = (m[i] / (1. - 0.9f64.powi(t as i32)), v[i] / (1. - 0.999f64.powi(t as i32)));
			c[i] -= LR * mh / (vh.sqrt() + 1e-8);
		}
	}
	let (nll, _) = rank::nll_grad(obs, &c);
	(c, nll)
}
