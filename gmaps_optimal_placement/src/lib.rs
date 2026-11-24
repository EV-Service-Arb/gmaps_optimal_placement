#![doc = include_str!("../README.md")]

#[cfg(feature = "testing")]
pub mod testing;

use std::sync::atomic::{AtomicUsize, Ordering};

use eyre::{Result, WrapErr, bail, ensure};
use gmaps_optimal_placement_core::{
	Ask, Category, Column, Country, Definition, Expr, Layer, LayerOut, Model, Notice, Payload, Poi, PoiOut, Scale, TierOut,
	model::CUTOFF,
	payload,
	rank::{self, Biz, Feats, Rank},
};
use gmaps_optimal_placement_sources::{Frame, GridSource, Ranking, Region, grid, poi, probe};
pub use gmaps_optimal_placement_sources::{Held, Work};
use schemars::JsonSchema;
use serde::Deserialize;

const EMPTY: &str = "No competitor is on this map, and nothing that reads one is offered.";
/// Probe strata per study: a k-d split to depth log2 of it, one equal share of demand each.
const NODES: usize = 32;

/// One page of Places results. Past it the probe saw nothing, which is the same thing as a business
/// it never returned, so the observed field zeroes there rather than going missing.
const CAP: usize = 20;

/// A country's base: the grid, and the columns and layers every trade there shares. Compiled in per
/// [`Country`], from `bases/`; a base may not name a trade (docs/ARCHITECTURE.md, Invariants).
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Base {
	pub country: Country,
	pub grid: GridSpec,
	/// Columns the source does not publish, evaluated in order over the ones it does. Later entries
	/// may read earlier ones, so this is a list.
	#[serde(rename = "column")]
	pub columns: Vec<Column>,
	#[serde(rename = "layer")]
	pub layers: Vec<Layer>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GridSpec {
	pub source: GridSource,
	pub vintage: u16,
}

/// A definition over its country's base, ready to build. No category is the identity of the
/// application: the area's base alone is already a study, and the one that costs nothing to build.
pub struct Study {
	def: Definition,
	/// The definition's: `<trade>·<CC>·<slug>`.
	name: String,
	grid: GridSpec,
	/// The base's, then the category's demand columns.
	columns: Vec<Column>,
	/// The base's heat layers, then the category's.
	layers: Vec<Layer>,
}
impl Study {
	/// Fails on the first expression that does not hold against the source's columns, before any
	/// POI call is made.
	fn cells(&self, work: &Work) -> Result<Cells> {
		let frame = Frame::from(&self.def.area);
		let mut grid = grid::load(self.grid.source, self.grid.vintage, frame.bbox, work)?;
		grid.retain(|c| {
			let (lon, lat) = c.ring.iter().fold((0., 0.), |(x, y), p| (x + p[0] / 4., y + p[1] / 4.));
			frame.contains([lat, lon])
		});
		ensure!(!grid.is_empty(), "no cell's centre falls inside {}", self.name);
		let n = grid.len();

		for c in &self.columns {
			ensure!(!grid.columns.contains_key(&c.name), "derived column {:?} is already published by the source", c.name);
			let values = Expr::parse(&c.expr)?.eval_column(&grid.columns, n).wrap_err_with(|| format!("derived column {:?}", c.name))?;
			grid.columns.insert(c.name.clone(), values);
		}

		let demand = self
			.def
			.category
			.as_ref()
			.and_then(|c| c.demand.as_ref())
			.map(|d| Expr::parse(&d.expr)?.eval_column(&grid.columns, n).wrap_err("the category's demand"))
			.transpose()?;
		let layers = self
			.layers
			.iter()
			.map(|l| {
				let values = Expr::parse(&l.expr)?.eval_column(&grid.columns, n).wrap_err_with(|| format!("layer {:?}", l.name))?;
				Ok(LayerOut {
					name: l.name.clone(),
					note: l.note.clone(),
					scale: l.scale,
					values: round(values, 3),
				})
			})
			.collect::<Result<Vec<_>>>()?;
		Ok(Cells { grid, demand, layers })
	}

	/// The ordering evidence this study contributes: the inventory sweep always, the probe wherever
	/// it has already been run and a demand places its nodes. Out of what is held, so a pairing
	/// nobody has ever pulled costs nothing and is `None`.
	pub fn observations(&self, work: &Work) -> Result<Option<Evidence>> {
		let c = self
			.def
			.category
			.as_ref()
			.ok_or_else(|| eyre::eyre!("{} is an area on its own, with no competitor to order", self.name))?;
		let cells = self.cells(work)?;
		let inv = poi::cached(c, Frame::from(&self.def.area), work)?;
		let mut obs = inv.obs;
		let swept = obs.len();
		if let Some(demand) = &cells.demand {
			obs.extend(probe::cached(work, &probe::plan(&at(&nodes(&cells, demand)?), &texts(&terms(c)), self.radius_m()))?);
		}
		eprintln!("{}: {swept} orderings from the inventory sweep, {} from the probe", self.name, obs.len() - swept);
		if obs.is_empty() {
			return Ok(None);
		}
		Ok(Some(Evidence {
			pois: inv.pois,
			places: cells.grid.cells.iter().map(|c| c.place.clone()).collect(),
			rankings: obs,
		}))
	}

	/// The `locationBias` circle: one stratum's worth of ground, as a circle. [`NODES`] cuts the
	/// frame into equal shares of demand, so this is how far a searcher standing at one of them is
	/// from its edge on average — which is the distance the coefficient is identified over.
	fn radius_m(&self) -> f64 {
		(Frame::from(&self.def.area).area_m2() / NODES as f64 / std::f64::consts::PI).sqrt()
	}

	/// The Places searches this study lacks, the first down each branch of its sweep; none for an
	/// area on its own. What lies behind one (the next page, a tile's quarters) is known only once it
	/// is answered, so a pull asks again until none is lacking.
	pub fn asks(&self, work: &Work) -> Result<Vec<Ask>> {
		self.keep(work)?;
		let Some(c) = &self.def.category else { return Ok(Vec::new()) };
		Ok(poi::cached(c, Frame::from(&self.def.area), work)?.asks)
	}

	/// Out of what is held: a study whose sweep is not all there opens on what is, and its notice
	/// says how many searches it lacks at least.
	pub fn build(&self, work: &Work) -> Result<Payload> {
		self.keep(work)?;
		let cells = self.cells(work)?;
		let n = cells.grid.len();
		let mut ring = Vec::with_capacity(n * 8);
		for c in &cells.grid.cells {
			for p in c.ring {
				// 5 decimals is ~1 m: below the cell size, and a third of the file size of full f64
				ring.push(round1(p[0], 5));
				ring.push(round1(p[1], 5));
			}
		}
		let grid = Model::try_new(Payload {
			name: self.name.clone(),
			center: Frame::from(&self.def.area).center,
			zoom: Frame::from(&self.def.area).zoom(),
			ring,
			place: cells.grid.cells.iter().map(|c| c.place.clone()).collect(),
			imputed: cells.grid.cells.iter().map(|c| u8::from(c.imputed)).collect(),
			layers: Vec::new(),
			trade: None,
			notice: None,
		})?;
		let (trade, notice) = match &self.def.category {
			None => (None, None),
			Some(c) => {
				let inv = poi::cached(c, Frame::from(&self.def.area), work)?;
				let notice = match inv.asks.len() {
					0 => None,
					missing => Some(Notice {
						text: match inv.pois.len() {
							0 => format!("At least {missing} Places searches this study is built from are not held, and a pull buys them. {EMPTY}"),
							n => format!(
								"At least {missing} Places searches this study is built from are not held, and a pull buys them. The {n} competitors on this map are the part of the sweep that is."
							),
						},
						missing: Some(missing),
					}),
				};
				match (notice.is_some(), inv.pois.is_empty()) {
					(true, true) => (None, notice),
					_ => (Some(billed(c, &cells, &grid, inv, self.radius_m(), work)?), notice),
				}
			}
		};
		let mut layers = Vec::new();
		if let Some(n) = &notice {
			eprintln!("{}: {}", self.name, n.text);
		}
		if trade.is_none()
			&& let Some(d) = self.def.category.as_ref().and_then(|c| c.demand.as_ref())
		{
			// the demand surface is the category's expression over the archive and was evaluated before
			// the first call, so it is the one thing a refusal does not take away. First, so the map opens
			// on it rather than on whatever the base happened to list
			layers.push(LayerOut {
				name: "Demand".to_owned(),
				note: d.expr.clone(),
				scale: Scale::Percentile,
				values: round(cells.demand.clone().expect("evaluated off the same category"), 3),
			});
		}
		layers.extend(cells.layers);
		Ok(Payload {
			layers,
			trade,
			notice,
			..grid.payload
		})
	}

	/// The definition, under the work dir's `studies/<id>.json`: what `fit` harvests, whichever host
	/// built it. Renamed into place, so a reader never sees half a definition.
	fn keep(&self, work: &Work) -> Result<()> {
		static PARTS: AtomicUsize = AtomicUsize::new(0);
		let dir = work.path().join("studies");
		std::fs::create_dir_all(&dir).wrap_err_with(|| format!("creating {}", dir.display()))?;
		let sha = self.def.id();
		let (path, part) = (
			dir.join(format!("{sha}.json")),
			dir.join(format!("{sha}.{}.{}.part", std::process::id(), PARTS.fetch_add(1, Ordering::Relaxed))),
		);
		std::fs::write(&part, serde_json::to_vec(&self.def.clone().canonical())?).wrap_err_with(|| format!("writing {}", part.display()))?;
		std::fs::rename(&part, &path).wrap_err_with(|| format!("keeping {}", path.display()))
	}
}

/// What a study collected of how Google orders its competitors, for a fit to read.
pub struct Evidence {
	pub pois: Vec<Poi>,
	/// Administrative labels in the study's frame.
	pub places: Vec<String>,
	pub rankings: Vec<Ranking>,
}

/// A definition over its country's base, the area on top, then the category's demand columns and
/// layers after the base's.
pub fn load(def: &Definition) -> Result<Study> {
	def.check()?;
	let country = def.area.country();
	let base: Base = toml::from_str(match country {
		Country::FR => include_str!("../bases/FR.toml"),
	})
	.wrap_err_with(|| format!("the compiled-in base for {country}"))?;
	assert_eq!(base.country, country, "bases/{country}.toml is the base for {}", base.country);
	let name = def.name();
	if base.layers.is_empty() {
		bail!("{name}: the base for {country} declares no [[layer]]");
	}
	let (mut columns, mut layers) = (base.columns, base.layers);
	if let Some(d) = def.category.as_ref().and_then(|c| c.demand.as_ref()) {
		columns.extend(d.columns.iter().cloned());
		layers.extend(d.layers.iter().cloned());
	}
	Ok(Study {
		def: def.clone(),
		name,
		grid: base.grid,
		columns,
		layers,
	})
}

/// The grid and everything derived from it. Everything up to, and not including, the billed part.
struct Cells {
	grid: gmaps_optimal_placement_core::Grid,
	/// `None` without a category that writes a demand.
	demand: Option<Vec<f64>>,
	layers: Vec<LayerOut>,
}

/// The billed half, over the inventory held. `grid` is the study with no trade on it yet.
fn billed(c: &Category, cells: &Cells, grid: &Model, inv: poi::Inventory, radius_m: f64, work: &Work) -> Result<payload::Trade> {
	let model = Rank::try_new(feats(&grid.payload.place, &inv.pois)?, rank::COEF)?;
	let terms = terms(c);

	// a study nobody has probed carries no nodes, and the map offers no observed coverage layer;
	// neither does one with no demand to place them by
	let nodes = match &cells.demand {
		Some(d) => at(&nodes(cells, d)?),
		None => Vec::new(),
	};
	let probed = probe::cached(work, &probe::plan(&nodes, &texts(&terms), radius_m))?;
	let seen = observed(&inv.pois, &nodes, &probed, &terms);
	let nodes = match probed.is_empty() {
		true => Vec::new(),
		false => nodes,
	};

	let kept: std::collections::HashSet<&str> = inv.pois.iter().map(|p| p.id.as_str()).collect();
	let mut queries: Vec<payload::QueryOut> = poi::queries(c)?
		.into_iter()
		.map(|(q, _)| {
			let hit: std::collections::HashSet<&str> = inv
				.obs
				.iter()
				.filter(|o| o.term == q)
				.flat_map(|o| o.ids.iter().map(String::as_str))
				.filter(|id| kept.contains(id))
				.collect();
			payload::QueryOut { text: q, hits: hit.len() }
		})
		.collect();
	queries.sort_by_key(|q| std::cmp::Reverse(q.hits));

	let mut pois: Vec<PoiOut> = inv
		.pois
		.into_iter()
		.zip(seen)
		.map(|(p, seen)| {
			let w = model.strength(&Biz::from(&p), &terms);
			PoiOut {
				poi: p,
				w,
				seen: match nodes.is_empty() {
					true => Vec::new(),
					false => seen,
				},
			}
		})
		.collect();
	let tiers = poi::tiers(c)?;
	let tier: Vec<usize> = pois
		.iter()
		.map(|p| tiers.iter().position(|(n, _)| *n == p.poi.tier).expect("the sweep files every competitor under one of its tiers"))
		.collect();
	let divisors = rank::divisors(&tier, &pois.iter().map(|p| p.w).collect::<Vec<_>>(), tiers.len())?;
	ensure!(
		divisors[0].is_some(),
		"no competitor came back from {:?}'s own searches, which is the scale every other tier is read against",
		tiers[0].0
	);
	for (p, t) in pois.iter_mut().zip(tier) {
		p.w = round1(p.w / divisors[t].expect("a tier a competitor is in has a divisor"), 4);
	}
	let tiers: Vec<TierOut> = tiers.into_iter().map(|(name, weight)| TierOut { name, weight }).collect();
	Ok(payload::Trade {
		lambda_m: rank::LAMBDA_M,
		pressure: pressure(grid, &pois, &tiers, rank::LAMBDA_M),
		demand: c.demand.as_ref().map(|d| payload::Demand {
			note: d.expr.clone(),
			values: round(cells.demand.clone().expect("evaluated off the same category"), 3),
		}),
		tiers,
		terms: terms.into_iter().map(|(text, weight)| payload::TermOut { text, weight }).collect(),
		unweighted: c.keywords.iter().filter(|k| k.weight.is_none()).map(|k| k.text.clone()).collect(),
		queries,
		nodes,
		pois,
		searches: inv.calls,
	})
}

/// Competitor pressure, one per cell: Σ over the competitors within [`CUTOFF`] λ of weight × its
/// tier's weight × exp(−distance/λ).
fn pressure(grid: &Model, pois: &[PoiOut], tiers: &[TierOut], lambda_m: f64) -> Vec<f64> {
	let comps: Vec<([f64; 2], f64)> = pois
		.iter()
		.map(|p| {
			let tier = tiers.iter().find(|t| t.name == p.poi.tier).expect("the sweep files every competitor under one of its tiers");
			(grid.to_local([p.poi.lat, p.poi.lng]), p.w * tier.weight)
		})
		.collect();
	grid.c_lat
		.iter()
		.zip(&grid.c_lng)
		.map(|(&lat, &lng)| {
			let [x, y] = grid.to_local([lat, lng]);
			comps
				.iter()
				.map(|([cx, cy], w)| (w, (x - cx).hypot(y - cy)))
				.filter(|(_, d)| *d <= CUTOFF * lambda_m)
				.map(|(w, d)| w * (-d / lambda_m).exp())
				.sum()
		})
		.collect()
}

/// The category's keywords that weigh something: what a competitor is scored on and the probe asks.
fn terms(c: &Category) -> Vec<(String, f64)> {
	c.keywords.iter().filter_map(|k| Some((k.text.clone(), k.weight?))).collect()
}

fn texts(terms: &[(String, f64)]) -> Vec<String> {
	terms.iter().map(|(t, _)| t.clone()).collect()
}

/// Equal-demand strata over the cells, where the probe stands.
fn nodes(cells: &Cells, demand: &[f64]) -> Result<Vec<rank::Node>> {
	let at: Vec<[f64; 2]> = cells
		.grid
		.cells
		.iter()
		.map(|c| {
			let (lon, lat) = c.ring.iter().fold((0., 0.), |(x, y), p| (x + p[0] / 4., y + p[1] / 4.));
			[lat, lon]
		})
		.collect();
	rank::nodes(&at, demand, NODES)
}
fn at(nodes: &[rank::Node]) -> Vec<[f64; 2]> {
	nodes.iter().map(|n| n.at).collect()
}

fn feats(places: &[String], pois: &[payload::Poi]) -> Result<Feats> {
	let rated: Vec<f64> = pois.iter().filter_map(|p| p.rating).collect();
	ensure!(!rated.is_empty(), "no competitor carries a rating, so there is nothing to shrink towards");
	Feats::try_new(rated.iter().sum::<f64>() / rated.len() as f64, places)
}

fn round(v: Vec<f64>, places: i32) -> Vec<f64> {
	v.into_iter().map(|x| round1(x, places)).collect()
}

fn round1(x: f64, places: i32) -> f64 {
	let s = 10f64.powi(places);
	(x * s).round() / s
}

/// What the probe saw of every business, node by node: per business, one value per node, term
/// weights applied. Pure data — the probe asked each term from each node and Google answered, and
/// this is that answer with nothing added.
///
/// A business Google put at the top of the page counts for one and one it put off the page counts
/// for nothing, linearly between. That is a reading of position, not an estimate of anything: what
/// the field is for is the *shape* of a competitor's reach, which is lumpy in a way no radial decay
/// can be.
fn observed(pois: &[Poi], nodes: &[[f64; 2]], rankings: &[Ranking], terms: &[(String, f64)]) -> Vec<Vec<f64>> {
	let mut out = vec![vec![0.; nodes.len()]; pois.len()];
	let total: f64 = terms.iter().map(|(_, w)| w).sum();
	if total <= 0. {
		return out;
	}
	let by_id: std::collections::HashMap<&str, usize> = pois.iter().enumerate().map(|(i, p)| (p.id.as_str(), i)).collect();
	for r in rankings {
		let Region::Node(at, _) = r.from else { continue };
		let (Some(n), Some((_, w))) = (nodes.iter().position(|o| *o == at), terms.iter().find(|(t, _)| *t == r.term)) else {
			continue;
		};
		for (rank, id) in r.ids.iter().enumerate().take(CAP) {
			if let Some(&i) = by_id.get(id.as_str()) {
				out[i][n] += w / total * (1. - rank as f64 / CAP as f64);
			}
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The payload `_core`'s tests read: one whole Clermont study, frozen.
	fn clermont() -> (Model, payload::Trade) {
		let mut p: Payload = serde_json::from_str(include_str!("../../gmaps_optimal_placement_core/tests/it/clermont_payload.json")).unwrap();
		let trade = p.trade.take().expect("the Clermont fixture is a trade over a location");
		(Model::try_new(p).unwrap(), trade)
	}

	#[test]
	fn pressure_is_what_the_frozen_study_carries() {
		let (grid, t) = clermont();
		let computed = pressure(&grid, &t.pois, &t.tiers, t.lambda_m);
		assert_eq!(computed.len(), t.pressure.len());
		let worst = computed.iter().zip(&t.pressure).map(|(a, b)| (a - b).abs()).fold(0., f64::max);
		assert!(worst < 1e-12, "a cell's pressure is {worst:e} off what the fixture carries");
	}

	/// Dropping a tier lowers pressure, a wider catchment and a heavier tier raise it, and it is a
	/// function of its inputs alone.
	#[test]
	fn pressure_moves_with_what_is_on_the_ground() {
		let (grid, t) = clermont();
		let sum = |pois: &[PoiOut], tiers: &[TierOut], lambda_m: f64| pressure(&grid, pois, tiers, lambda_m).iter().sum::<f64>();
		let no_wash: Vec<PoiOut> = t.pois.iter().filter(|p| p.poi.tier != "wash").cloned().collect();
		let mut heavy = t.tiers.clone();
		heavy[0].weight *= 2.;

		let base = sum(&t.pois, &t.tiers, t.lambda_m);
		let (no_wash, wide, heavy) = (sum(&no_wash, &t.tiers, t.lambda_m), sum(&t.pois, &t.tiers, 4000.), sum(&t.pois, &heavy, t.lambda_m));
		assert!(no_wash < base, "dropping washes must lower pressure");
		assert!(wide > base, "wider λ must raise pressure");
		assert!(heavy > base, "a heavier tier must raise pressure");
		assert_eq!(base, sum(&t.pois, &t.tiers, t.lambda_m), "it is deterministic");
		insta::assert_snapshot!(format!("base {base:.6}\nno wash {no_wash:.6}\nlambda 4000 {wide:.6}\nfirst tier doubled {heavy:.6}"), @"
		base 27340.837444
		no wash 12574.450215
		lambda 4000 87845.339268
		first tier doubled 39915.287659
		");
	}
}
