//! EPSG:3035 (ETRS89-LAEA) <-> WGS84. proj4rs speaks radians on the geographic side.
use eyre::{Result, eyre};
use proj4rs::Proj;

const LAEA: &str = "+proj=laea +lat_0=52 +lon_0=10 +x_0=4321000 +y_0=3210000 +ellps=GRS80 +towgs84=0,0,0,0,0,0,0 +units=m +no_defs";
const WGS84: &str = "+proj=longlat +ellps=WGS84 +towgs84=0,0,0,0,0,0,0 +no_defs";

pub struct Reproject {
	laea: Proj,
	wgs84: Proj,
}

impl Reproject {
	pub fn try_new() -> Result<Self> {
		Ok(Self {
			laea: Proj::from_proj_string(LAEA).map_err(|e| eyre!("EPSG:3035 proj string: {e}"))?,
			wgs84: Proj::from_proj_string(WGS84).map_err(|e| eyre!("EPSG:4326 proj string: {e}"))?,
		})
	}

	/// -> (lon, lat) in degrees
	pub fn to_wgs84(&self, [x, y]: [f64; 2]) -> Result<(f64, f64)> {
		let mut p = (x, y, 0.0);
		proj4rs::transform::transform(&self.laea, &self.wgs84, &mut p).map_err(|e| eyre!("3035 -> 4326 at ({x}, {y}): {e}"))?;
		Ok((p.0.to_degrees(), p.1.to_degrees()))
	}

	/// (lon, lat) in degrees -> EPSG:3035 metres
	pub fn to_laea(&self, [lon, lat]: [f64; 2]) -> Result<(f64, f64)> {
		let mut p = (lon.to_radians(), lat.to_radians(), 0.0);
		proj4rs::transform::transform(&self.wgs84, &self.laea, &mut p).map_err(|e| eyre!("4326 -> 3035 at ({lon}, {lat}): {e}"))?;
		Ok((p.0, p.1))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A degree of latitude is ~111 km, so this is a millimetre — five orders below the 200 m cell
	/// this positions, and the floor of proj4rs' iterative LAEA inverse.
	const EPS_DEG: f64 = 1e-8;

	/// Against PROJ's own answer for EPSG:3035 -> EPSG:4326.
	#[test]
	fn reference_point() {
		let r = Reproject::try_new().unwrap();
		let (lon, lat) = r.to_wgs84([3945600., 2513800.]).unwrap();
		assert!((lon - 5.189070396012167).abs() < EPS_DEG, "lon {lon}");
		assert!((lat - 45.62762410095679).abs() < EPS_DEG, "lat {lat}");
	}

	#[test]
	fn round_trip() {
		let r = Reproject::try_new().unwrap();
		for (lon, lat) in [(3.0863, 45.7797), (10.0, 52.0), (-9.1, 38.7), (24.9, 60.2)] {
			let (x, y) = r.to_laea([lon, lat]).unwrap();
			let (lon2, lat2) = r.to_wgs84([x, y]).unwrap();
			assert!((lon - lon2).abs() < EPS_DEG, "({lon}, {lat}) -> ({x}, {y}) -> ({lon2}, {lat2})");
			assert!((lat - lat2).abs() < EPS_DEG, "({lon}, {lat}) -> ({x}, {y}) -> ({lon2}, {lat2})");
		}
	}

	/// The LAEA origin is the one point whose expected value needs no reference table.
	#[test]
	fn false_origin() {
		let r = Reproject::try_new().unwrap();
		let (x, y) = r.to_laea([10.0, 52.0]).unwrap();
		assert!((x - 4321000.).abs() < 1e-6, "({x}, {y})");
		assert!((y - 3210000.).abs() < 1e-6, "({x}, {y})");
	}
}
