//! What is drawn of an area: its outline grown by its margin. Cells and competitors are taken
//! inside it.
use gmaps_optimal_placement_core::grid::Bbox;
use sa_core::{geo::LatLng, placement::Area};

const M_PER_DEG: f64 = 111_320.;

#[derive(Clone, Copy, Debug)]
pub struct Frame {
	pub bbox: Bbox,
	/// [lat, lon] the outline is about.
	pub center: [f64; 2],
	circle: Option<(LatLng, f64)>,
}
impl Frame {
	pub fn contains(&self, lat: f64, lng: f64) -> bool {
		let b = self.bbox;
		let boxed = lat >= b.lat[0] && lat <= b.lat[1] && lng >= b.lon[0] && lng <= b.lon[1];
		match self.circle {
			None => boxed,
			Some((c, r)) => {
				let dy = (lat - c.lat) * M_PER_DEG;
				let dx = (lng - c.lng) * M_PER_DEG * c.lat.to_radians().cos();
				boxed && dx * dx + dy * dy <= r * r
			}
		}
	}

	/// Ground covered, m².
	pub fn area_m2(&self) -> f64 {
		match self.circle {
			Some((_, r)) => std::f64::consts::PI * r * r,
			None => {
				let b = self.bbox;
				let lat_m = (b.lat[1] - b.lat[0]) * M_PER_DEG;
				let lon_m = (b.lon[1] - b.lon[0]) * M_PER_DEG * ((b.lat[0] + b.lat[1]) / 2.).to_radians().cos();
				lat_m * lon_m
			}
		}
	}

	/// The Web-Mercator zoom at which the frame fills a ~1000 px map.
	pub fn zoom(&self) -> u8 {
		let b = self.bbox;
		let span = (b.lon[1] - b.lon[0]).max((b.lat[1] - b.lat[0]) / ((b.lat[0] + b.lat[1]) / 2.).to_radians().cos());
		(1000. / 256. * 360. / span).log2().floor().clamp(0., 18.) as u8
	}
}

impl From<&Area> for Frame {
	fn from(area: &Area) -> Self {
		match *area {
			Area::City { viewport: v, margin_m, .. } => {
				let dlat = margin_m / M_PER_DEG;
				let dlon = margin_m / (M_PER_DEG * ((v.south + v.north) / 2.).to_radians().cos());
				Self {
					bbox: Bbox {
						lat: [v.south - dlat, v.north + dlat],
						lon: [v.west - dlon, v.east + dlon],
					},
					center: [(v.south + v.north) / 2., (v.west + v.east) / 2.],
					circle: None,
				}
			}
			Area::Circle { center: c, radius_m, margin_m, .. } => {
				let r = radius_m + margin_m;
				let (dlat, dlon) = (r / M_PER_DEG, r / (M_PER_DEG * c.lat.to_radians().cos()));
				Self {
					bbox: Bbox {
						lat: [c.lat - dlat, c.lat + dlat],
						lon: [c.lng - dlon, c.lng + dlon],
					},
					center: [c.lat, c.lng],
					circle: Some((c, r)),
				}
			}
		}
	}
}
