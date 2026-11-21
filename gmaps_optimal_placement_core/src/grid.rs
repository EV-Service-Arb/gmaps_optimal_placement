//! A grid is cell geometry plus whatever numeric columns the source published.
use eyre::{Result, bail, ensure};
use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::proj::Reproject;

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Bbox {
	/// [south, north], degrees
	pub lat: [f64; 2],
	/// [west, east], degrees
	pub lon: [f64; 2],
}

impl Bbox {
	/// The axis-aligned EPSG:3035 rectangle that contains this lat/lon box. LAEA meridians are not
	/// vertical, so all four corners are projected, not two.
	pub fn to_laea(&self, proj: &Reproject) -> Result<[f64; 4]> {
		ensure!(self.lat[0] < self.lat[1] && self.lon[0] < self.lon[1], "bbox is not ordered [min, max]: {self:?}");
		let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
		for lon in self.lon {
			for lat in self.lat {
				let (x, y) = proj.to_laea([lon, lat])?;
				(x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
			}
		}
		Ok([x0, y0, x1, y1])
	}
}

/// SW corner of an EPSG:3035 cell, as encoded in `CRS3035RES{res}mN{north}E{east}`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellId {
	pub res_m: u32,
	pub north: i64,
	pub east: i64,
}

impl CellId {
	pub fn parse(s: &str) -> Result<Self> {
		let rest = s.strip_prefix("CRS3035RES").ok_or_else(|| eyre::eyre!("cell id {s:?} is not CRS3035"))?;
		let (res, rest) = rest.split_once('m').ok_or_else(|| eyre::eyre!("cell id {s:?} has no resolution"))?;
		let rest = rest.strip_prefix('N').ok_or_else(|| eyre::eyre!("cell id {s:?} has no northing"))?;
		let (north, east) = rest.split_once('E').ok_or_else(|| eyre::eyre!("cell id {s:?} has no easting"))?;
		Ok(Self {
			res_m: res.parse()?,
			north: north.parse()?,
			east: east.parse()?,
		})
	}

	/// Cell outline, SW -> SE -> NE -> NW, as (lon, lat) degrees.
	pub fn ring(&self, proj: &Reproject) -> Result<[[f64; 2]; 4]> {
		let (e, n, s) = (self.east as f64, self.north as f64, self.res_m as f64);
		let mut ring = [[0.0; 2]; 4];
		for (slot, (x, y)) in ring.iter_mut().zip([(e, n), (e + s, n), (e + s, n + s), (e, n + s)]) {
			let (lon, lat) = proj.to_wgs84([x, y])?;
			*slot = [lon, lat];
		}
		Ok(ring)
	}
}

#[derive(Clone, Debug, Serialize)]
pub struct Cell {
	pub id: String,
	/// Human label from the source's own administrative column; "" when it publishes none.
	pub place: String,
	pub ring: [[f64; 2]; 4],
	/// Modelled rather than observed, where the source says so.
	pub imputed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Grid {
	pub cells: Vec<Cell>,
	pub columns: IndexMap<String, Vec<f64>>,
}

impl Grid {
	pub fn len(&self) -> usize {
		self.cells.len()
	}

	pub fn is_empty(&self) -> bool {
		self.cells.is_empty()
	}

	pub fn push(&mut self, cell: Cell, row: &[(&str, f64)]) -> Result<()> {
		let n = self.cells.len();
		if n == 0 {
			for (name, _) in row {
				self.columns.insert((*name).to_owned(), Vec::new());
			}
		}
		ensure!(row.len() == self.columns.len(), "row has {} columns, grid has {}", row.len(), self.columns.len());
		for (name, v) in row {
			let Some(col) = self.columns.get_mut(*name) else {
				bail!("column {name:?} absent from the first row")
			};
			ensure!(col.len() == n, "column {name:?} appears twice in one row");
			col.push(*v);
		}
		self.cells.push(cell);
		Ok(())
	}

	/// Only the cells `keep` holds, every column with them.
	pub fn retain(&mut self, keep: impl Fn(&Cell) -> bool) {
		let flags: Vec<bool> = self.cells.iter().map(&keep).collect();
		for col in self.columns.values_mut() {
			let mut f = flags.iter();
			col.retain(|_| *f.next().expect("a column is as long as the cells"));
		}
		let mut f = flags.iter();
		self.cells.retain(|_| *f.next().expect("one flag per cell"));
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn ids_at_both_resolutions() {
		assert_eq!(
			CellId::parse("CRS3035RES200mN2029400E4259000").unwrap(),
			CellId {
				res_m: 200,
				north: 2029400,
				east: 4259000
			}
		);
		assert_eq!(
			CellId::parse("CRS3035RES1000mN2683000E4285000").unwrap(),
			CellId {
				res_m: 1000,
				north: 2683000,
				east: 4285000
			}
		);
		assert_eq!(
			CellId::parse("CRS3035RES32000mN2016000E4256000").unwrap(),
			CellId {
				res_m: 32000,
				north: 2016000,
				east: 4256000
			}
		);
		for bad in ["CRS3035RES200mN2029400", "1kmN123E456", "CRS3035RES200mNxxxE4259000"] {
			assert!(CellId::parse(bad).is_err(), "{bad} parsed");
		}
	}

	#[test]
	fn ring_is_a_cell_of_the_stated_size() {
		let proj = Reproject::try_new().unwrap();
		let ring = CellId::parse("CRS3035RES200mN2513800E3945600").unwrap().ring(&proj).unwrap();
		assert!((ring[0][0] - 5.189070396012167).abs() < 1e-9, "SW corner is the id corner: {:?}", ring[0]);
		// 200 m of northing at 45.6 deg is 200/111320 deg of latitude, within LAEA's local distortion
		let dlat = ring[3][1] - ring[0][1];
		assert!((dlat - 200. / 111_320.).abs() < 2e-5, "north edge spans {dlat} deg");
	}
}
