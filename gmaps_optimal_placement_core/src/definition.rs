//! A study as its host defines it: an area and a category, addressed by the SHA-256 of their
//! canonical form; and the searches a study lacks, which its host buys for a pull.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use crate::payload::Scale;

/// The one Places request a study is built from.
pub const SEARCH_TEXT: &str = "https://places.googleapis.com/v1/places:searchText";

/// A [`SEARCH_TEXT`] a study lacks: its host buys it, and keys it by [`SEARCH_TEXT`] and `body`.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Ask {
	/// `X-Goog-FieldMask`.
	pub mask: String,
	/// Sent verbatim: it is the key.
	pub body: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
	pub area: Area,
	/// None is the area on its own: the grid's layers, and not one billed call.
	pub category: Option<Category>,
}
impl Definition {
	/// Fails on the first field the conductor could not have handed in.
	pub fn check(&self) -> Result<(), Invalid> {
		let ensure = |ok: bool, why: String| if ok { Ok(()) } else { Err(Invalid(why)) };
		let a = &self.area;
		let slug = a.slug();
		ensure(
			!slug.is_empty() && slug.len() <= 64 && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
			format!("area slug {slug:?} is not [a-z0-9-]{{1,64}}"),
		)?;
		let distance = |m: f64| m.is_finite() && m >= 0.;
		match a {
			Area::City { viewport: v, margin_m, .. } => {
				ensure(v.south < v.north && v.west < v.east, format!("viewport {v:?} is not ordered south < north, west < east"))?;
				ensure(distance(*margin_m), format!("margin_m {margin_m} is not a distance"))?;
			}
			Area::Circle { center, radius_m, margin_m, .. } => {
				ensure(
					(-90. ..=90.).contains(&center.lat) && (-180. ..=180.).contains(&center.lng),
					format!("center {center:?} is not on the globe"),
				)?;
				ensure(distance(*radius_m) && *radius_m > 0., format!("radius_m {radius_m} is not a distance"))?;
				ensure(distance(*margin_m), format!("margin_m {margin_m} is not a distance"))?;
			}
		}
		let Some(c) = &self.category else { return Ok(()) };
		let named = format!("category {}·{}", c.name, c.country);
		ensure(c.country == a.country(), format!("{named} applied to an area in {}", a.country()))?;
		ensure(!c.name.trim().is_empty(), format!("trade {} has no name", c.trade))?;
		let googles: Vec<&Gcid> = std::iter::once(&c.primary).chain(c.additional.iter().map(|x| &x.google)).collect();
		ensure(googles.len() <= 10, format!("{named} carries {} additional Google categories, not ≤ 9", c.additional.len()))?;
		for (i, g) in googles.iter().enumerate() {
			ensure(g.id.starts_with("gcid:") && g.id.len() > 5, format!("Google category {:?} is not a gcid", g.id))?;
			ensure(!g.name.trim().is_empty(), format!("Google category {} has no name", g.id))?;
			ensure(!googles[..i].iter().any(|o| o.id == g.id), format!("{named} carries {} twice", g.id))?;
		}
		for x in &c.additional {
			ensure(weight(x.weight), format!("{named}'s {} weighs {}", x.google.id, x.weight))?;
		}
		weighed(&c.keywords)?;
		let primary = primary_keyword(&c.primary)?;
		ensure(
			c.keywords.iter().any(|k| k.text == primary && k.weight == Some(1.)),
			format!("{named} does not carry its primary's name {primary:?} as a keyword at weight 1"),
		)?;
		c.excluded.check()?;
		c.demand.as_ref().map_or(Ok(()), Demand::check)?;
		Ok(())
	}

	/// Keywords are a set to a study, so their order is not part of what it is; the additional
	/// categories' is, as first match wins.
	pub fn canonical(mut self) -> Self {
		if let Some(c) = &mut self.category {
			c.keywords.sort_by(|a, b| a.text.cmp(&b.text));
		}
		self
	}

	/// SHA-256 of the canonical form, hex: the study's address, since one slug or one category can
	/// be redefined under the same name.
	pub fn id(&self) -> String {
		let bytes = serde_json::to_vec(&self.clone().canonical()).expect("a definition serialises");
		Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
	}

	/// `<trade>·<CC>·<slug>`, or the slug alone without a category.
	pub fn name(&self) -> String {
		match &self.category {
			Some(c) => format!("{}·{}·{}", c.name, c.country, self.area.slug()),
			None => self.area.slug().to_owned(),
		}
	}
}

/// Keywords in [`keywords`]' form, each weight positive and finite.
pub fn weighed(k: &[Keyword]) -> Result<(), Invalid> {
	let texts: Vec<String> = k.iter().map(|k| k.text.clone()).collect();
	ensure(keywords(&texts)? == texts, "keywords are not as a category stores them".to_owned())?;
	for k in k {
		if let Some(w) = k.weight {
			ensure(weight(w), format!("the keyword {:?} weighs {w}", k.text))?;
		}
	}
	Ok(())
}

/// The primary's name as a keyword: the term every other is weighed against.
pub fn primary_keyword(primary: &Gcid) -> Result<String, Invalid> {
	Ok(keywords(std::slice::from_ref(&primary.name))?.remove(0))
}

/// An area as the conductor keeps it: a city's viewport or a circle, and a margin around it that
/// is still "next to it".
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Area {
	City {
		slug: String,
		country: Country,
		viewport: Viewport,
		margin_m: f64,
	},
	Circle {
		slug: String,
		country: Country,
		center: LatLng,
		radius_m: f64,
		margin_m: f64,
	},
}
impl Area {
	pub fn slug(&self) -> &str {
		match self {
			Self::City { slug, .. } | Self::Circle { slug, .. } => slug,
		}
	}

	pub fn country(&self) -> Country {
		match self {
			Self::City { country, .. } | Self::Circle { country, .. } => *country,
		}
	}
}

/// A trade set up for a country: everything a study of it reads, whichever area it is applied to.
/// Its competitors are tiered by which of its Google categories' searches found them, first match
/// wins (docs/ARCHITECTURE.md, Placement).
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
	pub trade: u64,
	/// The trade's name: what a study is called by.
	pub name: String,
	pub country: Country,
	/// Tier 1, what its searches return, and the weight of 1 the rest is weighed against: searched by
	/// its name and by each keyword.
	pub primary: Gcid,
	/// A tier each after the primary's, in this order: what the search of its exact name returns.
	pub additional: Vec<Additional>,
	/// [`keywords`]' form, the primary's name among them at weight 1.
	pub keywords: Vec<Keyword>,
	pub excluded: Excluded,
	/// None: no demand surface, and nothing that reads one.
	pub demand: Option<Demand>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Keyword {
	pub text: String,
	/// What ranking for it is worth against the primary's name; None: scanned, not scored.
	pub weight: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Additional {
	pub google: Gcid,
	/// A competitor of its tier against one of the primary's.
	pub weight: f64,
}

/// Places the sweep turns up that are never competitors, checked before any tier.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Excluded {
	/// Lowercase; a place whose name holds one.
	pub words: Vec<String>,
	/// Places types (`hardware_store`); a place whose primary type is one.
	pub types: Vec<String>,
}
impl Excluded {
	pub fn check(&self) -> Result<(), Invalid> {
		for (i, w) in self.words.iter().enumerate() {
			ensure(!w.is_empty() && *w == w.trim().to_lowercase(), format!("the excluded word {w:?} is not trimmed lowercase"))?;
			ensure(!self.words[..i].contains(w), format!("the word {w:?} is excluded twice"))?;
		}
		for (i, t) in self.types.iter().enumerate() {
			ensure(
				!t.is_empty() && t.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
				format!("the excluded type {t:?} is not a Places type"),
			)?;
			ensure(!self.types[..i].contains(t), format!("the type {t:?} is excluded twice"))?;
		}
		Ok(())
	}
}

/// What the country's grid means to the trade, per cell.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Demand {
	/// Over the grid's columns and `columns`.
	pub expr: String,
	/// Derived before `expr` reads them, in order.
	pub columns: Vec<Column>,
	/// Heat layers the trade adds to the country's.
	pub layers: Vec<Layer>,
}
impl Demand {
	pub fn check(&self) -> Result<(), Invalid> {
		ensure(!self.expr.trim().is_empty(), "the demand is empty".to_owned())?;
		for (i, col) in self.columns.iter().enumerate() {
			let ident = col.name.bytes().next().is_some_and(|b| b.is_ascii_lowercase()) && col.name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
			ensure(ident, format!("the column {:?} is not [a-z][a-z0-9_]*", col.name))?;
			ensure(!col.expr.trim().is_empty(), format!("the column {} is empty", col.name))?;
			ensure(!self.columns[..i].iter().any(|o| o.name == col.name), format!("the column {} is derived twice", col.name))?;
		}
		for (i, l) in self.layers.iter().enumerate() {
			ensure(
				!l.name.trim().is_empty() && !l.expr.trim().is_empty(),
				format!("the layer {:?} has no name or no expression", l.name),
			)?;
			ensure(!self.layers[..i].iter().any(|o| o.name == l.name), format!("the layer {} is added twice", l.name))?;
		}
		Ok(())
	}
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
	pub name: String,
	pub expr: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
	pub name: String,
	pub expr: String,
	pub scale: Scale,
	pub note: String,
}

/// One of Google's Business Profile categories, named in the country's language.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Gcid {
	pub id: String,
	pub name: String,
}

/// ISO 3166-1 alpha-2, of the countries a base is compiled in for.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, strum::Display, strum::EnumString, strum::VariantArray)]
#[schemars(rename = "BaseCountry")]
pub enum Country {
	FR,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct LatLng {
	pub lat: f64,
	pub lng: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
pub struct Viewport {
	pub south: f64,
	pub west: f64,
	pub north: f64,
	pub east: f64,
}

/// A definition its host could not have handed in.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Invalid(pub String);
impl std::fmt::Display for Invalid {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str(&self.0)
	}
}
impl std::error::Error for Invalid {}

/// A category's keywords as stored: each trimmed, its inner whitespace one space, lowercased, at
/// most 80 chars; at most 50, none twice. A keyword names no city: the grid point is the location,
/// and a city term is one location's own.
pub fn keywords(raw: &[String]) -> Result<Vec<String>, Invalid> {
	let mut out: Vec<String> = Vec::with_capacity(raw.len());
	for typed in raw {
		let k = typed.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
		ensure(!k.is_empty(), "a keyword is not empty".to_owned())?;
		ensure(k.chars().count() <= 80, format!("{k:?} is over 80 characters"))?;
		ensure(!out.contains(&k), format!("{typed:?} is the keyword {k:?} twice"))?;
		out.push(k);
	}
	ensure(out.len() <= 50, format!("{} keywords: a category takes 50", out.len()))?;
	Ok(out)
}

fn weight(w: f64) -> bool {
	w.is_finite() && w > 0.
}

fn ensure(ok: bool, why: String) -> Result<(), Invalid> {
	if ok { Ok(()) } else { Err(Invalid(why)) }
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A study's address is its definition's: one that moves is every work dir's studies keyed anew.
	#[test]
	fn a_study_keeps_its_address() {
		let def = Definition {
			area: Area::City {
				slug: "lyon".into(),
				country: Country::FR,
				viewport: Viewport {
					south: 45.62,
					west: 4.7,
					north: 45.9,
					east: 5.02,
				},
				margin_m: 0.,
			},
			category: Some(Category {
				trade: 1,
				name: "Plumbing".into(),
				country: Country::FR,
				primary: Gcid {
					id: "gcid:plumber".into(),
					name: "Plombier".into(),
				},
				additional: vec![],
				keywords: vec![
					Keyword {
						text: "plombier".into(),
						weight: Some(1.),
					},
					Keyword {
						text: "fuite d'eau".into(),
						weight: None,
					},
				],
				excluded: Excluded::default(),
				demand: None,
			}),
		};
		def.check().unwrap();
		assert_eq!(def.id(), PINNED);
	}

	const PINNED: &str = "f335012cc483774064b9050692a54b57fd76b40d109d7009f74c2deae9bfcf55";
}
