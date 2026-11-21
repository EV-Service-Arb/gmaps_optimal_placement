//! Statistical grids published as one CSV of EPSG:3035 cells.
//!
//! Both catalogued sources encode the cell corner in its id, so the whole difference between them
//! is the [`Archive`] table below: where the file is, which columns are text, and which one carries
//! provenance.
use std::collections::{BTreeSet, HashMap};

use eyre::{Result, WrapErr, bail, ensure};
use gmaps_optimal_placement_core::{
	Reproject,
	grid::{Bbox, Cell, CellId, Grid},
};
use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::work::{Kind, Work};

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub enum GridSource {
	/// INSEE Filosofi — France, 200 m, households / housing type / standard of living.
	#[serde(rename = "insee_filosofi_200m")]
	InseeFilosofi200m,
	/// Eurostat census grid — EU, 1 km, head counts by age, sex, employment and origin.
	#[serde(rename = "geostat_1km")]
	Geostat1km,
}
impl GridSource {
	fn archive(self, vintage: u16) -> Result<Archive> {
		Ok(match (self, vintage) {
			(Self::InseeFilosofi200m, 2021) => Archive {
				url: "https://www.insee.fr/fr/statistiques/fichier/8735162/Filosofi2021_carreaux_200m_csv.zip",
				member: "carreaux_200m_met.csv",
				id_col: "idcar_200m",
				place_col: "lcog_geo",
				imputed_col: Some("i_est_200"),
				text_cols: &["idcar_200m", "idcar_1km", "idcar_nat", "lcog_geo"],
				res_m: 200,
			},
			(Self::Geostat1km, 2021) => Archive {
				url: "https://gisco-services.ec.europa.eu/census/2021/Eurostat_Census-GRID_2021_V3.zip",
				member: "Eurostat_Census-GRID_2021_V3/ESTAT_Census_2021_V3.csv",
				id_col: "GRD_ID",
				place_col: "CNTR_ID",
				imputed_col: None,
				text_cols: &["GRD_ID", "CNTR_ID"],
				res_m: 1000,
			},
			(s, v) => bail!("no {s:?} archive for vintage {v}"),
		})
	}
}

/// The cells of one frame, out of an archive that holds a country's worth.
///
/// The scan reads every row the archive has — 2.3 million for France — to keep the ten thousand
/// inside the frame, and the answer never moves: the archive is a fixed vintage and the frame is the
/// study's. So the rows it kept are written out beside it, verbatim, and a rerun reads those.
///
/// Rows and not cells: the numbers on a cell are parsed and reprojected from this text, and a cache
/// of the results would have to round-trip a float exactly to be the same grid. Keeping the input
/// and running the same code over it needs no such promise.
pub fn load(source: GridSource, vintage: u16, bbox: Bbox, work: &Work) -> Result<Grid> {
	let a = source.archive(vintage)?;
	let proj = Reproject::try_new()?;
	let key = Work::digest(&[&format!("{source:?}"), &vintage.to_string(), &serde_json::to_string(&bbox)?]);
	let kept = work.derived("data/extract", &format!("{key}.csv"))?;

	let mut grid = match std::fs::File::open(&kept).ok() {
		Some(f) => {
			let mut rdr = csv::ReaderBuilder::new().from_reader(std::io::BufReader::new(f));
			let grid = collect(&a, bbox.to_laea(&proj)?, &proj, &mut rdr, None)?;
			eprintln!("{:?}: {} cells, off the rows an earlier scan kept", a.member, grid.len());
			grid
		}
		None => {
			let path = work.archive(a.url)?;
			let file = std::fs::File::open(&path).wrap_err_with(|| format!("opening {}", path.display()))?;
			let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).wrap_err_with(|| format!("{} is not a zip", path.display()))?;
			let member = zip.by_name(a.member).wrap_err_with(|| format!("{} has no member {:?}", path.display(), a.member))?;
			let mut rdr = csv::ReaderBuilder::new().from_reader(std::io::BufReader::with_capacity(1 << 20, member));
			// written aside under a name no other scan can be using, and renamed into place: two runs
			// over one frame are one file, and a scan that dies leaves no half a frame to be read as whole
			let part = kept.with_extension(format!("{}.{}.part", std::process::id(), PARTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
			let mut out = csv::Writer::from_path(&part).wrap_err_with(|| format!("creating {}", part.display()))?;
			let grid = collect(&a, bbox.to_laea(&proj)?, &proj, &mut rdr, Some(&mut out))?;
			out.flush()?;
			drop(out);
			std::fs::rename(&part, &kept)?;
			grid
		}
	};
	ensure!(!grid.is_empty(), "no {source:?} cell falls inside {bbox:?}");

	if source == GridSource::InseeFilosofi200m {
		let labels = commune_names(&grid.cells.iter().map(|c| c.place.clone()).collect(), work)?;
		for cell in &mut grid.cells {
			cell.place = labels[&cell.place].clone();
		}
	}
	Ok(grid)
}

/// One administrative unit of the source's own place column: every numeric column summed over its
/// cells, and where those cells are.
#[derive(Clone, Debug)]
pub struct Place {
	pub code: String,
	/// The source's label for the code, or the code itself where it publishes none.
	pub name: String,
	/// [lat, lon]. The mean of the unit's cell centres, and Filosofi publishes only inhabited cells,
	/// so this sits where the people are rather than in the middle of the commune's outline.
	pub at: [f64; 2],
	pub cells: usize,
	pub sum: IndexMap<String, f64>,
}

/// The whole country rolled up by place, without the bbox that [`load`] needs: summing is O(units)
/// where keeping the cells is O(cells), and for France that is 35 k against 2.3 M.
pub fn places(source: GridSource, vintage: u16, work: &Work) -> Result<IndexMap<String, Place>> {
	let a = source.archive(vintage)?;
	let proj = Reproject::try_new()?;

	let path = work.archive(a.url)?;
	let file = std::fs::File::open(&path).wrap_err_with(|| format!("opening {}", path.display()))?;
	let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).wrap_err_with(|| format!("{} is not a zip", path.display()))?;
	let member = zip.by_name(a.member).wrap_err_with(|| format!("{} has no member {:?}", path.display(), a.member))?;
	let mut rdr = csv::ReaderBuilder::new().from_reader(std::io::BufReader::with_capacity(1 << 20, member));

	let header: Vec<String> = rdr.headers()?.iter().map(str::to_owned).collect();
	let at = |name: &str| {
		header
			.iter()
			.position(|h| h == name)
			.ok_or_else(|| eyre::eyre!("{:?} has no column {name:?}; header is {}", a.member, header.join(", ")))
	};
	let (i_id, i_place) = (at(a.id_col)?, at(a.place_col)?);
	let numeric: Vec<usize> = (0..header.len()).filter(|i| !a.text_cols.contains(&header[*i].as_str())).collect();

	// Summed in projected metres and reprojected once per unit rather than once per cell: at commune
	// scale the two centres differ by centimetres, and this is 35 k reprojections instead of 2.3 M.
	let mut laea: IndexMap<String, (f64, f64, usize)> = IndexMap::new();
	let mut out: IndexMap<String, Place> = IndexMap::new();
	let (mut scanned, mut row) = (0usize, csv::StringRecord::new());
	while rdr.read_record(&mut row)? {
		scanned += 1;
		let id = &row[i_id];
		let cell = CellId::parse(id).wrap_err_with(|| format!("row {scanned} of {:?}", a.member))?;
		ensure!(cell.res_m == a.res_m, "{id} is a {} m cell, {:?} is meant to be {} m", cell.res_m, a.member, a.res_m);
		// A cell straddling a border lists every commune it touches; the first is the dominant one.
		let code = row[i_place].split(',').next().unwrap_or_default();
		let half = a.res_m as f64 / 2.;
		let e = laea.entry(code.to_owned()).or_insert((0., 0., 0));
		*e = (e.0 + cell.east as f64 + half, e.1 + cell.north as f64 + half, e.2 + 1);
		let place = out.entry(code.to_owned()).or_insert_with(|| Place {
			code: code.to_owned(),
			name: code.to_owned(),
			at: [0., 0.],
			cells: 0,
			sum: IndexMap::new(),
		});
		place.cells += 1;
		for &i in &numeric {
			let raw = &row[i];
			let v: f64 = raw.parse().wrap_err_with(|| format!("column {:?} of cell {id} is {raw:?}, not a number", header[i]))?;
			*place.sum.entry(header[i].clone()).or_insert(0.) += v;
		}
	}
	ensure!(!out.is_empty(), "{:?} published no cell at all", a.member);
	for (code, (x, y, n)) in &laea {
		let (lon, lat) = proj.to_wgs84([x / *n as f64, y / *n as f64])?;
		out[code].at = [lat, lon];
	}
	eprintln!("{:?}: scanned {scanned} cells, rolled up into {} places", a.member, out.len());

	if source == GridSource::InseeFilosofi200m {
		let labels = commune_names(&out.keys().cloned().collect(), work)?;
		for place in out.values_mut() {
			place.name = labels[&place.code].clone();
		}
	}
	Ok(out)
}

/// Distinguishes one in-flight scan from another inside a process, as the pid does across them.
static PARTS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Every row inside the frame, as cells. `keep` is where a scan of the whole archive writes the rows
/// it is keeping; reading those back comes through here again with nothing to write.
fn collect<R: std::io::Read>(a: &Archive, frame: [f64; 4], proj: &Reproject, rdr: &mut csv::Reader<R>, mut keep: Option<&mut csv::Writer<std::fs::File>>) -> Result<Grid> {
	let [x0, y0, x1, y1] = frame;
	let header: Vec<String> = rdr.headers()?.iter().map(str::to_owned).collect();
	let at = |name: &str| {
		header
			.iter()
			.position(|h| h == name)
			.ok_or_else(|| eyre::eyre!("{:?} has no column {name:?}; header is {}", a.member, header.join(", ")))
	};
	let (i_id, i_place) = (at(a.id_col)?, at(a.place_col)?);
	let i_imputed = a.imputed_col.map(&at).transpose()?;
	for t in a.text_cols {
		at(t)?;
	}
	let numeric: Vec<usize> = (0..header.len()).filter(|i| !a.text_cols.contains(&header[*i].as_str())).collect();
	if let Some(w) = keep.as_deref_mut() {
		w.write_record(&header)?;
	}

	let mut grid = Grid::default();
	let (mut scanned, mut row) = (0usize, csv::StringRecord::new());
	let mut values: Vec<(&str, f64)> = Vec::with_capacity(numeric.len());
	while rdr.read_record(&mut row)? {
		scanned += 1;
		let id = &row[i_id];
		let cell = CellId::parse(id).wrap_err_with(|| format!("row {scanned} of {:?}", a.member))?;
		ensure!(cell.res_m == a.res_m, "{id} is a {} m cell, {:?} is meant to be {} m", cell.res_m, a.member, a.res_m);
		let (e, n) = (cell.east as f64, cell.north as f64);
		if e < x0 || e > x1 || n < y0 || n > y1 {
			continue;
		}
		values.clear();
		for &i in &numeric {
			let raw = &row[i];
			let v: f64 = raw.parse().wrap_err_with(|| format!("column {:?} of cell {id} is {raw:?}, not a number", header[i]))?;
			values.push((&header[i], v));
		}
		let imputed = match i_imputed {
			Some(i) => match &row[i] {
				"1" => true,
				"0" => false,
				other => bail!("provenance flag {:?} of cell {id} is {other:?}, expected 0 or 1", a.imputed_col),
			},
			None => false,
		};
		if let Some(w) = keep.as_deref_mut() {
			w.write_record(row.iter())?;
		}
		let cell = Cell {
			id: id.to_owned(),
			place: row[i_place].split(',').next().unwrap_or_default().to_owned(),
			ring: cell.ring(proj)?,
			imputed,
		};
		grid.push(cell, &values)?;
	}
	if keep.is_some() {
		eprintln!("{:?}: scanned {scanned} cells, kept {}", a.member, grid.len());
	}
	Ok(grid)
}

struct Archive {
	url: &'static str,
	member: &'static str,
	id_col: &'static str,
	place_col: &'static str,
	/// 1 where the row is modelled rather than observed.
	imputed_col: Option<&'static str>,
	/// Everything else in the header must parse as a number, for every cell.
	text_cols: &'static [&'static str],
	res_m: u32,
}

/// INSEE's `lcog_geo` is a commune code. The map wants the name. Every code asked for comes back,
/// naming itself where the current commune list no longer has it.
fn commune_names(codes: &BTreeSet<String>, work: &Work) -> Result<HashMap<String, String>> {
	let mut names: HashMap<String, String> = HashMap::new();
	let depts: BTreeSet<&str> = codes.iter().map(|c| &c[..2]).collect();
	let urls = depts
		.iter()
		.map(|d| (format!("https://geo.api.gouv.fr/departements/{d}/communes?fields=nom"), format!("communes_{d}.json")));
	// Paris, Lyon and Marseille are one commune each to the department list and a dozen to Filosofi,
	// which counts their arrondissements. Asked for only when something went unnamed without it, so a
	// study outside the three pays nothing.
	let arm = std::iter::once((
		"https://geo.api.gouv.fr/communes?type=arrondissement-municipal&fields=nom".to_owned(),
		"communes_arrondissements.json".to_owned(),
	));
	for (url, cache) in urls.chain(arm) {
		if codes.iter().all(|c| names.contains_key(c)) {
			break;
		}
		let (list, at) = work.cached_get(&url, &cache)?;
		work.record(Kind::Communes, at);
		for c in list.as_array().ok_or_else(|| eyre::eyre!("{url} did not return an array"))? {
			let (code, nom) = (c["code"].as_str(), c["nom"].as_str());
			let (Some(code), Some(nom)) = (code, nom) else {
				bail!("{url} returned a commune without code and nom: {c}")
			};
			names.insert(code.to_owned(), nom.to_owned());
		}
	}
	// A grid vintage is fixed; the commune list is current. Codes retired by a merger since keep
	// their code as the label — this names a cell, it does not feed the model.
	let mut retired = BTreeSet::new();
	let out = codes
		.iter()
		.map(|code| {
			let name = names.get(code).cloned().unwrap_or_else(|| {
				retired.insert(code.clone());
				code.clone()
			});
			(code.clone(), name)
		})
		.collect();
	if !retired.is_empty() {
		eprintln!("communes: {} code(s) no longer current, shown as codes: {retired:?}", retired.len());
	}
	if work.age(Kind::Communes).is_some_and(|a| a.stale) {
		eprintln!("communes: the cached list is past `age.communes`, so a merger since then shows as a code");
	}
	Ok(out)
}
