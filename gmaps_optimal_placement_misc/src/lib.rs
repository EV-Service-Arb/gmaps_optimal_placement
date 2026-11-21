#![doc = include_str!("../README.md")]

pub mod cadastre;

use eyre::{Result, ensure};
use gmaps_optimal_placement_sources::{GridSource, Work, grid};
use indexmap::IndexMap;

const MAP: &str = include_str!("map_template.html");

/// Who publishes the grid and the cadastre. Closed, matched on, one row per country — the same
/// containment `GridSource` has.
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum Country {
	France,
}

impl Country {
	/// The vintage is the country's, not a choice: it is the grid the cadastre snapshot is paired
	/// with, and pairing a 2021 denominator with a 2026 count is already the widest gap here.
	fn grid(self) -> (GridSource, u16) {
		match self {
			Self::France => (GridSource::InseeFilosofi200m, 2021),
		}
	}

	fn count(self, tally: Tally, codes: &[String], work: &Work) -> Result<Vec<Option<u32>>> {
		match self {
			Self::France => cadastre::count(tally, codes, work),
		}
	}
}

/// What to count. Everything here is something an official cadastre already draws, so a variant is a
/// layer name and a filter, never a detection of our own.
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum Tally {
	/// In-ground swimming pool. The one variant checked against a published count.
	Pool,
	/// Every building footprint. ~1 MB per commune against a pool layer's ~10 kB.
	Building,
	/// Every cadastral parcel. Same weight as `Building`.
	Parcel,
}

#[derive(Clone, Debug)]
pub struct Unit {
	pub code: String,
	pub name: String,
	/// [lat, lon], where the unit's inhabited cells are.
	pub at: [f64; 2],
	/// The denominator column, summed over the unit.
	pub per: f64,
	/// `None` where the cadastre publishes no vector layer for this unit.
	pub tally: Option<u32>,
}

impl Unit {
	/// Per thousand, because per capita reads as a string of zeroes for anything a household buys
	/// once.
	fn rate(&self) -> Option<f64> {
		self.tally.map(|t| 1000. * f64::from(t) / self.per)
	}
}

pub struct Compiled {
	pub country: Country,
	pub tally: Tally,
	/// The grid column the tally is divided by.
	pub per: String,
	pub floor: f64,
	/// Above the floor, ordered by rate, densest first. Units the cadastre has no layer for sort last.
	pub units: Vec<Unit>,
}
impl Compiled {
	pub fn name(&self) -> String {
		format!("{:?}-{:?}-per-{}", self.country, self.tally, self.per).to_lowercase()
	}

	/// One file that opens from disk. Colour is the rate, circle area is the denominator.
	pub fn render(&self) -> Result<String> {
		let plotted: Vec<&Unit> = self.units.iter().filter(|u| u.tally.is_some()).collect();
		ensure!(!plotted.is_empty(), "the cadastre published no layer for any unit above the floor");
		// The ramp tops out at the 98th percentile: the Riviera is an order of magnitude off the rest
		// of France and would otherwise flatten the whole country to one shade.
		let cap = plotted[plotted.len().div_euclid(50)].rate().expect("filtered to Some");
		ensure!(cap > 0., "the 98th percentile rate is {cap}, so there is no ramp to draw");

		let rows: Vec<_> = plotted
			.iter()
			.map(|u| serde_json::json!([u.name, round(u.at[0], 4), round(u.at[1], 4), u.per, u.tally, round(u.rate().expect("filtered to Some"), 1)]))
			.collect();
		let (source, vintage) = self.country.grid();
		let sub = format!("{source:?} {vintage} · {} units of {}+ {}", plotted.len(), self.floor, self.per);
		let mut html = MAP.to_owned();
		for (tag, value) in [
			("/*__DATA__*/null", serde_json::to_string(&rows)?),
			("/*__CAP__*/0", format!("{cap:.1}")),
			("__TITLE__", format!("{} per 1000 {}", self.label(), self.per)),
			("__SUB__", sub),
		] {
			ensure!(html.contains(tag), "placeholder {tag} missing from the template");
			html = html.replace(tag, &value);
		}
		Ok(html)
	}

	/// Summary first, then the head of the ranking — the only part of a 5 000-row table anyone reads.
	pub fn stats(&self) -> IndexMap<String, String> {
		let plotted: Vec<&Unit> = self.units.iter().filter(|u| u.tally.is_some()).collect();
		let (t, p) = plotted.iter().fold((0u64, 0.), |(t, p), u| (t + u64::from(u.tally.expect("filtered to Some")), p + u.per));
		let mut m = IndexMap::from([
			("units".to_owned(), format!("{} above {} {}", plotted.len(), self.floor, self.per)),
			("no_cadastre".to_owned(), (self.units.len() - plotted.len()).to_string()),
			(format!("{}_total", self.label()), t.to_string()),
			("national".to_owned(), format!("{:.1} per 1000 {} over the units plotted", 1000. * t as f64 / p, self.per)),
		]);
		for (i, u) in plotted.iter().take(12).enumerate() {
			m.insert(
				format!("{:2}. {}", i + 1, u.name),
				format!(
					"{:.1} per 1000 · {} {} · {:.0} {}",
					u.rate().expect("filtered to Some"),
					u.tally.expect("filtered to Some"),
					self.label(),
					u.per,
					self.per
				),
			);
		}
		m
	}

	fn label(&self) -> String {
		format!("{:?}s", self.tally).to_lowercase()
	}
}

/// Roll the country up by administrative unit, drop everything under the floor, and count the tally
/// in what is left. The floor comes first because it is the only thing that keeps this from
/// downloading a country: four fifths of French communes are under 2 000 inhabitants.
pub fn compile(country: Country, tally: Tally, per: &str, floor: f64, work: &Work) -> Result<Compiled> {
	ensure!(floor >= 0., "floor {floor} is negative");
	let (source, vintage) = country.grid();
	let places = grid::places(source, vintage, work)?;

	let first = places.values().next().expect("grid::places errors on an empty archive");
	ensure!(
		first.sum.contains_key(per),
		"{source:?} publishes no column {per:?}; it has {}",
		first.sum.keys().cloned().collect::<Vec<_>>().join(", ")
	);
	let kept: Vec<&grid::Place> = places.values().filter(|p| p.sum[per] >= floor && p.sum[per] > 0.).collect();
	ensure!(
		!kept.is_empty(),
		"no unit reaches a {per} of {floor}; the largest is {:.0}",
		places.values().map(|p| p.sum[per]).fold(0., f64::max)
	);
	eprintln!("{} of {} places clear {per} >= {floor}", kept.len(), places.len());

	let codes: Vec<String> = kept.iter().map(|p| p.code.clone()).collect();
	let counts = country.count(tally, &codes, work)?;
	let mut units: Vec<Unit> = kept
		.iter()
		.zip(counts)
		.map(|(p, tally)| Unit {
			code: p.code.clone(),
			name: p.name.clone(),
			at: p.at,
			per: round(p.sum[per], 1),
			tally,
		})
		.collect();
	units.sort_by(|a, b| b.rate().unwrap_or(f64::MIN).total_cmp(&a.rate().unwrap_or(f64::MIN)));
	Ok(Compiled {
		country,
		tally,
		per: per.to_owned(),
		floor,
		units,
	})
}

fn round(x: f64, places: i32) -> f64 {
	let s = 10f64.powi(places);
	(x * s).round() / s
}
