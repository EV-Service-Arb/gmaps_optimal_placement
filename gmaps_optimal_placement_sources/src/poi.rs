//! Competitor inventory: whoever is already selling the service in the area.
use std::time::{SystemTime, UNIX_EPOCH};

use eyre::{Result, WrapErr, bail, ensure};
use gmaps_optimal_placement_core::{grid::Bbox, payload::Poi};
use indexmap::IndexMap;
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::work::{Kind, Need, Work};

pub const SEARCH_TEXT: &str = "https://places.googleapis.com/v1/places:searchText";
/// Enterprise SKU: `rating`, `websiteUri` and `nationalPhoneNumber` each pull the mask up a tier.
/// The map displays all three.
const FIELDS: &str = "places.id,places.displayName,places.formattedAddress,places.location,places.rating,places.userRatingCount,\
	places.types,places.primaryType,places.primaryTypeDisplayName,places.businessStatus,places.websiteUri,places.nationalPhoneNumber,nextPageToken";
/// Three pages of twenty is all one text search will hand back, so a tile that comes back at this
/// has more in it than it said.
const CAP: usize = 60;
/// Tiles a side the frame starts as. The whole frame saturates for any dense trade, so asking it
/// buys a chain of calls to learn that; answers from before the recursion are keyed on this grid.
const ROOT: u32 = 3;
/// How far a saturated tile may be quartered. Over a city frame the floor is a couple of
/// kilometres, and a query still at the cap down there is reported rather than passed off as the
/// whole inventory.
const DEPTH: u32 = 3;
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub enum PoiSource {
	#[serde(rename = "google_places")]
	GooglePlaces,
}

/// A name pattern, a set of source-native categories, or both — either one hits.
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MatchSpec {
	/// Case-insensitive regex over the display name.
	#[serde(default, rename = "match")]
	pub pattern: Option<String>,
	#[serde(default)]
	pub types: Vec<String>,
}
impl MatchSpec {
	fn compile(&self) -> Result<Matcher> {
		ensure!(self.pattern.is_some() || !self.types.is_empty(), "a match rule needs `match`, `types`, or both");
		let pattern = self
			.pattern
			.as_deref()
			.map(|p| Regex::new(&format!("(?i){p}")))
			.transpose()
			.wrap_err_with(|| format!("match pattern {:?}", self.pattern.as_deref().unwrap_or_default()))?;
		Ok(Matcher { pattern, types: self.types.clone() })
	}
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Tier {
	pub name: String,
	/// How much one of these counts against a new entrant, relative to a direct competitor.
	pub weight: f64,
	#[serde(flatten)]
	pub spec: MatchSpec,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PoiConfig {
	pub source: PoiSource,
	pub queries: Vec<String>,
	/// One Places *type*, which is a coarser taxonomy than the Business Profile category the map
	/// displays: there is no `house_cleaning_service`, and a third of the cleaning firms Google
	/// returns carry no primary type at all. Set, it is forced — `strictTypeFiltering` — so a tile
	/// saturates on competitors rather than on the retail the query drags in, at the price of every
	/// competitor Google never typed. Unset is the honest default; see `docs/ARCHITECTURE.md`.
	#[serde(default)]
	pub included_type: Option<String>,
	/// First match wins; anything matching none is not a competitor. `types` here hits any category
	/// the source lists — a supermarket forecourt is typed `gas_station` and carries `car_wash`.
	#[serde(rename = "tier")]
	pub tiers: Vec<Tier>,
	/// Checked before the tiers — text search drags in unrelated retail. `types` here hits only the
	/// source's *primary* category, because the full list is noisy enough to veto real competitors:
	/// Google tags several car washes `laundry`.
	pub drop: Option<MatchSpec>,
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
	/// (query, tile) pairs this walk found no answer for. A lower bound on what a keyed sweep would
	/// spend, because a tile that comes back at the cap opens four more. Zero once one has paid.
	pub missing: usize,
}

/// One text search, paged, in rank order. The body and the field mask are the caller's, because
/// they are what decides the SKU.
///
/// No key is the read-only mode: pages come out of the work dir and the first miss ends the search.
/// A caller holding no key cannot buy, which is what lets the fit replay a sweep rather than pay
/// for one.
pub fn search_text(work: &Work, key: Option<&str>, body: &serde_json::Value, mask: &str, pages: usize, kind: Kind) -> Result<Vec<(String, serde_json::Value, SystemTime)>> {
	let mut out = Vec::new();
	let mut token: Option<String> = None;
	for _ in 0..pages {
		let mut body = body.clone();
		if let Some(t) = &token {
			body["pageToken"] = serde_json::Value::String(t.clone());
		}
		let (res, at) = match key {
			Some(k) => work.cached_post(SEARCH_TEXT, &body, &[("X-Goog-Api-Key", k), ("X-Goog-FieldMask", mask)])?,
			None => match work.cached(SEARCH_TEXT, &body)? {
				Some(hit) => hit,
				None => break,
			},
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
	Ok(out)
}

pub fn load(cfg: &PoiConfig, bbox: Bbox, what: &str, work: &Work) -> Result<Inventory> {
	let key = std::env::var("GOOGLE_MAPS_KEY").wrap_err("GOOGLE_MAPS_KEY is not set")?;
	// the free walk first, so what this is about to buy is on screen before it buys it. It reads the
	// cache and nothing else, and a sweep is a tenth of a second against the grid scan behind it
	work.announce(what, Need::AtLeast(sweep(cfg, bbox, None, work)?.missing));
	work.forget(Kind::Inventory);
	sweep(cfg, bbox, Some(&key), work)
}

/// The same sweep, restricted to what the work dir already holds. A pairing nobody ever harvested
/// yields nothing, which is how a refit reads evidence without turning into a purchase.
pub fn cached(cfg: &PoiConfig, bbox: Bbox, work: &Work) -> Result<Inventory> {
	sweep(cfg, bbox, None, work)
}

fn sweep(cfg: &PoiConfig, bbox: Bbox, key: Option<&str>, work: &Work) -> Result<Inventory> {
	let PoiSource::GooglePlaces = cfg.source;
	let drop = cfg.drop.as_ref().map(MatchSpec::compile).transpose()?;
	let tiers: Vec<(&Tier, Matcher)> = cfg.tiers.iter().map(|t| Ok((t, t.spec.compile()?))).collect::<Result<_>>()?;
	ensure!(!tiers.is_empty(), "at least one [[poi.tier]] is needed to say what a competitor is");

	let mut found = Found::default();
	for q in &cfg.queries {
		let (before, spent) = (found.raw.len(), work.calls(Kind::Inventory));
		// every answer that arrived is already on disk and the recursion is deterministic, so whatever
		// stopped this — a daily quota above all — a rerun picks up where it stopped and pays only for
		// what is still missing
		let (dlat, dlon) = ((bbox.lat[1] - bbox.lat[0]) / f64::from(ROOT), (bbox.lon[1] - bbox.lon[0]) / f64::from(ROOT));
		for i in 0..ROOT {
			for j in 0..ROOT {
				// the cache key is the body verbatim, so these expressions may not be re-associated
				let (lat0, lon0) = (bbox.lat[0] + f64::from(i) * dlat, bbox.lon[0] + f64::from(j) * dlon);
				let (lat1, lon1) = (bbox.lat[0] + f64::from(i + 1) * dlat, bbox.lon[0] + f64::from(j + 1) * dlon);
				let tile = Bbox {
					lat: [lat0, lat1],
					lon: [lon0, lon1],
				};
				descend(work, key, q, cfg.included_type.as_deref(), tile, 0, &mut found)
					.wrap_err_with(|| format!("sweeping {q:?}: a rerun resumes from here, and re-asks nothing already answered"))?;
			}
		}
		// new, not returned: the queries overlap by design, and what the eighth one adds over the seven
		// before it is what says whether it earns its calls
		eprintln!(
			"  {q:?}: {} new of {} calls, {} orderings, {} billed so far",
			found.raw.len() - before,
			work.calls(Kind::Inventory) - spent,
			found.obs.len(),
			work.billed()
		);
	}
	let Found { raw, obs, censored, missing, calls } = found;
	if let Some(a) = work.age(Kind::Inventory) {
		eprintln!("places: served from cache, {a}");
		if a.stale {
			eprintln!("  past `age.inventory` — shops have opened and closed since. `--refresh` re-asks, and costs a day's quota");
		}
	}
	if !censored.is_empty() {
		eprintln!(
			"places: {} tiles are still at the {CAP}-result cap at depth {DEPTH}, so the inventory under them is partial:",
			censored.len()
		);
		for (q, t) in &censored {
			eprintln!("  {q:?} over lat {:?} lon {:?}", t.lat, t.lon);
		}
	}

	let mut out = Vec::new();
	for (id, (p, at)) in &raw {
		match p["businessStatus"].as_str() {
			None | Some("OPERATIONAL") => {}
			Some(_) => continue,
		}
		let Some(name) = p["displayName"]["text"].as_str() else {
			bail!("Places result {id} has no display name: {p}")
		};
		let primary = p["primaryType"].as_str().unwrap_or_default();
		let mut kinds: Vec<&str> = p["types"].as_array().into_iter().flatten().filter_map(serde_json::Value::as_str).collect();
		kinds.push(primary);
		if drop.as_ref().is_some_and(|d| d.hits(name, &[primary])) {
			continue;
		}
		let Some((tier, _)) = tiers.iter().find(|(_, m)| m.hits(name, &kinds)) else { continue };
		let (lat, lng) = (p["location"]["latitude"].as_f64(), p["location"]["longitude"].as_f64());
		let (Some(lat), Some(lng)) = (lat, lng) else {
			bail!("Places result {name:?} has no location: {p}")
		};
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
			tier: tier.name.clone(),
			fetched_s: at.duration_since(UNIX_EPOCH).wrap_err("a cached answer is dated before 1970")?.as_secs(),
		});
	}
	out.sort_by(|a, b| b.n_rev.total_cmp(&a.n_rev));
	eprintln!(
		"places: {} raw over {} searches ({} billed, {missing} not in the work dir), {} kept",
		raw.len(),
		obs.len(),
		work.billed(),
		out.len()
	);
	let (total, interior) = calls.iter().fold((0, 0), |(t, i), (c, q)| (t + c, i + q));
	eprintln!("places: {total} calls, {interior} on tiles that quartered and were re-covered by their four children:");
	for (d, (c, q)) in calls.iter().enumerate().filter(|(_, (c, _))| *c > 0) {
		eprintln!("  depth {d}: {c} calls, {q} of them interior");
	}
	Ok(Inventory { pois: out, obs, missing })
}

/// One query over one tile, quartered wherever the answer came back at the cap. Splitting is per
/// query and not once for the whole sweep: "lavage auto" saturates downtown where "covering
/// carrosserie" does not, and the quadrants of a tile nobody filled are not worth asking.
///
/// Not a study's to state: the cap bites on competitor density, which is a fact about the trade and
/// the ground together, and which no document knows before it asks.
fn descend(work: &Work, key: Option<&str>, query: &str, typed: Option<&str>, tile: Bbox, depth: u32, found: &mut Found) -> Result<()> {
	// the cache key is the body verbatim, so a tile's corners may not be re-associated
	let rect = serde_json::json!({
		"low":  {"latitude": tile.lat[0], "longitude": tile.lon[0]},
		"high": {"latitude": tile.lat[1], "longitude": tile.lon[1]},
	});
	let mut body = serde_json::json!({"textQuery": query, "pageSize": 20, "locationRestriction": {"rectangle": rect}});
	// absent rather than null when unset: the body is the cache key, and an untyped sweep already
	// paid for these answers. Places rejects a type outside Table A by name, which is the only list
	// of them worth keeping.
	if let Some(t) = typed {
		body["includedType"] = serde_json::Value::String(t.to_owned());
		body["strictTypeFiltering"] = serde_json::Value::Bool(true);
	}
	// asked here rather than inferred from an empty page: a tile nobody harvested and one Google
	// returned nothing for read the same out of `search_text`, and only the first would cost anything
	if work.refreshing() || work.cached(SEARCH_TEXT, &body)?.is_none() {
		found.missing += 1;
	}
	let spent = work.calls(Kind::Inventory);
	let page = search_text(work, key, &body, FIELDS, 3, Kind::Inventory)?;
	let paged = work.calls(Kind::Inventory) - spent;
	found.calls[depth as usize].0 += paged;
	// an ordering of nothing identifies nothing, and under a read-only key it is also what a tile
	// that was never harvested looks like
	if page.is_empty() {
		return Ok(());
	}
	found.obs.push(Ranking {
		from: Region::Tile(tile),
		term: query.to_owned(),
		ids: page.iter().map(|(id, ..)| id.clone()).collect(),
	});
	let full = page.len() >= CAP;
	// the freshest answer naming a place is what it is known as of
	for (id, p, at) in page {
		found.raw.entry(id).and_modify(|e| e.1 = e.1.max(at)).or_insert((p, at));
	}
	if !full {
		return Ok(());
	}
	if depth == DEPTH {
		found.censored.push((query.to_owned(), tile));
		return Ok(());
	}
	found.calls[depth as usize].1 += paged;
	let (mid_lat, mid_lon) = ((tile.lat[0] + tile.lat[1]) / 2., (tile.lon[0] + tile.lon[1]) / 2.);
	for lat in [[tile.lat[0], mid_lat], [mid_lat, tile.lat[1]]] {
		for lon in [[tile.lon[0], mid_lon], [mid_lon, tile.lon[1]]] {
			descend(work, key, query, typed, Bbox { lat, lon }, depth + 1, found)?;
		}
	}
	Ok(())
}

/// What the recursion accumulates, before the tiering rules turn it into an `Inventory`.
#[derive(Default)]
struct Found {
	raw: IndexMap<String, (serde_json::Value, SystemTime)>,
	obs: Vec<Ranking>,
	/// Query and tile still at the cap at the depth floor: the inventory under them is partial.
	censored: Vec<(String, Bbox)>,
	missing: usize,
	/// Per depth: calls made, and how many of those were on a tile that then quartered.
	calls: [(usize, usize); DEPTH as usize + 1],
}

struct Matcher {
	pattern: Option<Regex>,
	types: Vec<String>,
}

impl Matcher {
	fn hits(&self, name: &str, kinds: &[&str]) -> bool {
		self.pattern.as_ref().is_some_and(|r| r.is_match(name)) || kinds.iter().any(|k| self.types.iter().any(|t| t == k))
	}
}
