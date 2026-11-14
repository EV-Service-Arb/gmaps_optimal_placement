//! What a host hands in to have a study built: the conductor's area and the conductor's category,
//! named as the conductor names them. Shared by the overlay, which addresses the definition by its
//! hash, and the server, which checks that hash against the body it was sent.
use eyre::{Result, ensure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::grid::Bbox;

const M_PER_DEG: f64 = 111_320.;

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
	pub area: Area,
	/// None is the area on its own: the grid's layers, and not one billed call.
	pub category: Option<Category>,
}

/// An area as the conductor keeps it: everyone's geography, a city's viewport or a circle, and a
/// margin around it that is still "next to it".
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Area {
	City {
		slug: String,
		country: String,
		viewport: Viewport,
		margin_m: f64,
	},
	Circle {
		slug: String,
		country: String,
		center: LatLng,
		radius_m: f64,
		margin_m: f64,
	},
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Viewport {
	pub south: f64,
	pub west: f64,
	pub north: f64,
	pub east: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LatLng {
	pub lat: f64,
	pub lng: f64,
}

/// A trade set up for a country, as the conductor holds it. `study` names the gop trade that
/// carries what only an expert writes: the demand model, the tier vocabulary, λ.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
	pub trade: u64,
	pub country: String,
	pub study: String,
	/// What a customer types, in the country's language, naming no city.
	pub keywords: Vec<String>,
	pub google_primary: Gcid,
	pub google_additional: Vec<Gcid>,
}

/// One of Google's Business Profile categories, named in the country's language.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Gcid {
	pub id: String,
	pub name: String,
}

impl Gcid {
	/// The Places type of the same name, which Places returns only for the categories its own
	/// taxonomy has; any other stem matches nothing.
	pub fn places_type(&self) -> &str {
		self.id.strip_prefix("gcid:").expect("checked by `Definition::check`")
	}
}

/// What is drawn: the outline grown by its margin. Cells and competitors are taken inside it.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
	pub bbox: Bbox,
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
}

impl Area {
	pub fn slug(&self) -> &str {
		match self {
			Self::City { slug, .. } | Self::Circle { slug, .. } => slug,
		}
	}

	pub fn country(&self) -> &str {
		match self {
			Self::City { country, .. } | Self::Circle { country, .. } => country,
		}
	}

	pub fn frame(&self) -> Frame {
		match *self {
			Self::City { viewport: v, margin_m, .. } => {
				let dlat = margin_m / M_PER_DEG;
				let dlon = margin_m / (M_PER_DEG * ((v.south + v.north) / 2.).to_radians().cos());
				Frame {
					bbox: Bbox {
						lat: [v.south - dlat, v.north + dlat],
						lon: [v.west - dlon, v.east + dlon],
					},
					circle: None,
				}
			}
			Self::Circle { center: c, radius_m, margin_m, .. } => {
				let r = radius_m + margin_m;
				let (dlat, dlon) = (r / M_PER_DEG, r / (M_PER_DEG * c.lat.to_radians().cos()));
				Frame {
					bbox: Bbox {
						lat: [c.lat - dlat, c.lat + dlat],
						lon: [c.lng - dlon, c.lng + dlon],
					},
					circle: Some((c, r)),
				}
			}
		}
	}

	/// [lat, lon] the outline is about.
	pub fn center(&self) -> [f64; 2] {
		match self {
			Self::City { viewport: v, .. } => [(v.south + v.north) / 2., (v.west + v.east) / 2.],
			Self::Circle { center, .. } => [center.lat, center.lng],
		}
	}

	/// The Web-Mercator zoom at which the frame fills a ~1000 px map.
	pub fn zoom(&self) -> u8 {
		let b = self.frame().bbox;
		let span = (b.lon[1] - b.lon[0]).max((b.lat[1] - b.lat[0]) / ((b.lat[0] + b.lat[1]) / 2.).to_radians().cos());
		(1000. / 256. * 360. / span).log2().floor().clamp(0., 18.) as u8
	}
}

impl Definition {
	/// Fails on the first field the conductor could not have handed in.
	pub fn check(&self) -> Result<()> {
		let a = &self.area;
		let slug = a.slug();
		ensure!(
			!slug.is_empty() && slug.len() <= 64 && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
			"area slug {slug:?} is not [a-z0-9-]{{1,64}}"
		);
		let iso = |c: &str| c.len() == 2 && c.bytes().all(|b| b.is_ascii_uppercase());
		ensure!(iso(a.country()), "area country {:?} is not ISO 3166-1 alpha-2", a.country());
		match a {
			Area::City { viewport: v, margin_m, .. } => {
				ensure!(v.south < v.north && v.west < v.east, "viewport {v:?} is not ordered south < north, west < east");
				ensure!(margin_m.is_finite() && *margin_m >= 0., "margin_m {margin_m} is not a distance");
			}
			Area::Circle { center, radius_m, margin_m, .. } => {
				ensure!(
					(-90. ..=90.).contains(&center.lat) && (-180. ..=180.).contains(&center.lng),
					"center {center:?} is not on the globe"
				);
				ensure!(radius_m.is_finite() && *radius_m > 0., "radius_m {radius_m} is not a distance");
				ensure!(margin_m.is_finite() && *margin_m >= 0., "margin_m {margin_m} is not a distance");
			}
		}
		if let Some(c) = &self.category {
			ensure!(c.country == a.country(), "category {}·{} applied to an area in {}", c.trade, c.country, a.country());
			ensure!(!c.study.is_empty() && c.study.trim() == c.study, "category {}·{} names study {:?}", c.trade, c.country, c.study);
			ensure!(
				(1..=50).contains(&c.keywords.len()),
				"category {}·{} carries {} keywords, not 1..=50",
				c.trade,
				c.country,
				c.keywords.len()
			);
			ensure!(
				c.google_additional.len() <= 9,
				"category {}·{} carries {} additional Google categories, not ≤ 9",
				c.trade,
				c.country,
				c.google_additional.len()
			);
			for g in std::iter::once(&c.google_primary).chain(&c.google_additional) {
				ensure!(g.id.starts_with("gcid:") && g.id.len() > 5, "Google category {:?} is not a gcid", g.id);
				ensure!(!g.name.trim().is_empty(), "Google category {} has no name", g.id);
			}
		}
		Ok(())
	}

	/// Keywords and additional categories are sets to a study, so their order is not part of what
	/// it is.
	pub fn canonical(mut self) -> Self {
		if let Some(c) = &mut self.category {
			c.keywords.sort();
			c.google_additional.sort();
		}
		self
	}

	/// SHA-256 of the canonical form, hex: the study's cache key, since one slug or one category can
	/// be redefined under the same name.
	pub fn id(&self) -> String {
		let bytes = serde_json::to_vec(&self.clone().canonical()).expect("a definition serialises");
		Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
	}

	/// `<study>·<CC>·<slug>`, or the slug alone without a category.
	pub fn name(&self) -> String {
		match &self.category {
			Some(c) => format!("{}·{}·{}", c.study, c.country, self.area.slug()),
			None => self.area.slug().to_owned(),
		}
	}
}
