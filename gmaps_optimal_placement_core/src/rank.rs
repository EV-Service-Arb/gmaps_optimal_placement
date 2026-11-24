//! How much one competitor counts, fitted rather than guessed.
//!
//! Google hands back an ordering. Plackett–Luce over those orderings, in Cox partial-likelihood
//! form, turns them into a strength per business — truncation after the page cap needs no extra
//! machinery, the sum simply stops. `gmaps_optimal_placement fit` runs the Adam loop offline and writes [`COEF`]
//! back here; everything below is the mechanism both it and the browser read.
//!
//! ```text
//!   score_i = COEF · x_i          strength_i = exp(score_i)
//!   nll     = − Σ_obs Σ_{k<K} [ score_(k) − logsumexp{ score_j : j not yet placed } ]
//! ```
//!
//! No intercept: PL is shift-invariant and an intercept random-walks under Adam.
use eyre::{Result, ensure};

use crate::payload::Poi;

/// `log1p(n_rev)`, shrunk rating deviation, name↔term containment, the name carries a place label,
/// node→business distance in km.
pub const N: usize = 5;
pub const NAMES: [&str; N] = ["log1p_reviews", "rating_dev", "name_match", "place_token", "distance_km"];
pub const REV: usize = 0;
pub const RATING: usize = 1;
pub const NAME: usize = 2;
pub const PLACE: usize = 3;
pub const DIST: usize = 4;

/// `gmaps_optimal_placement fit`, pooled over both Clermont pairings: 200 orderings, of which the 96
/// the probe asked from a node are the only ones carrying distance. Regenerate, do not edit.
pub const COEF: [f64; N] = [0.4536, 1.4674, 1.7023, -0.3212, -0.2838];
/// The catchment the fitted distance term implies: `exp(COEF[DIST]·d_km) = exp(−d/λ)`, the decay
/// `model::pressure` applies.
pub const LAMBDA_M: f64 = -1000. / COEF[DIST];
/// Only the top few slots are a decision; nobody's choice turns on rank 18 against 19. The
/// denominator still runs over the whole returned set — the losers are what identify the
/// coefficients.
pub const K: usize = 5;

const _: () = assert!(COEF[DIST] < 0., "a fit whose distance term is not negative is refused by `Fit::check`");
/// Reviews of prior mass in the rating shrinkage. A shop with this many reviews sits halfway
/// between its own rating and the area's.
const SHRINK: f64 = 20.;
const R: f64 = 6378137.;

/// What the model reads off a business. A prospective one has no id and no reviews, which is the
/// whole point of the what-if.
#[derive(Clone, Copy, Debug)]
pub struct Biz<'a> {
	pub name: &'a str,
	pub n_rev: f64,
	pub rating: Option<f64>,
	pub lat: f64,
	pub lng: f64,
}

impl<'a> From<&'a Poi> for Biz<'a> {
	fn from(p: &'a Poi) -> Self {
		Self {
			name: &p.name,
			n_rev: p.n_rev,
			rating: p.rating,
			lat: p.lat,
			lng: p.lng,
		}
	}
}

/// The study-wide constants a feature vector is read against. Carries no coefficients: the fit
/// needs features before it has any.
#[derive(Clone)]
pub struct Feats {
	/// Shrinkage target: the mean rating over the inventory.
	mu: f64,
	/// Administrative labels in the study area, folded. A shop named after the city is making a
	/// claim about relevance that Google reads.
	places: Vec<String>,
}

impl Feats {
	pub fn try_new(mu: f64, places: &[String]) -> Result<Self> {
		ensure!((1. ..=5.).contains(&mu), "mean rating {mu} is not on the 1..5 scale");
		let mut places: Vec<String> = places.iter().flat_map(|p| words(&fold(p))).filter(|w| w.len() >= 4).collect();
		places.sort();
		places.dedup();
		Ok(Self { mu, places })
	}

	/// `node` is where the searcher stood. A `locationRestriction` rectangle has nobody standing in
	/// it, so its orderings carry no distance and identify none.
	pub fn at(&self, b: &Biz, term: &str, node: Option<[f64; 2]>) -> [f64; N] {
		let folded = fold(b.name);
		let mut x = [0.; N];
		x[REV] = b.n_rev.ln_1p();
		// missing rating shrinks to a deviation of exactly zero, so no separate binary — it would be
		// collinear with this one at n_rev = 0
		x[RATING] = match b.rating {
			Some(r) => (b.n_rev * r + SHRINK * self.mu) / (b.n_rev + SHRINK) - self.mu,
			None => 0.,
		};
		x[NAME] = containment(&words(&folded).iter().map(|w| grams(w)).collect::<Vec<_>>(), term);
		x[PLACE] = f64::from(self.places.iter().any(|p| folded.contains(p.as_str())));
		x[DIST] = node.map_or(0., |n| km(n, [b.lat, b.lng]));
		x
	}
}

/// A fitted coefficient set, read against the features it scores.
pub struct Rank {
	pub coef: [f64; N],
	pub feats: Feats,
}

impl Rank {
	/// Rejects a coefficient set that says more reviews or a closer name match make a business
	/// weaker. That is a data problem, and it should fail where it is read rather than paint a map.
	/// Distance is not checked here because [`Rank::strength`] does not read it — the sign and the
	/// catchment it implies are the fit's own to answer for.
	pub fn try_new(feats: Feats, coef: [f64; N]) -> Result<Self> {
		ensure!(coef.iter().all(|c| c.is_finite()), "coefficients are not all finite: {coef:?}");
		ensure!(coef[REV] > 0., "fitted review coefficient is {}, so more reviews would rank a shop lower", coef[REV]);
		ensure!(coef[NAME] > 0., "fitted name coefficient is {}, so matching the query would rank a shop lower", coef[NAME]);
		Ok(Self { coef, feats })
	}

	/// Per business, over the study's terms: `Σ_t weight_t · exp(score_i(t))`, with the distance term
	/// left out — `model::pressure` supplies `exp(−d/λ)` itself, and applying it twice would count
	/// the same geometry against a competitor twice.
	pub fn strength(&self, b: &Biz, terms: &[(String, f64)]) -> f64 {
		self.contributions(b, terms).iter().sum()
	}

	/// [`Self::strength`]'s terms, one per search.
	pub fn contributions(&self, b: &Biz, terms: &[(String, f64)]) -> Vec<f64> {
		terms
			.iter()
			.map(|(t, w)| {
				let x = self.feats.at(b, t, None);
				w * (0..N).filter(|&i| i != DIST).map(|i| self.coef[i] * x[i]).sum::<f64>().exp()
			})
			.collect()
	}
}

/// Per tier, the median raw [`Rank::strength`] of its competitors (`tier[i]` is competitor `i`'s):
/// what a payload's `w` is that strength divided by. `w = 1` is "a competitor as strong as the thing
/// I am about to open" — `Model::capture_at` gives a site's own pull an implicit weight of 1.0.
/// Per tier, because `Model::pressure` already multiplies by the tier weight: a fitted strength that
/// has learned a wash ranks below a detailer would otherwise be discounted twice. `None` for a tier
/// nobody is in.
pub fn divisors(tier: &[usize], raw: &[f64], tiers: usize) -> Result<Vec<Option<f64>>> {
	assert_eq!(tier.len(), raw.len(), "one tier per competitor");
	(0..tiers)
		.map(|t| {
			let mut w: Vec<f64> = tier.iter().zip(raw).filter(|(x, _)| **x == t).map(|(_, r)| *r).collect();
			if w.is_empty() {
				return Ok(None);
			}
			w.sort_by(f64::total_cmp);
			let med = w[w.len().div_euclid(2)];
			ensure!(med > 0., "tier {t} has a median strength of {med}, so it cannot be normalised");
			Ok(Some(med))
		})
		.collect()
}

/// One observed ordering. The first `ranked` feature vectors are in the order Places returned them;
/// the rest are the remainder of the choice set, which is what identifies the coefficients.
#[derive(Clone, Debug)]
pub struct Obs {
	pub x: Vec<[f64; N]>,
	pub ranked: usize,
}

/// Negative log partial likelihood and its analytic gradient.
pub fn nll_grad(obs: &[Obs], coef: &[f64; N]) -> (f64, [f64; N]) {
	let (mut nll, mut grad) = (0., [0.; N]);
	let mut s = Vec::new();
	for o in obs {
		s.clear();
		s.extend(o.x.iter().map(|x| (0..N).map(|i| coef[i] * x[i]).sum::<f64>()));
		for k in 0..o.ranked.min(K) {
			let risk = &s[k..];
			let hi = risk.iter().copied().fold(f64::NEG_INFINITY, f64::max);
			let lse = hi + risk.iter().map(|v| (v - hi).exp()).sum::<f64>().ln();
			nll -= s[k] - lse;
			for (sj, x) in risk.iter().zip(&o.x[k..]) {
				let p = (sj - lse).exp();
				for (g, xi) in grad.iter_mut().zip(x) {
					*g += p * xi;
				}
			}
			for (g, xi) in grad.iter_mut().zip(&o.x[k]) {
				*g -= xi;
			}
		}
	}
	(nll, grad)
}

/// One demand stratum, reduced to where its query is asked from.
pub struct Node {
	/// [lat, lon] — the stratum's demand centroid, where a typical searcher in it stands.
	pub at: [f64; 2],
	/// Share of the study's demand. Equal by construction, which is what makes an unweighted average
	/// over nodes already the demand-weighted one.
	pub share: f64,
}

/// Equal-demand strata: split the extent at the demand-weighted median of its longer axis, recurse
/// to depth log2(`n`), and stand the node at each leaf's demand centroid. Steps stretch where
/// demand thins, so long node→business baselines survive and the distance coefficient stays
/// identified.
///
/// The sampler may look at demand and at nothing else. Demand is census, pre-treatment with respect
/// to how Google ranks; business locations are the regressor, so putting more nodes where the
/// competitors cluster would be selection on the very thing being estimated. No POI argument
/// reaches this function, and none may.
pub fn nodes(at: &[[f64; 2]], demand: &[f64], n: usize) -> Result<Vec<Node>> {
	ensure!(at.len() == demand.len(), "{} cells against {} demand values", at.len(), demand.len());
	ensure!(n.is_power_of_two(), "{n} strata is not a power of two");
	ensure!(at.len() >= n, "{} cells cannot carry {n} equal-demand strata", at.len());
	ensure!(demand.iter().all(|d| d.is_finite() && *d >= 0.), "demand carries a negative or non-finite value");
	let mut idx: Vec<usize> = (0..at.len()).filter(|&i| demand[i] > 0.).collect();
	ensure!(idx.len() >= n, "{} cells carry demand, which cannot carry {n} strata", idx.len());

	let lat0 = at.iter().map(|p| p[0]).sum::<f64>() / at.len() as f64;
	let total: f64 = idx.iter().map(|&i| demand[i]).sum();
	let mut out = Vec::with_capacity(n);
	Strata { at, demand, lat0, total }.split(&mut idx, n.trailing_zeros(), &mut out);
	ensure!(out.len() == n, "the split produced {} strata, not {n}", out.len());
	Ok(out)
}

/// What every split of [`nodes`] reads.
struct Strata<'a> {
	at: &'a [[f64; 2]],
	demand: &'a [f64],
	/// Where east-west is foreshortened.
	lat0: f64,
	total: f64,
}
impl Strata<'_> {
	fn split(&self, idx: &mut [usize], depth: u32, out: &mut Vec<Node>) {
		let Self { at, demand, lat0, total } = *self;
		if depth == 0 {
			let mass: f64 = idx.iter().map(|&i| demand[i]).sum();
			let mut c = [0., 0.];
			for &i in idx.iter() {
				c[0] += demand[i] * at[i][0];
				c[1] += demand[i] * at[i][1];
			}
			out.push(Node {
				at: [c[0] / mass, c[1] / mass],
				share: mass / total,
			});
			return;
		}
		let span = |k: usize| {
			let (lo, hi) = idx.iter().fold((f64::MAX, f64::MIN), |(lo, hi), &i| (lo.min(at[i][k]), hi.max(at[i][k])));
			hi - lo
		};
		let axis = usize::from(span(1) * lat0.to_radians().cos() > span(0));
		idx.sort_by(|&a, &b| at[a][axis].total_cmp(&at[b][axis]));

		let half: f64 = idx.iter().map(|&i| demand[i]).sum::<f64>() / 2.;
		let (mut acc, mut cut) = (0., 0);
		while cut < idx.len() - 1 && acc + demand[idx[cut]] <= half {
			acc += demand[idx[cut]];
			cut += 1;
		}
		// a leaf with no cell has no centroid, so each side keeps one cell per leaf below it
		let keep = 1usize << (depth - 1);
		let cut = cut.clamp(keep, idx.len() - keep);
		let (lo, hi) = idx.split_at_mut(cut);
		self.split(lo, depth - 1, out);
		self.split(hi, depth - 1, out);
	}
}

/// Flat-earth, over one agglomeration — the same approximation `model` projects with.
fn km(a: [f64; 2], b: [f64; 2]) -> f64 {
	let m_per_deg = std::f64::consts::PI / 180. * R;
	let dy = (b[0] - a[0]) * m_per_deg;
	let dx = (b[1] - a[1]) * m_per_deg * ((a[0] + b[0]) / 2.).to_radians().cos();
	dx.hypot(dy) / 1000.
}

/// Asymmetric `|A∩B|/|B|` over trigrams: per term word, the best-covering token of the name. Not
/// Dice — Dice penalises extra tokens, so "Plombier Clermont" would score worse than bare
/// "Plombier", backwards from the premise.
fn containment(tokens: &[Vec<String>], term: &str) -> f64 {
	let term = words(&fold(term));
	if term.is_empty() || tokens.is_empty() {
		return 0.;
	}
	let per_word = term.iter().map(|w| {
		let b = grams(w);
		tokens.iter().map(|a| b.iter().filter(|g| a.contains(g)).count() as f64 / b.len() as f64).fold(0., f64::max)
	});
	per_word.sum::<f64>() / term.len() as f64
}

fn grams(w: &str) -> Vec<String> {
	let c: Vec<char> = w.chars().collect();
	match c.len() < 3 {
		true => vec![w.to_owned()],
		false => c.windows(3).map(|g| g.iter().collect()).collect(),
	}
}

fn words(s: &str) -> Vec<String> {
	s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_owned).collect()
}

/// Lowercase and strip the accents a French shop name and a French query disagree about.
fn fold(s: &str) -> String {
	s.to_lowercase()
		.chars()
		.map(|c| match c {
			'à' | 'â' | 'ä' | 'á' | 'ã' | 'å' => 'a',
			'é' | 'è' | 'ê' | 'ë' => 'e',
			'î' | 'ï' | 'í' | 'ì' => 'i',
			'ô' | 'ö' | 'ó' | 'ò' | 'õ' => 'o',
			'ù' | 'û' | 'ü' | 'ú' => 'u',
			'ÿ' | 'ý' => 'y',
			'ç' => 'c',
			'ñ' => 'n',
			other => other,
		})
		.collect()
}
