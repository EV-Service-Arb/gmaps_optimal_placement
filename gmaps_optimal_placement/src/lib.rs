#![feature(default_field_values)]
#![doc = include_str!("../README.md")]

pub mod config;
pub mod render;
pub mod settings;

use std::path::{Path, PathBuf};

use eyre::{Result, WrapErr, bail, ensure};
pub use gmaps_optimal_placement_core as core;
use gmaps_optimal_placement_core::{
	Expr, LayerOut, PoiOut, Scale, TierOut,
	rank::{self, Biz, Feats, Rank},
};
pub use gmaps_optimal_placement_core::{Payload, payload};
/// Whole-country reconnaissance rather than one agglomeration: which towns are worth a study.
pub use gmaps_optimal_placement_misc as misc;
use gmaps_optimal_placement_rank::Observed;
pub use gmaps_optimal_placement_sources as sources;
use gmaps_optimal_placement_sources::{Keyword, Ranking, Work, grid, poi, probe, searches};
use indexmap::IndexMap;
use regex::Regex;
use serde::Serialize;

use crate::config::Group;
pub use crate::config::Study;

/// One chart's worth of demand-side numbers. Members ride along with their own series, because a
/// sum you cannot audit is a sum you cannot argue with.
#[derive(Debug, Serialize)]
pub struct SearchPayload {
	pub name: String,
	pub place: String,
	/// `YYYY-MM`, shared by every group.
	pub months: Vec<String>,
	pub groups: Vec<GroupOut>,
}

#[derive(Debug, Serialize)]
pub struct GroupOut {
	pub name: String,
	/// Per month, summed over `members`.
	pub total: Vec<u64>,
	pub members: Vec<MemberOut>,
	/// Kept by the filter, but the provider has no volume for them. Never zero-filled into `total`.
	pub no_data: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct MemberOut {
	pub text: String,
	pub avg: f64,
	pub monthly: Vec<u64>,
}

/// The grid and everything derived from it. Everything up to, and not including, the billed part.
pub struct Cells {
	pub grid: gmaps_optimal_placement_core::Grid,
	/// `None` for a location opened without a trade: a demand expression is a trade's to write.
	pub demand: Option<Vec<f64>>,
	pub layers: Vec<LayerOut>,
}

impl Study {
	/// Fails on the first expression that does not hold against the source's columns, before any
	/// POI call is made.
	pub fn cells(&self, work: &Work) -> Result<Cells> {
		let mut grid = grid::load(self.grid.source, self.grid.vintage, self.area.bbox, work)?;
		let n = grid.len();

		for c in &self.columns {
			ensure!(!grid.columns.contains_key(&c.name), "derived column {:?} is already published by the source", c.name);
			let values = Expr::parse(&c.expr)?.eval_column(&grid.columns, n).wrap_err_with(|| format!("derived column {:?}", c.name))?;
			grid.columns.insert(c.name.clone(), values);
		}

		let demand = self
			.model
			.as_ref()
			.map(|m| Expr::parse(&m.demand)?.eval_column(&grid.columns, n).wrap_err("[model] demand"))
			.transpose()?;
		let layers = self
			.layers
			.iter()
			.map(|l| {
				let values = Expr::parse(&l.expr)?.eval_column(&grid.columns, n).wrap_err_with(|| format!("layer {:?}", l.name))?;
				Ok(LayerOut {
					name: l.name.clone(),
					note: l.note.clone().unwrap_or_else(|| l.expr.clone()),
					scale: l.scale,
					values: round(values, 3),
				})
			})
			.collect::<Result<Vec<_>>>()?;
		Ok(Cells { grid, demand, layers })
	}

	/// What a trade decides, or the error every subcommand but `serve` owes a location that was
	/// opened without one. The three travel together: a trade file writes all of them.
	fn trade(&self) -> Result<(&gmaps_optimal_placement_sources::PoiConfig, &config::Model, &config::RankSpec)> {
		let missing = |what: &str| eyre::eyre!("{} declares no `{what}` — a location on its own carries the grid and nothing a trade decides", self.name);
		Ok((
			self.poi.as_ref().ok_or_else(|| missing("poi"))?,
			self.model.as_ref().ok_or_else(|| missing("model"))?,
			self.rank.as_ref().ok_or_else(|| missing("rank"))?,
		))
	}

	/// The demand a trade's model puts over this grid.
	fn demand<'a>(&self, cells: &'a Cells) -> Result<&'a [f64]> {
		self.trade()?;
		Ok(cells.demand.as_deref().expect("a study with a model evaluates one"))
	}

	/// The ordering evidence this study contributes: the inventory sweep always, the probe wherever
	/// it has already been run. Reads the work dir, never the network — so a pairing nobody has ever
	/// built costs nothing and is `None`.
	pub fn observations(&self, work: &Work) -> Result<Option<Observed>> {
		let cells = self.cells(work)?;
		let (poi, _, _) = self.trade()?;
		let inv = poi::cached(poi, self.area.bbox, work)?;
		let mut obs = inv.obs;
		let probed = probe::cached(work, &probe::plan(&at(&self.nodes(&cells)?), &self.terms()?, self.radius_m()?))?;
		eprintln!("{}: {} orderings from the inventory sweep, {} from the probe", self.name, obs.len(), probed.len());
		obs.extend(probed);
		if obs.is_empty() {
			return Ok(None);
		}
		let places: Vec<String> = cells.grid.cells.iter().map(|c| c.place.clone()).collect();
		Observed::new(inv.pois, places, &obs).map(Some)
	}

	pub fn probe(&self, work: &Work, dry: bool) -> Result<Vec<Ranking>> {
		let nodes = self.nodes(&self.cells(work)?)?;
		let plan = probe::plan(&at(&nodes), &self.terms()?, self.radius_m()?);
		if dry {
			for n in &nodes {
				eprintln!("  {:.5}, {:.5}   {:.1}% of demand", n.at[0], n.at[1], 100. * n.share);
			}
			eprintln!("{} nodes, biased {:.1} km", nodes.len(), self.radius_m()? / 1000.);
			let need = probe::unanswered(work, &plan)?;
			eprintln!("{} searches, {need} still unanswered: {need} Text Search Essentials", plan.len());
			return Ok(Vec::new());
		}
		let out = probe::run(work, &self.name, plan)?;
		eprintln!("probe: {} orderings over {} billed calls", out.len(), work.billed());
		Ok(out)
	}

	fn terms(&self) -> Result<Vec<String>> {
		Ok(self.trade()?.2.terms.iter().map(|t| t.text.clone()).collect())
	}

	/// The `locationBias` circle: one stratum's worth of ground, as a circle. `rank.nodes` cuts the
	/// frame into equal shares of demand, so this is how far a searcher standing at one of them is
	/// from its edge on average — which is the distance the coefficient is identified over. Not a
	/// study's to state: it needs the frame and the node count, and those sit on opposite axes.
	fn radius_m(&self) -> Result<f64> {
		let b = self.area.bbox;
		let lat_m = (b.lat[1] - b.lat[0]) * 111_320.;
		let lon_m = (b.lon[1] - b.lon[0]) * 111_320. * ((b.lat[0] + b.lat[1]) / 2.).to_radians().cos();
		Ok((lat_m * lon_m / self.trade()?.2.nodes as f64 / std::f64::consts::PI).sqrt())
	}

	fn nodes(&self, cells: &Cells) -> Result<Vec<rank::Node>> {
		let at: Vec<[f64; 2]> = cells
			.grid
			.cells
			.iter()
			.map(|c| {
				let (lon, lat) = c.ring.iter().fold((0., 0.), |(x, y), p| (x + p[0] / 4., y + p[1] / 4.));
				[lat, lon]
			})
			.collect();
		rank::nodes(&at, self.demand(cells)?, self.trade()?.2.nodes)
	}

	/// A location opened without a trade stops at the grid: nothing below reaches `poi::load`, so it
	/// spends no Places call and no day of the quota.
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
		let (trade, notice) = match (&self.poi, &self.model, &self.rank) {
			(None, None, None) => (None, None),
			// a half-written trade is not a location, and `trade()` says which block is missing
			_ => match self.billed(&cells, &place, work) {
				Ok(t) => (Some(t), None),
				// the quota and `--block-spend` are the refusals that are neither this tool's fault nor a study
				// that will not build: what the archive paid for is already here, so the map opens on it and
				// says why the ground is empty. A partial sweep is not served — half an inventory reads as a whole one
				Err(e) => match e
					.downcast_ref::<sources::work::Exhausted>()
					.map(ToString::to_string)
					.or_else(|| e.downcast_ref::<sources::work::Blocked>().map(ToString::to_string))
				{
					Some(why) => (None, Some(format!("{why}. No competitor is on this map, and nothing that reads one is offered."))),
					None => return Err(e),
				},
			},
		};
		let mut layers = Vec::new();
		if let Some(n) = &notice {
			eprintln!("{}: {n}", self.name);
			// the demand surface is the trade's expression over the archive and was evaluated before the
			// first call, so it is the one thing a spent day does not take away. First, so the map opens
			// on it rather than on whatever the location happened to list
			let model = self.model.as_ref().expect("a study that reached the sweep declares a model");
			layers.push(LayerOut {
				name: "Demand".to_owned(),
				note: model.demand.clone(),
				scale: Scale::Percentile,
				values: round(self.demand(&cells)?.to_vec(), 3),
			});
		}
		layers.extend(cells.layers);
		Ok(Payload {
			name: self.name.clone(),
			center: self.area.center,
			zoom: self.area.zoom,
			ring,
			imputed: cells.grid.cells.iter().map(|c| u8::from(c.imputed)).collect(),
			place,
			layers,
			candidates: self.candidates.clone(),
			trade,
			notice,
		})
	}

	/// The billed half. Every Places call this tool makes on a map is behind this.
	fn billed(&self, cells: &Cells, place: &[String], work: &Work) -> Result<payload::Trade> {
		let (poi_cfg, model_cfg, rank_cfg) = self.trade()?;
		let inv = poi::load(poi_cfg, self.area.bbox, &self.name, work)?;
		let model = Rank::try_new(feats(place, &inv.pois)?, rank::COEF)?;
		let terms: Vec<(String, f64)> = rank_cfg.terms.iter().map(|t| (t.text.clone(), t.weight)).collect();

		// a study nobody has probed carries no nodes, and the map offers no observed coverage layer
		let nodes = at(&self.nodes(cells)?);
		let probed = probe::cached(work, &probe::plan(&nodes, &self.terms()?, self.radius_m()?))?;
		let seen = gmaps_optimal_placement_rank::observed(&inv.pois, &nodes, &probed, &terms);
		let nodes = match probed.is_empty() {
			true => Vec::new(),
			false => nodes,
		};

		let kept: std::collections::HashSet<&str> = inv.pois.iter().map(|p| p.id.as_str()).collect();
		let mut queries: Vec<payload::QueryOut> = poi_cfg
			.queries
			.iter()
			.map(|q| {
				let hit: std::collections::HashSet<&str> = inv
					.obs
					.iter()
					.filter(|o| &o.term == q)
					.flat_map(|o| o.ids.iter().map(String::as_str))
					.filter(|id| kept.contains(id))
					.collect();
				payload::QueryOut { text: q.clone(), hits: hit.len() }
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
		normalise(&mut pois, &poi_cfg.tiers)?;
		for t in &poi_cfg.tiers {
			ensure!(pois.iter().any(|p| p.poi.tier == t.name), "no competitor fell into tier {:?}", t.name);
		}
		Ok(payload::Trade {
			lambda_m: model_cfg.lambda_m,
			demand_note: model_cfg.demand.clone(),
			demand: round(self.demand(cells)?.to_vec(), 3),
			tiers: poi_cfg
				.tiers
				.iter()
				.map(|t| TierOut {
					name: t.name.clone(),
					weight: t.weight,
				})
				.collect(),
			terms: terms.into_iter().map(|(text, weight)| payload::TermOut { text, weight }).collect(),
			queries,
			nodes,
			pois,
			searches: inv.calls,
		})
	}

	pub fn searches(&self, work: &Work) -> Result<SearchPayload> {
		let cfg = self.searches.as_ref().ok_or_else(|| eyre::eyre!("study {:?} declares no `searches` block", self.name))?;
		ensure!(!cfg.groups.is_empty(), "searches declares no group");
		let provider = searches::provider(&cfg.provider)?;
		let (mut months, mut groups) = (Vec::new(), Vec::<GroupOut>::new());
		for g in &cfg.groups {
			ensure!(!groups.iter().any(|o| o.name == g.name), "two searches.group are both named {:?}", g.name);
			let ideas = provider.ideas(&g.seed, &cfg.place, &cfg.language, work)?;
			eprintln!("{}: {} ideas around {:?}", g.name, ideas.len(), g.seed);
			let (axis, out) = fold(g, ideas)?;
			match groups.first() {
				None => months = axis,
				Some(first) => ensure!(months == axis, "group {:?} covers {axis:?}, group {:?} covers {months:?}", g.name, first.name),
			}
			groups.push(out);
		}
		Ok(SearchPayload {
			name: self.name.clone(),
			place: cfg.place.clone(),
			months,
			groups,
		})
	}
}

/// The group filter and its sum. Kept out of the provider so a canned response can drive it.
pub fn fold(group: &Group, ideas: Vec<Keyword>) -> Result<(Vec<String>, GroupOut)> {
	let compile = |p: &String| Regex::new(&format!("(?i){p}")).wrap_err_with(|| format!("group {:?} pattern {p:?}", group.name));
	let keep = group.pattern.as_ref().map(&compile).transpose()?;
	let drop = group.drop.as_ref().map(&compile).transpose()?;

	let mut months: Vec<String> = Vec::new();
	let (mut members, mut no_data) = (Vec::new(), Vec::new());
	for k in ideas {
		if drop.as_ref().is_some_and(|r| r.is_match(&k.text)) || !keep.as_ref().is_none_or(|r| r.is_match(&k.text)) {
			continue;
		}
		let Some(series) = k.monthly else {
			no_data.push(k.text);
			continue;
		};
		ensure!(!series.is_empty(), "group {:?}: keyword {:?} carries an empty series", group.name, k.text);
		let axis: Vec<String> = series.iter().map(|m| format!("{:04}-{:02}", m.year, m.month)).collect();
		if months.is_empty() {
			months = axis;
		} else {
			ensure!(months == axis, "group {:?}: keyword {:?} covers {axis:?}, the group covers {months:?}", group.name, k.text);
		}
		let monthly: Vec<u64> = series.iter().map(|m| m.searches).collect();
		let avg = monthly.iter().sum::<u64>() as f64 / monthly.len() as f64;
		members.push(MemberOut {
			text: k.text,
			avg: round1(avg, 1),
			monthly,
		});
	}
	ensure!(
		!members.is_empty(),
		"group {:?} kept no keyword the provider has volume for ({} matched without data)",
		group.name,
		no_data.len()
	);

	let mut total = vec![0u64; months.len()];
	for m in &members {
		for (t, v) in total.iter_mut().zip(&m.monthly) {
			*t += v;
		}
	}
	members.sort_by(|a, b| b.avg.total_cmp(&a.avg));
	Ok((
		months,
		GroupOut {
			name: group.name.clone(),
			total,
			members,
			no_data,
		},
	))
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

/// The stem, which is half a study's name and the whole of what a picker shows.
pub fn stem(p: &Path) -> &str {
	p.file_stem().and_then(|s| s.to_str()).expect("a study path is UTF-8 with a stem")
}

/// One of an axis, for a one-artifact subcommand. A directory is offered through `fzf`; a file is
/// itself, so a scripted run never opens a picker. `serve` does not come here — the page it serves
/// has a picker of its own, and two pickers over one directory is one too many.
pub fn pick(path: &Path) -> Result<PathBuf> {
	let found = files(path)?;
	if let [only] = found.as_slice() {
		return Ok(only.clone());
	}
	let stems: Vec<&str> = found.iter().map(|p| stem(p)).collect();

	let mut fzf = std::process::Command::new("fzf")
		.arg("--prompt")
		.arg(format!("{}> ", path.file_name().unwrap_or(path.as_os_str()).to_string_lossy()))
		.stdin(std::process::Stdio::piped())
		.stdout(std::process::Stdio::piped())
		.spawn()
		.wrap_err("spawning `fzf` — choosing among a directory needs it on PATH")?;
	let mut sink = fzf.stdin.take().expect("stdin is piped");
	std::io::Write::write_all(&mut sink, stems.join("\n").as_bytes())?;
	drop(sink);
	let out = fzf.wait_with_output()?;

	let picked = std::str::from_utf8(&out.stdout)?.trim();
	ensure!(!picked.is_empty(), "nothing picked out of {}", path.display());
	let i = stems
		.iter()
		.position(|o| *o == picked)
		.ok_or_else(|| eyre::eyre!("fzf returned {picked:?}, which is nothing under {}", path.display()))?;
	Ok(found[i].clone())
}

/// A trade applied to a location. The trade file is a function of the location's attrset, which is
/// what lets the frame and the statistics office it publishes be stated once per city rather than
/// once per city and trade.
///
/// No trade is the identity of that application: the location's own attrset is already a study, and
/// it is the one that costs nothing to build.
pub fn load(trade: Option<&Path>, location: &Path) -> Result<Study> {
	let (trade, location) = (trade.map(abs).transpose()?, abs(location)?);
	let expr = match &trade {
		Some(t) => format!("import {} (import {})", t.display(), location.display()),
		None => format!("import {}", location.display()),
	};
	// `--impure` for the absolute paths: pure evaluation only admits a path it was handed as the
	// root, and the root here is a pair
	let out = std::process::Command::new("nix")
		.args(["eval", "--impure", "--json", "--expr", &expr])
		.output()
		.wrap_err("running `nix eval` — a study is a Nix expression, so nix must be on PATH")?;
	ensure!(out.status.success(), "evaluating {expr}:\n{}", String::from_utf8_lossy(&out.stderr).trim());
	let mut study: Study = serde_json::from_slice(&out.stdout).wrap_err_with(|| format!("parsing {expr}"))?;
	study.name = match &trade {
		Some(t) => format!("{}_-_{}", stem(t), stem(&location)),
		None => stem(&location).to_owned(),
	};
	if study.layers.is_empty() {
		bail!("{} declares no [[layer]]", study.name);
	}
	Ok(study)
}

/// `nix eval --expr` resolves a bare path against the invocation directory, and the server builds
/// from whichever thread a request landed on.
fn abs(p: &Path) -> Result<PathBuf> {
	std::fs::canonicalize(p).wrap_err_with(|| format!("{} does not exist", p.display()))
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
		m.insert("demand_total".to_owned(), format!("{:.1}", sum(&t.demand)));
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
pub fn search_stats(p: &SearchPayload) -> IndexMap<String, String> {
	let mut m = IndexMap::from([
		("place".to_owned(), p.place.clone()),
		("months".to_owned(), format!("{}..{}", p.months[0], p.months[p.months.len() - 1])),
	]);
	for g in &p.groups {
		let avg = g.total.iter().sum::<u64>() as f64 / g.total.len() as f64;
		m.insert(
			format!("{}_per_month", g.name),
			format!("{avg:.0} over {} keywords, {} without data", g.members.len(), g.no_data.len()),
		);
	}
	m
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
fn normalise(pois: &mut [PoiOut], tiers: &[gmaps_optimal_placement_sources::poi::Tier]) -> Result<()> {
	for t in tiers {
		let mut w: Vec<f64> = pois.iter().filter(|p| p.poi.tier == t.name).map(|p| p.w).collect();
		if w.is_empty() {
			continue;
		}
		w.sort_by(f64::total_cmp);
		let med = w[w.len() / 2];
		ensure!(med > 0., "tier {:?} has a median strength of {med}, so it cannot be normalised", t.name);
		for p in pois.iter_mut().filter(|p| p.poi.tier == t.name) {
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
