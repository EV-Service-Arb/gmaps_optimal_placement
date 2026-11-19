#![feature(default_field_values)]
#![doc = include_str!("../README.md")]

pub mod config;
pub mod settings;

use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, bail, ensure};
pub use gmaps_optimal_placement_core as core;
use gmaps_optimal_placement_core::{
	Expr, LayerOut, Notice, PoiOut, Scale, TierOut,
	rank::{self, Biz, Feats, Rank},
};
pub use gmaps_optimal_placement_core::{Payload, payload};
/// Whole-country reconnaissance rather than one agglomeration: which towns are worth a study.
pub use gmaps_optimal_placement_misc as misc;
use gmaps_optimal_placement_rank::Observed;
pub use gmaps_optimal_placement_sources as sources;
use gmaps_optimal_placement_sources::{Frame, Work, grid, poi, probe};
use indexmap::IndexMap;
use sa_core::placement::{self, Area, Ask, Category, Definition};

pub use crate::config::Study;

const EMPTY: &str = "No competitor is on this map, and nothing that reads one is offered.";
/// Probe strata per study: a k-d split to depth log2 of it, one equal share of demand each.
const NODES: usize = 32;

/// The grid and everything derived from it. Everything up to, and not including, the billed part.
pub struct Cells {
	pub grid: gmaps_optimal_placement_core::Grid,
	/// `None` without a category that writes a demand.
	pub demand: Option<Vec<f64>>,
	pub layers: Vec<LayerOut>,
}

impl Study {
	/// Fails on the first expression that does not hold against the source's columns, before any
	/// POI call is made.
	pub fn cells(&self, work: &Work) -> Result<Cells> {
		let frame = Frame::from(&self.area);
		let mut grid = grid::load(self.grid.source, self.grid.vintage, frame.bbox, work)?;
		grid.retain(|c| {
			let (lon, lat) = c.ring.iter().fold((0., 0.), |(x, y), p| (x + p[0] / 4., y + p[1] / 4.));
			frame.contains(lat, lon)
		});
		ensure!(!grid.is_empty(), "no cell's centre falls inside {}", self.name);
		let n = grid.len();

		for c in &self.columns {
			ensure!(!grid.columns.contains_key(&c.name), "derived column {:?} is already published by the source", c.name);
			let values = Expr::parse(&c.expr)?.eval_column(&grid.columns, n).wrap_err_with(|| format!("derived column {:?}", c.name))?;
			grid.columns.insert(c.name.clone(), values);
		}

		let demand = self
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
					scale: match l.scale {
						placement::Scale::Percentile => Scale::Percentile,
						placement::Scale::Linear => Scale::Linear,
					},
					values: round(values, 3),
				})
			})
			.collect::<Result<Vec<_>>>()?;
		Ok(Cells { grid, demand, layers })
	}

	/// The ordering evidence this study contributes: the inventory sweep always, the probe wherever
	/// it has already been run and a demand places its nodes. Reads the work dir, never the network —
	/// so a pairing nobody has ever built costs nothing and is `None`.
	pub fn observations(&self, work: &Work) -> Result<Option<Observed>> {
		let c = self
			.category
			.as_ref()
			.ok_or_else(|| eyre::eyre!("{} is an area on its own, with no competitor to order", self.name))?;
		let cells = self.cells(work)?;
		let inv = poi::cached(c, Frame::from(&self.area), work)?;
		let mut obs = inv.obs;
		let swept = obs.len();
		if let Some(demand) = &cells.demand {
			obs.extend(probe::cached(work, &probe::plan(&at(&nodes(&cells, demand)?), &texts(&terms(c)), self.radius_m()))?);
		}
		eprintln!("{}: {swept} orderings from the inventory sweep, {} from the probe", self.name, obs.len() - swept);
		if obs.is_empty() {
			return Ok(None);
		}
		let places: Vec<String> = cells.grid.cells.iter().map(|c| c.place.clone()).collect();
		Observed::try_new(inv.pois, places, &obs).map(Some)
	}

	/// The `locationBias` circle: one stratum's worth of ground, as a circle. [`NODES`] cuts the
	/// frame into equal shares of demand, so this is how far a searcher standing at one of them is
	/// from its edge on average — which is the distance the coefficient is identified over.
	fn radius_m(&self) -> f64 {
		(Frame::from(&self.area).area_m2() / NODES as f64 / std::f64::consts::PI).sqrt()
	}

	/// The Places searches this study lacks, the first down each branch of its sweep; none for an
	/// area on its own.
	pub fn asks(&self, work: &Work) -> Result<Vec<Ask>> {
		let Some(c) = &self.category else { return Ok(Vec::new()) };
		Ok(poi::cached(c, Frame::from(&self.area), work)?.asks)
	}

	/// Out of what the work dir and the queries cache hold: a study whose sweep is not all there
	/// opens on what is, and its notice says how many searches it lacks at least.
	pub fn build(&self, work: &Work) -> Result<Payload> {
		let cells = self.cells(work)?;
		let n = cells.grid.len();
		let place: Vec<String> = cells.grid.cells.iter().map(|c| c.place.clone()).collect();
		for (i, c) in self.candidates.iter().enumerate() {
			// the pin file hides by name, so a duplicate would hide two pins at once
			ensure!(!self.candidates[..i].iter().any(|o| o.name == c.name), "two [[candidate]] are both named {:?}", c.name);
		}

		let mut ring = Vec::with_capacity(n * 8);
		for c in &cells.grid.cells {
			for p in c.ring {
				// 5 decimals is ~1 m: below the cell size, and a third of the file size of full f64
				ring.push(round1(p[0], 5));
				ring.push(round1(p[1], 5));
			}
		}
		let (trade, notice) = match &self.category {
			None => (None, None),
			Some(c) => {
				let inv = poi::cached(c, Frame::from(&self.area), work)?;
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
					_ => (Some(billed(c, &cells, &place, inv, self.radius_m(), work)?), notice),
				}
			}
		};
		let mut layers = Vec::new();
		if let Some(n) = &notice {
			eprintln!("{}: {}", self.name, n.text);
		}
		if trade.is_none()
			&& let Some(d) = self.category.as_ref().and_then(|c| c.demand.as_ref())
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
			name: self.name.clone(),
			center: Frame::from(&self.area).center,
			zoom: Frame::from(&self.area).zoom(),
			ring,
			imputed: cells.grid.cells.iter().map(|c| u8::from(c.imputed)).collect(),
			place,
			layers,
			candidates: self.candidates.clone(),
			trade,
			notice,
		})
	}
}

/// Every `*.nix` directly under `dir`, in file-stem order. A file is itself, so one axis of the
/// product can be named on the command line while the other stays a directory.
pub fn files(path: &Path) -> Result<Vec<PathBuf>> {
	if path.is_file() {
		return Ok(vec![path.to_owned()]);
	}
	let mut out: Vec<PathBuf> = std::fs::read_dir(path)
		.wrap_err_with(|| format!("reading {}", path.display()))?
		.map(|e| Ok(e?.path()))
		.collect::<Result<Vec<_>>>()?
		.into_iter()
		.filter(|p| p.extension().is_some_and(|e| e == "nix"))
		.collect();
	out.sort_by_key(|p| p.file_stem().expect("a *.nix path has a stem").to_owned());
	ensure!(!out.is_empty(), "no *.nix under {}", path.display());
	Ok(out)
}

/// The stem: a location's slug, a country base's country.
pub fn stem(p: &Path) -> &str {
	p.file_stem().and_then(|s| s.to_str()).expect("a study path is UTF-8 with a stem")
}

/// What an area's attrset is built from: a location file, which carries its own frame and premises,
/// or a country base, over which an area handed in at runtime has none.
#[derive(Clone, Copy, Debug)]
pub enum Base<'a> {
	Location(&'a Path),
	Country(&'a Path),
}

/// The area a location file draws: its outline, named by its stem as the conductor names an area by
/// its slug.
pub fn location_area(location: &Path) -> Result<Area> {
	let location = abs(location)?;
	let slug = stem(&location).to_lowercase();
	eval(&format!("let l = import {}; in l.area // {{ slug = \"{slug}\"; country = l.country; }}", location.display()))
}

/// A category applied to an area: the base's attrset with the area on top, then the category's
/// demand columns and layers after the base's. No category is the identity of that application: the
/// area's own attrset is already a study, and it is the one that costs nothing to build.
pub fn load(def: &Definition, base: Base) -> Result<Study> {
	def.check()?;
	let (base_expr, candidates) = match base {
		Base::Location(p) => (abs(p)?, ""),
		Base::Country(p) => (abs(p)?, " candidate = [];"),
	};
	// a Nix string of the JSON: `\`, `"` and the `$` of an interpolation escaped
	let area = serde_json::to_string(&def.area)?.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$");
	let mut study: Study = eval(&format!("(import {}) // {{ area = builtins.fromJSON \"{area}\";{candidates} }}", base_expr.display()))?;
	study.name = def.name();
	ensure!(
		study.country == def.area.country(),
		"{}: {} is a base for {}, not {}",
		study.name,
		base_expr.display(),
		study.country,
		def.area.country()
	);
	if study.layers.is_empty() {
		bail!("{} declares no [[layer]]", study.name);
	}
	if let Some(d) = def.category.as_ref().and_then(|c| c.demand.as_ref()) {
		study.columns.extend(d.columns.iter().cloned());
		study.layers.extend(d.layers.iter().cloned());
	}
	study.category = def.category.clone();
	Ok(study)
}

/// Summary statistics, so a model change that moves numbers is visible rather than silent.
pub fn stats(p: &Payload) -> IndexMap<String, String> {
	let sum = |v: &[f64]| v.iter().sum::<f64>();
	let mut m = IndexMap::from([
		("cells".to_owned(), p.place.len().to_string()),
		("imputed_cells".to_owned(), p.imputed.iter().filter(|&&i| i == 1).count().to_string()),
	]);
	if let Some(t) = &p.trade {
		let mut w: Vec<f64> = t.pois.iter().map(|q| q.w).collect();
		w.sort_by(f64::total_cmp);
		if let Some(d) = &t.demand {
			m.insert("demand_total".to_owned(), format!("{:.1}", sum(&d.values)));
		}
		m.insert("competitors".to_owned(), t.pois.len().to_string());
		m.insert(
			"competitor_w".to_owned(),
			format!("total {:.2}, median {:.2}, max {:.2}", sum(&w), w[w.len() / 2], w[w.len() - 1]),
		);
		for tier in &t.tiers {
			m.insert(format!("tier_{}", tier.name), t.pois.iter().filter(|q| q.poi.tier == tier.name).count().to_string());
		}
	}
	for l in &p.layers {
		m.insert(format!("layer_{}_total", l.name.to_lowercase().replace(' ', "_")), format!("{:.1}", sum(&l.values)));
	}
	m
}

/// The billed half, over the inventory held.
fn billed(c: &Category, cells: &Cells, place: &[String], inv: poi::Inventory, radius_m: f64, work: &Work) -> Result<payload::Trade> {
	let model = Rank::try_new(feats(place, &inv.pois)?, rank::COEF)?;
	let terms = terms(c);

	// a study nobody has probed carries no nodes, and the map offers no observed coverage layer;
	// neither does one with no demand to place them by
	let nodes = match &cells.demand {
		Some(d) => at(&nodes(cells, d)?),
		None => Vec::new(),
	};
	let probed = probe::cached(work, &probe::plan(&nodes, &texts(&terms), radius_m))?;
	let seen = gmaps_optimal_placement_rank::observed(&inv.pois, &nodes, &probed, &terms);
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
	queries.sort_by(|a, b| b.hits.cmp(&a.hits));

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
	normalise(&mut pois, &tiers)?;
	ensure!(
		pois.iter().any(|p| p.poi.tier == tiers[0].0),
		"no competitor came back from {:?}'s own searches, which is the scale every other tier is read against",
		tiers[0].0
	);
	Ok(payload::Trade {
		lambda_m: rank::LAMBDA_M,
		demand: c.demand.as_ref().map(|d| payload::Demand {
			note: d.expr.clone(),
			values: round(cells.demand.clone().expect("evaluated off the same category"), 3),
		}),
		tiers: tiers.into_iter().map(|(name, weight)| TierOut { name, weight }).collect(),
		terms: terms.into_iter().map(|(text, weight)| payload::TermOut { text, weight }).collect(),
		unweighted: c.keywords.iter().filter(|k| k.weight.is_none()).map(|k| k.text.clone()).collect(),
		queries,
		nodes,
		pois,
		searches: inv.calls,
	})
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
fn eval<T: serde::de::DeserializeOwned>(expr: &str) -> Result<T> {
	// `--impure` for the absolute paths: pure evaluation only admits a path it was handed as the root
	let out = std::process::Command::new("nix")
		.args(["eval", "--impure", "--json", "--expr", expr])
		.output()
		.wrap_err("running `nix eval` — a study is a Nix expression, so nix must be on PATH")?;
	ensure!(out.status.success(), "evaluating {expr}:\n{}", String::from_utf8_lossy(&out.stderr).trim());
	serde_json::from_slice(&out.stdout).wrap_err_with(|| format!("parsing {expr}"))
}

/// `nix eval --expr` resolves a bare path against the invocation directory, and the server builds
/// from whichever thread a request landed on.
fn abs(p: &Path) -> Result<PathBuf> {
	std::fs::canonicalize(p).wrap_err_with(|| format!("{} does not exist", p.display()))
}

fn at(nodes: &[rank::Node]) -> Vec<[f64; 2]> {
	nodes.iter().map(|n| n.at).collect()
}

fn feats(places: &[String], pois: &[payload::Poi]) -> Result<Feats> {
	let rated: Vec<f64> = pois.iter().filter_map(|p| p.rating).collect();
	ensure!(!rated.is_empty(), "no competitor carries a rating, so there is nothing to shrink towards");
	Feats::try_new(rated.iter().sum::<f64>() / rated.len() as f64, places)
}

/// `w = 1` is "a competitor as strong as the thing I am about to open" — `Model::capture_at` gives a
/// candidate's own pull an implicit weight of 1.0. Per tier, because `Model::pressure` already
/// multiplies by the tier weight: a fitted strength that has learned a wash ranks below a detailer
/// would otherwise be discounted twice, and the tier slider would stop being the only cross-tier
/// statement in the study.
fn normalise(pois: &mut [PoiOut], tiers: &[(String, f64)]) -> Result<()> {
	for (name, _) in tiers {
		let mut w: Vec<f64> = pois.iter().filter(|p| p.poi.tier == *name).map(|p| p.w).collect();
		if w.is_empty() {
			continue;
		}
		w.sort_by(f64::total_cmp);
		let med = w[w.len() / 2];
		ensure!(med > 0., "tier {name:?} has a median strength of {med}, so it cannot be normalised");
		for p in pois.iter_mut().filter(|p| p.poi.tier == *name) {
			p.w = round1(p.w / med, 4);
		}
	}
	Ok(())
}

fn round(v: Vec<f64>, places: i32) -> Vec<f64> {
	v.into_iter().map(|x| round1(x, places)).collect()
}

fn round1(x: f64, places: i32) -> f64 {
	let s = 10f64.powi(places);
	(x * s).round() / s
}
