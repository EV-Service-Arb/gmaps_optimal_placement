//! Competitor inventory: whoever is already selling the service in the area.
use std::time::{SystemTime, UNIX_EPOCH};

use eyre::{Result, WrapErr, bail, ensure};
use gmaps_optimal_placement_core::{Ask, Category, grid::Bbox, payload::Poi, trade_keyword};
use indexmap::IndexMap;

use crate::{
	Frame,
	work::{Kind, Work},
};

/// Enterprise SKU: `rating`, `websiteUri` and `nationalPhoneNumber` each pull the mask up a tier.
/// The map displays all three.
const FIELDS: &str = "places.id,places.displayName,places.formattedAddress,places.location,places.rating,places.userRatingCount,\
	places.types,places.primaryType,places.primaryTypeDisplayName,places.businessStatus,places.websiteUri,places.nationalPhoneNumber,nextPageToken";
/// Of twenty each: what one text search will hand back.
const PAGES: usize = 3;

/// A search's hits in rank order: each place's id, its answer, and when that answer was fetched.
type Hits = Vec<(String, serde_json::Value, SystemTime)>;

/// One tile of the grid every sweep asks: one Web-Mercator level for every frame, so an answer is about
/// a tile of ground rather than about whichever area asked, and two areas that overlap share what
/// either bought. What a search finds in one past its `PAGES` is not collected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Tile {
	x: u32,
	y: u32,
}
impl Tile {
	/// ~7 km a side at 45°N: an agglomeration is a handful of tiles, and a small circle one or four.
	const ZOOM: u32 = 12;

	/// Every tile that meets `bbox`.
	pub fn covering(bbox: Bbox) -> Vec<Self> {
		let n = f64::from(1u32 << Self::ZOOM);
		let x = |lon: f64| (((lon + 180.) / 360. * n).floor() as u32).min((1 << Self::ZOOM) - 1);
		let y = |lat: f64| {
			let r = lat.to_radians();
			(((1. - (r.tan() + 1. / r.cos()).ln() / std::f64::consts::PI) / 2. * n).floor() as u32).min((1 << Self::ZOOM) - 1)
		};
		let (x0, x1, y0, y1) = (x(bbox.lon[0]), x(bbox.lon[1]), y(bbox.lat[1]), y(bbox.lat[0]));
		(y0..=y1).flat_map(|y| (x0..=x1).map(move |x| Self { x, y })).collect()
	}

	/// Corners rounded to 1e-7°, about a centimetre: the bbox is the cache key, and `sinh`/`atan` may
	/// differ in the last bit between one libm and the next.
	pub fn bbox(self) -> Bbox {
		let n = f64::from(1u32 << Self::ZOOM);
		let r = |v: f64| (v * 1e7).round() / 1e7;
		let lon = |x: u32| r(f64::from(x) / n * 360. - 180.);
		let lat = |y: u32| r((std::f64::consts::PI * (1. - 2. * f64::from(y) / n)).sinh().atan().to_degrees());
		Bbox {
			lat: [lat(self.y + 1), lat(self.y)],
			lon: [lon(self.x), lon(self.x + 1)],
		}
	}
}

/// What the sweep asks, each with the index into [`tiers`] of the tier what it returns is filed under:
/// the trade's name and every keyword but the one that is that name, then each additional's name.
pub fn queries(c: &Category) -> Result<Vec<(String, usize)>> {
	let own = trade_keyword(&c.trade)?;
	let trade = std::iter::once(c.trade.name.clone())
		.chain(c.keywords.iter().map(|k| k.text.clone()).filter(|k| *k != own))
		.map(|q| (q, 0));
	let mut out: Vec<(String, usize)> = Vec::new();
	for (q, tier) in trade.chain(c.additional.iter().enumerate().map(|(i, a)| (a.google.name.clone(), i + 1))) {
		match out.iter().find(|(o, _)| *o == q) {
			None => out.push((q, tier)),
			Some((_, t)) => ensure!(*t == tier, "{q:?} is searched for tier {t} and again for tier {tier}, so what it finds cannot be told apart"),
		}
	}
	Ok(out)
}

/// Each tier's name and weight: the trade's at 1, then each additional's.
pub fn tiers(c: &Category) -> Result<Vec<(String, f64)>> {
	let out: Vec<(String, f64)> = std::iter::once((c.trade.name.clone(), 1.))
		.chain(c.additional.iter().map(|a| (a.google.name.clone(), a.weight)))
		.collect();
	for (i, (n, _)) in out.iter().enumerate() {
		ensure!(!out[..i].iter().any(|(o, _)| o == n), "two Google categories are named {n:?}, and a tier is known by its name");
	}
	Ok(out)
}

/// What Places returned for one query, in the order it ranked them. Both the tiled inventory sweep
/// and the probe produce these; the fit reads nothing else.
#[derive(Clone, Debug)]
pub struct Ranking {
	pub from: Region,
	pub term: String,
	pub ids: Vec<String>,
}

/// Where the search was asked from, which is what says who could have been returned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Region {
	/// `locationRestriction`: a hard filter with nobody standing in it. Distance is not defined,
	/// and a coefficient fitted on it would be measuring the tiling.
	Tile(Bbox),
	/// `locationBias`: a searcher at [lat, lon], and a radius the ranking may reach past.
	Node([f64; 2], f64),
}

/// The competitors, and every ordering the search that found them handed back.
pub struct Inventory {
	pub pois: Vec<Poi>,
	pub obs: Vec<Ranking>,
	/// The first page of each search this found no answer for, which is what the conductor buys next:
	/// whether another is behind it is known only once it is answered.
	pub asks: Vec<Ask>,
	/// Pages this was answered with: what a pull asks again, if every tile fills as it did.
	pub calls: usize,
}

/// The sweep, out of what the work dir and the queries cache hold. A pairing nobody ever harvested
/// yields nothing, which is how a refit reads evidence without turning into a purchase.
///
/// A place is a competitor of the first tier whose search found it ([`queries`]), unless its name
/// holds an excluded word or its primary type is an excluded one. Places shows no Business Profile
/// category, so which search found a place is all that says what it is filed under.
pub fn cached(category: &Category, frame: Frame, work: &Work) -> Result<Inventory> {
	let tiers = tiers(category)?;
	let queries = queries(category)?;
	let tiles = Tile::covering(frame.bbox);
	let start = work.calls(Kind::Inventory);
	// each place, when it was last seen, and the tier of the first search that found it
	let mut raw: IndexMap<String, (serde_json::Value, SystemTime, usize)> = IndexMap::new();
	let (mut obs, mut asks) = (Vec::new(), Vec::new());
	for (q, tier) in &queries {
		let (before, spent) = (raw.len(), work.calls(Kind::Inventory));
		for tile in &tiles {
			let bbox = tile.bbox();
			let (page, lacks) = search_text(work, &ask(q, bbox), FIELDS, PAGES, Kind::Inventory).wrap_err_with(|| format!("sweeping {q:?}"))?;
			asks.extend(lacks);
			// an ordering of nothing identifies nothing, and it is also what a tile that was never harvested looks like
			if page.is_empty() {
				continue;
			}
			obs.push(Ranking {
				from: Region::Tile(bbox),
				term: q.clone(),
				ids: page.iter().map(|(id, ..)| id.clone()).collect(),
			});
			// the freshest answer naming a place is what it is known as of
			for (id, p, at) in page {
				raw.entry(id).and_modify(|e| e.1 = e.1.max(at)).or_insert((p, at, *tier));
			}
		}
		// new, not returned: the queries overlap by design, and what the eighth one adds over the seven
		// before it is what says whether it earns its calls
		eprintln!("  {q:?}: {} new of {} calls, {} orderings", raw.len() - before, work.calls(Kind::Inventory) - spent, obs.len(),);
	}
	let calls = work.calls(Kind::Inventory) - start;
	if let Some(a) = work.age(Kind::Inventory) {
		eprintln!("places: served from cache, {a}");
		if a.stale {
			eprintln!("  past `age.inventory` — shops have opened and closed since. A pull re-asks, and costs a day's quota");
		}
	}
	let mut out = Vec::new();
	for (id, (p, at, tier)) in &raw {
		match p["businessStatus"].as_str() {
			None | Some("OPERATIONAL") => {}
			Some(_) => continue,
		}
		let Some(name) = p["displayName"]["text"].as_str() else {
			bail!("Places result {id} has no display name: {p}")
		};
		let primary = p["primaryType"].as_str().unwrap_or_default(); // Places omits it for a place it typed as nothing
		let folded = name.to_lowercase();
		let excluded = &category.excluded;
		if excluded.words.iter().any(|w| folded.contains(w.as_str())) || excluded.types.iter().any(|t| t == primary) {
			continue;
		}
		let (lat, lng) = (p["location"]["latitude"].as_f64(), p["location"]["longitude"].as_f64());
		let (Some(lat), Some(lng)) = (lat, lng) else {
			bail!("Places result {name:?} has no location: {p}")
		};
		// a tile reaches past the frame, and what it found there is someone else's map
		if !frame.contains([lat, lng]) {
			continue;
		}
		out.push(Poi {
			id: id.clone(),
			name: name.to_owned(),
			addr: p["formattedAddress"].as_str().unwrap_or_default().to_owned(),
			lat,
			lng,
			rating: p["rating"].as_f64(),
			n_rev: p["userRatingCount"].as_f64().unwrap_or(0.), // Places omits the count at zero reviews
			kind: primary.to_owned(),
			kind_label: p["primaryTypeDisplayName"]["text"].as_str().unwrap_or_default().to_owned(),
			web: p["websiteUri"].as_str().unwrap_or_default().to_owned(),
			tel: p["nationalPhoneNumber"].as_str().unwrap_or_default().to_owned(),
			tier: tiers[*tier].0.clone(),
			fetched_s: at.duration_since(UNIX_EPOCH).wrap_err("a cached answer is dated before 1970")?.as_secs(),
		});
	}
	out.sort_by(|a, b| b.n_rev.total_cmp(&a.n_rev));
	eprintln!(
		"places: {} raw over {} searches ({} lacking), {calls} calls, {} kept",
		raw.len(),
		obs.len(),
		asks.len(),
		out.len()
	);
	Ok(Inventory { pois: out, obs, asks, calls })
}

/// One text search, paged, in rank order, out of what the work dir's `Held` holds. The body and the
/// field mask are the caller's, because they are what decides the SKU.
///
/// The first page with no answer, or one fetched before [`Work::since`], ends the search: the hits
/// before it, and it.
pub(crate) fn search_text(work: &Work, body: &serde_json::Value, mask: &str, pages: usize, kind: Kind) -> Result<(Hits, Option<Ask>)> {
	let mut out = Vec::new();
	let mut token: Option<String> = None;
	for _ in 0..pages {
		let mut body = body.clone();
		if let Some(t) = &token {
			body["pageToken"] = serde_json::Value::String(t.clone());
		}
		body.sort_all_objects(); // the cache's key is this string: a consumer's `serde_json/preserve_order` must not move it
		let ask = Ask {
			mask: mask.to_owned(),
			body: serde_json::to_string(&body)?,
		};
		let Some((res, at)) = work.answer(&ask)? else {
			return Ok((out, Some(ask)));
		};
		work.record(kind, at);
		if let Some(e) = res.get("error") {
			bail!("Places text search {}: {e}", body["textQuery"]);
		}
		for p in res["places"].as_array().into_iter().flatten() {
			let id = p["id"].as_str().ok_or_else(|| eyre::eyre!("Places returned a result without an id: {p}"))?;
			out.push((id.to_owned(), p.clone(), at));
		}
		match res["nextPageToken"].as_str() {
			Some(t) => token = Some(t.to_owned()),
			None => break,
		}
	}
	Ok((out, None))
}

/// The first page of `query` over `tile`, which is also its cache key.
fn ask(query: &str, tile: Bbox) -> serde_json::Value {
	// the cache key is the body verbatim, so a tile's corners may not be re-associated
	let rect = serde_json::json!({
		"low":  {"latitude": tile.lat[0], "longitude": tile.lon[0]},
		"high": {"latitude": tile.lat[1], "longitude": tile.lon[1]},
	});
	serde_json::json!({"textQuery": query, "pageSize": 20, "locationRestriction": {"rectangle": rect}})
}
