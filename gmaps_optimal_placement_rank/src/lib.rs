#![doc = include_str!("../README.md")]

pub mod fit;

use eyre::{Result, ensure};
use gmaps_optimal_placement_core::{
	payload::Poi,
	rank::{Biz, DIST, Feats, K, N, NAMES, REV},
};
use gmaps_optimal_placement_sources::{Ranking, Region};
use indexmap::IndexMap;

/// Five folds over ~200 orderings: a single 80/20 split leaves a test set too small to separate
/// entrants, and leave-one-out would refit once per ordering.
pub const FOLDS: usize = 5;
/// A business absent from a search is absent for four reasons and only one of them is "weak": past
/// the page cap (informative), outside the search area (a filter, not a ranking), irrelevant to the
/// term, or deduped. So the choice set is what the *same term* returned anywhere — asking why a
/// plumber did not appear in car-wash results collapses the fit to all-zero — intersected with the
/// area this search could reach. For a node that is `REACH` times the furthest result it did return,
/// which is self-calibrating and needs no magic radius; for a tile it is the tile.
const REACH: f64 = 1.5;

/// Google's log-preference for one business, on one term, asked from one place.
///
/// `exp(score)` is the Plackett–Luce weight, so the likelihood reads differences and the absolute
/// level is the strategy's own to calibrate. A strategy whose output is ordinal does not belong
/// here — it would win or lose on an arbitrary scale.
pub trait Strength: Send + Sync {
	fn score(&self, b: &Biz, term: &str, node: Option<[f64; 2]>) -> f64;
}

/// An entrant. Everything it learns, it learns in `fit` from the orderings it is handed.
///
/// `fit` returns a fresh [`Strength`] rather than mutating `self`: the harness fits the same entrant
/// once per fold, and fold independence should be structural rather than remembered.
pub trait Strategy {
	fn name(&self) -> &str;
	fn fit(&self, ctx: &Feats, train: &[Ordering]) -> Result<Box<dyn Strength>>;
}

/// One observed choice set, as observed. The first `ranked` are the order Places returned; the rest
/// are the remainder of the set, which is what identifies anything at all.
///
/// Raw businesses rather than feature vectors: a strategy that cannot choose its own features is a
/// coefficient set wearing a trait.
#[derive(Clone, Debug)]
pub struct Ordering<'a> {
	pub term: &'a str,
	/// Folds split on this, so no node's competitor set or geometry crosses a fold boundary.
	pub from: Region,
	pub biz: Vec<Biz<'a>>,
	pub ranked: usize,
}

impl Ordering<'_> {
	/// Where the searcher stood. `None` for a `locationRestriction` rectangle — nobody stands in one.
	pub fn node(&self) -> Option<[f64; 2]> {
		match self.from {
			Region::Tile(_) => None,
			Region::Node(at, _) => Some(at),
		}
	}

	/// Negative log partial likelihood of this one ordering under `s`.
	pub fn nll(&self, s: &dyn Strength) -> f64 {
		let score: Vec<f64> = self.biz.iter().map(|b| s.score(b, self.term, self.node())).collect();
		let mut nll = 0.;
		for k in 0..self.ranked.min(K) {
			let risk = &score[k..];
			let hi = risk.iter().copied().fold(f64::NEG_INFINITY, f64::max);
			nll += hi + risk.iter().map(|v| (v - hi).exp()).sum::<f64>().ln() - score[k];
		}
		nll
	}

	/// Share of the top three the model and Google agree on. `None` when Google returned fewer than
	/// three, which is nothing to agree about.
	pub fn top3(&self, s: &dyn Strength) -> Option<f64> {
		if self.ranked < 3 {
			return None;
		}
		let score: Vec<f64> = self.biz.iter().map(|b| s.score(b, self.term, self.node())).collect();
		let mut by_score: Vec<usize> = (0..score.len()).collect();
		// ties break on a fixed scramble of position rather than on position: a null model ties
		// everywhere, and a stable sort would hand it Google's own answer for nothing
		by_score.sort_by(|&a, &b| score[b].total_cmp(&score[a]).then(scramble(a).cmp(&scramble(b))));
		Some(by_score[..3].iter().filter(|&&j| j < 3).count() as f64 / 3.)
	}
}

/// A coefficient set over `core::rank`'s five features, with a mask saying which are estimated.
/// Masked-off coefficients stay at Adam's zero init, so the all-off entrant scores every competitor
/// the same — the premise this whole model replaces, made runnable so it can be beaten on the record.
pub struct Linear {
	name: String,
	on: [bool; N],
}

impl Linear {
	/// Every competitor counts the same. The null every other entrant is read against.
	pub fn flat() -> Self {
		Self {
			name: "FLAT".to_owned(),
			on: [false; N],
		}
	}

	/// Reviews and geometry only — prominence without relevance.
	pub fn reviews() -> Self {
		let mut on = [false; N];
		(on[REV], on[DIST]) = (true, true);
		Self { name: "REVIEWS".to_owned(), on }
	}

	/// What ships today.
	pub fn fitted() -> Self {
		Self {
			name: "FITTED".to_owned(),
			on: [true; N],
		}
	}

	/// `FITTED` with one feature refitted out — the held-out counterpart of `fit`'s in-sample
	/// "Δnll if dropped" column.
	pub fn without(i: usize) -> Self {
		let mut on = [true; N];
		on[i] = false;
		Self {
			name: format!("FITTED −{}", NAMES[i]),
			on,
		}
	}

	pub fn ladder() -> Vec<Self> {
		let mut out = vec![Self::flat(), Self::reviews(), Self::fitted()];
		out.extend((0..N).map(Self::without));
		out
	}
}

impl Strategy for Linear {
	fn name(&self) -> &str {
		&self.name
	}

	fn fit(&self, ctx: &Feats, train: &[Ordering]) -> Result<Box<dyn Strength>> {
		Ok(Box::new(Fitted {
			coef: fit::adam(&fit::obs(ctx, train), self.on).0,
			feats: ctx.clone(),
		}))
	}
}

/// Held out, and per ordering so folds of unequal size average honestly.
#[derive(Clone, Copy, Debug)]
pub struct Loss {
	/// Negative log partial likelihood per ordering. Primary: proper, and it reads the whole returned
	/// list rather than the top of it.
	pub nll: f64,
	/// Share of the top 3 the model and Google agree on. Scale-free, and the one a person argues with.
	pub top3: f64,
	pub orderings: usize,
}

/// K-fold, split by [`Region`]: every ordering asked from one node shares that node's competitor set
/// and its geometry, so splitting inside a node leaks the answer across the fold boundary. The fold
/// is the region's position in order of first appearance, mod [`FOLDS`] — deterministic, so two runs
/// of the table are comparable.
pub fn cv(ctx: &Feats, all: &[Ordering], entrant: &dyn Strategy) -> Result<Loss> {
	ensure!(!all.is_empty(), "no ordering to cross-validate over");
	let fold = folds(all);
	let (mut nll, mut agree, mut agreed_on) = (0., 0., 0usize);
	for f in 0..FOLDS {
		let train: Vec<Ordering> = all.iter().zip(&fold).filter(|(_, g)| **g != f).map(|(o, _)| o.clone()).collect();
		let test: Vec<&Ordering> = all.iter().zip(&fold).filter(|(_, g)| **g == f).map(|(o, _)| o).collect();
		ensure!(!train.is_empty(), "fold {f} of {} leaves nothing to fit on", entrant.name());
		if test.is_empty() {
			continue;
		}
		let s = entrant.fit(ctx, &train)?;
		for o in test {
			nll += o.nll(s.as_ref());
			if let Some(a) = o.top3(s.as_ref()) {
				agree += a;
				agreed_on += 1;
			}
		}
	}
	ensure!(agreed_on > 0, "no ordering carried three ranked businesses, so nothing agrees or disagrees");
	Ok(Loss {
		nll: nll / all.len() as f64,
		top3: agree / agreed_on as f64,
		orderings: all.len(),
	})
}

/// Every entrant on the ladder, over the same folds.
pub struct League(IndexMap<String, Loss>);
impl League {
	pub fn run(ctx: &Feats, all: &[Ordering]) -> Result<Self> {
		let mut out = IndexMap::new();
		for e in Linear::ladder() {
			out.insert(e.name().to_owned(), cv(ctx, all, &e)?);
		}
		Ok(Self(out))
	}

	pub fn stats(&self) -> IndexMap<String, String> {
		let flat = self.0["FLAT"];
		let mut m = IndexMap::from([
			("orderings".to_owned(), flat.orderings.to_string()),
			("folds".to_owned(), format!("{FOLDS}, split by the region the search was asked from")),
		]);
		for (name, l) in &self.0 {
			m.insert(name.clone(), format!("nll/ordering {:.3}   {:+.3} vs FLAT   top3 {:.2}", l.nll, l.nll - flat.nll, l.top3));
		}
		m
	}

	/// The map paints competitor weights this model produced. If counting every competitor the same
	/// predicts Google's own orderings at least as well, those weights are noise, and a table saying
	/// so in small print is a table that launders one.
	pub fn check(&self) -> Result<()> {
		let (flat, fitted) = (self.0["FLAT"], self.0["FITTED"]);
		ensure!(
			fitted.nll < flat.nll,
			"FITTED scores {:.3} held-out nll per ordering against FLAT's {:.3}: the fitted weights the map is painted with are noise",
			fitted.nll,
			flat.nll
		);
		Ok(())
	}
}

/// The orderings on disk, turned into choice sets. Every entrant reads the same ones: who *could*
/// have been returned is a property of the search, not of the model that scores it.
pub struct Observed {
	pub pois: Vec<Poi>,
	/// Administrative labels in the study's frame, for [`Feats`].
	pub places: Vec<String>,
	/// This study's own feature context. Pooling across studies is [`pooled`].
	pub feats: Feats,
	/// node→business over everything Places actually returned, km.
	pub reach: (f64, f64),
	sets: Vec<Set>,
}
impl Observed {
	pub fn try_new(pois: Vec<Poi>, places: Vec<String>, rankings: &[Ranking]) -> Result<Self> {
		let feats = feats(&places, &pois)?;
		let by_id: IndexMap<&str, usize> = pois.iter().enumerate().map(|(i, p)| (p.id.as_str(), i)).collect();
		// attributes join by id, so a result the inventory dropped cannot be scored and leaves the set
		let known = |ids: &[String]| -> Vec<usize> {
			let mut out: Vec<usize> = Vec::new();
			for id in ids {
				if let Some(&i) = by_id.get(id.as_str())
					&& !out.contains(&i)
				{
					out.push(i);
				}
			}
			out
		};

		let mut pool: IndexMap<&str, Vec<usize>> = IndexMap::new();
		for r in rankings {
			let seen = pool.entry(r.term.as_str()).or_default();
			for i in known(&r.ids) {
				if !seen.contains(&i) {
					seen.push(i);
				}
			}
		}

		let (mut sets, mut reach) = (Vec::new(), (f64::MAX, 0f64));
		for r in rankings {
			let seen = known(&r.ids);
			if seen.len() < 2 {
				continue;
			}
			let node = match r.from {
				Region::Tile(_) => None,
				Region::Node(at, _) => Some(at),
			};
			let km = |i: usize| feats.at(&Biz::from(&pois[i]), &r.term, node)[DIST];
			let far = seen.iter().map(|&i| km(i)).fold(0., f64::max);
			if node.is_some() {
				reach = (reach.0.min(seen.iter().map(|&i| km(i)).fold(f64::MAX, f64::min)), reach.1.max(far));
			}

			let mut idx = seen.clone();
			for &i in pool[r.term.as_str()].iter().filter(|i| !seen.contains(i)) {
				let reachable = match r.from {
					Region::Tile(b) => (b.lat[0]..=b.lat[1]).contains(&pois[i].lat) && (b.lon[0]..=b.lon[1]).contains(&pois[i].lng),
					Region::Node(..) => km(i) <= REACH * far,
				};
				if reachable {
					idx.push(i);
				}
			}
			sets.push(Set {
				term: r.term.clone(),
				from: r.from,
				idx,
				ranked: seen.len(),
			});
		}
		ensure!(!sets.is_empty(), "no ordering carried two businesses the inventory knows");
		Ok(Self { pois, places, feats, reach, sets })
	}

	pub fn orderings(&self) -> Vec<Ordering<'_>> {
		self.sets
			.iter()
			.map(|s| Ordering {
				term: &s.term,
				from: s.from,
				biz: s.idx.iter().map(|&i| Biz::from(&self.pois[i])).collect(),
				ranked: s.ranked,
			})
			.collect()
	}
}

/// One feature context over several studies. How Google ranks is one mechanism, so the evidence
/// pools; the shrinkage target and the place list have to pool with it.
pub fn pooled(studies: &[Observed]) -> Result<Feats> {
	let pois: Vec<&Poi> = studies.iter().flat_map(|o| o.pois.iter()).collect();
	let places: Vec<String> = studies.iter().flat_map(|o| o.places.iter().cloned()).collect();
	let rated: Vec<f64> = pois.iter().filter_map(|p| p.rating).collect();
	ensure!(!rated.is_empty(), "no competitor carries a rating, so there is nothing to shrink towards");
	Feats::try_new(rated.iter().sum::<f64>() / rated.len() as f64, &places)
}

struct Fitted {
	coef: [f64; N],
	feats: Feats,
}

impl Strength for Fitted {
	fn score(&self, b: &Biz, term: &str, node: Option<[f64; 2]>) -> f64 {
		let x = self.feats.at(b, term, node);
		(0..N).map(|i| self.coef[i] * x[i]).sum()
	}
}

/// One choice set, as positions into [`Observed::pois`] with the ranked ones first and in order.
struct Set {
	term: String,
	from: Region,
	idx: Vec<usize>,
	ranked: usize,
}

fn feats(places: &[String], pois: &[Poi]) -> Result<Feats> {
	let rated: Vec<f64> = pois.iter().filter_map(|p| p.rating).collect();
	ensure!(!rated.is_empty(), "no competitor carries a rating, so there is nothing to shrink towards");
	Feats::try_new(rated.iter().sum::<f64>() / rated.len() as f64, places)
}

fn folds(all: &[Ordering]) -> Vec<usize> {
	let mut seen: Vec<Region> = Vec::new();
	all.iter()
		.map(|o| {
			let g = seen.iter().position(|r| *r == o.from).unwrap_or_else(|| {
				seen.push(o.from);
				seen.len() - 1
			});
			g % FOLDS
		})
		.collect()
}

fn scramble(i: usize) -> u32 {
	(i as u32).wrapping_mul(2_654_435_761)
}
