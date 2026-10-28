//! The work dir: bulk archives and API responses, so a rerun costs nothing.
//!
//! Nothing here expires. An answer past the age its kind allows is still served and its age is
//! reported, because the alternative — refetching — spends a day of quota to learn the same thing,
//! and the quota is the scarce side. What the ages buy is a map that says how old it is.
//!
//! Nothing here counts the quota either. `SearchTextRequestPerDayPerProject` is not readable with an
//! API key, so a local tally is a guess about a number two other runs and yesterday's clock all move
//! — and a guess that refuses is a guess that stops work Google would have served. What a sweep is
//! about to ask for is stated before it asks, out of what the cache is missing; what is actually
//! left is Google's to say, and it says it in the 429.
use std::{
	cell::{Cell, RefCell},
	fs,
	io::Read,
	path::{Path, PathBuf},
	time::{Duration, SystemTime},
};

use eyre::{Result, WrapErr, bail};
use sha2::{Digest, Sha256};

/// What an answer is about, which is what says how old it may get before the map says so.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
	/// Shop names, ratings, addresses. Shops open and close.
	Inventory,
	/// Who ranks where from each stratum — what `core::rank::COEF` is fitted on.
	Ordering,
	/// The commune list. Drifts only on mergers.
	Communes,
}

/// How old an answer of each kind may get before it is reported as old.
#[derive(Clone, Copy, Debug)]
pub struct Age {
	pub inventory: Duration,
	pub ordering: Duration,
	pub communes: Duration,
}
impl Age {
	pub const COMMUNES: Duration = YEAR;
	pub const INVENTORY: Duration = Duration::from_secs(7 * 86_400);
	pub const ORDERING: Duration = YEAR;

	fn of(&self, kind: Kind) -> Duration {
		match kind {
			Kind::Inventory => self.inventory,
			Kind::Ordering => self.ordering,
			Kind::Communes => self.communes,
		}
	}
}
impl Default for Age {
	fn default() -> Self {
		Self {
			inventory: Self::INVENTORY,
			ordering: Self::ORDERING,
			communes: Self::COMMUNES,
		}
	}
}
const YEAR: Duration = Duration::from_secs(365 * 86_400);
/// The one quota a sweep runs out of.
const QUOTA: &str = "SearchTextRequestPerDayPerProject";

/// Why a key can buy no more searches today. An API key cannot read either state ahead of time —
/// `serviceusage` answers `API_KEY_SERVICE_BLOCKED` and `monitoring` refuses keys outright — so a
/// refusal is the only place either is ever visible.
#[derive(Clone, Debug)]
pub enum Spent {
	/// The project's `SearchTextRequestPerDayPerProject`, as the 429 spelled it.
	Quota {
		limit: String,
		/// Unix seconds the window rolls over at.
		reset: Option<u64>,
	},
	/// The project has no billing, which is what an ended trial looks like.
	Billing,
}
impl std::fmt::Display for Spent {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Quota { limit, reset } => {
				write!(f, "its project's {limit} searches a day are used")?;
				if let Some(r) = reset {
					write!(f, ", until {:02}:{:02} UTC", r % 86_400 / 3600, r % 3600 / 60)?;
				}
				Ok(())
			}
			Self::Billing => write!(f, "its project's billing is disabled"),
		}
	}
}

/// Every configured key, spent. The one refusal this tool expects to meet.
///
/// Its own type because a caller painting a map has somewhere to go from here and a caller buying an
/// inventory does not, and the two cannot tell a spent day from a malformed query through a string.
#[derive(Debug)]
pub struct Exhausted(pub Vec<Spent>);
impl std::fmt::Display for Exhausted {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let why: Vec<String> = self.0.iter().map(ToString::to_string).collect();
		write!(
			f,
			"the day's Google Places searches are spent on every configured key ({}). Nothing already answered is re-asked, so a rerun pays only for the rest",
			why.join("; ")
		)
	}
}
impl std::error::Error for Exhausted {}

/// `--block-spend` met a Places search the work dir has no answer for. A refusal of the same shape as
/// [`Exhausted`], so a map opens on the grid for either.
#[derive(Debug)]
pub struct Blocked;
impl std::fmt::Display for Blocked {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "`--block-spend` refuses Places searches the work dir has no answer for")
	}
}
impl std::error::Error for Blocked {}

/// Oldest and median age of the answers one run served for a kind.
pub struct Aged {
	pub oldest: Duration,
	pub median: Duration,
	/// The oldest is past what the config allows. Reported, never acted on.
	pub stale: bool,
}
impl std::fmt::Display for Aged {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "oldest {:.1} d, median {:.1} d", days(self.oldest), days(self.median))
	}
}

/// How many searches something is about to ask for. A sweep cannot say exactly — a tile at the
/// result cap opens four more — so the two cases are distinguished rather than averaged into a
/// number that reads as certain.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Need {
	Exact(usize),
	AtLeast(usize),
}
impl std::fmt::Display for Need {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::Exact(n) => write!(f, "{n}"),
			Self::AtLeast(n) => write!(f, "at least {n}"),
		}
	}
}

pub struct Work {
	dir: PathBuf,
	billed: Cell<usize>,
	/// Configured rather than `ureq::post`: the default turns a 4xx into an error before the body is
	/// read, and a Google error body is the only thing that says which quota ran out.
	agent: ureq::Agent,
	age: Age,
	tiling: crate::poi::Tiling,
	/// Re-ask every billed request and overwrite what it answered. The only thing that spends on a
	/// question already answered; age alone never does.
	refresh: bool,
	/// Refuse every Places search the work dir has no answer for.
	block_spend: bool,
	/// Age of every cached answer this run served, by kind.
	served: RefCell<Vec<(Kind, Duration)>>,
	/// Google Maps keys in the order they are spent, each its own project's quota and billing.
	keys: Vec<String>,
	/// Why each of the first `len` keys can buy no more. Per `Work`, so the next run starts over on
	/// the most preferred key: a 429 is not billed, and quota comes back the next day.
	spent: RefCell<Vec<Spent>>,
}

impl Work {
	pub fn at(dir: impl Into<PathBuf>) -> Self {
		Self {
			dir: dir.into(),
			billed: Cell::new(0),
			agent: ureq::Agent::config_builder().http_status_as_error(false).build().new_agent(),
			age: Age::default(),
			tiling: crate::poi::Tiling::default(),
			refresh: false,
			block_spend: false,
			served: RefCell::default(),
			keys: Vec::new(),
			spent: RefCell::default(),
		}
	}

	pub fn keys(mut self, keys: Vec<String>) -> Self {
		assert!(
			keys.iter().enumerate().all(|(i, k)| !keys[..i].contains(k)),
			"a key listed twice is one project's quota counted twice"
		);
		self.keys = keys;
		self
	}

	/// The policy the CLI resolved out of its config. Plain values rather than the config type, so
	/// `_sources` stands alone and the wasm side never sees the settings crate.
	pub fn policy(mut self, age: Age, tiling: crate::poi::Tiling) -> Self {
		self.age = age;
		self.tiling = tiling;
		self
	}

	/// Which tiles the inventory sweep asks for, which is which answers in this dir are the ones asked for.
	pub(crate) fn tiling(&self) -> crate::poi::Tiling {
		self.tiling
	}

	pub fn refresh(mut self, yes: bool) -> Self {
		self.refresh = yes;
		self
	}

	pub fn block_spend(mut self, yes: bool) -> Self {
		self.block_spend = yes;
		self
	}

	/// Whether an answer already on disk will be bought again — which is what a preflight has to
	/// count, and the read-only walk that counts it is deliberately not affected.
	pub fn refreshing(&self) -> bool {
		self.refresh
	}

	pub fn path(&self) -> &Path {
		&self.dir
	}

	/// Cache misses since this `Work` was made — the calls a run actually paid for.
	pub fn billed(&self) -> usize {
		self.billed.get()
	}

	/// Answers served for `kind`, paid or not — one per page, which is one unit of
	/// `SearchTextRequestPerDayPerProject` whatever the mask. `billed` counts only the misses, so
	/// this is what a keyless walk can report and that one cannot.
	pub(crate) fn calls(&self, kind: Kind) -> usize {
		self.served.borrow().iter().filter(|(k, _)| *k == kind).count()
	}

	/// What was served out of the cache for `kind`, and whether the oldest is past its age. `None`
	/// when this run served nothing of that kind.
	pub fn age(&self, kind: Kind) -> Option<Aged> {
		let mut seen: Vec<Duration> = self.served.borrow().iter().filter(|(k, _)| *k == kind).map(|(_, d)| *d).collect();
		if seen.is_empty() {
			return None;
		}
		seen.sort_unstable();
		let oldest = seen[seen.len() - 1];
		Some(Aged {
			oldest,
			median: seen[seen.len() / 2],
			stale: oldest > self.age.of(kind),
		})
	}

	/// Drop what has been recorded for `kind`. For a walk that read the cache to price a purchase
	/// rather than to answer with it: what it served is not what the map is painted from.
	pub fn forget(&self, kind: Kind) {
		self.served.borrow_mut().retain(|(k, _)| *k != kind);
	}

	/// An answer was served, and this is when it was written. Called by whoever knows what the call
	/// was about — the cache itself only sees a URL and a body.
	pub fn record(&self, kind: Kind, at: SystemTime) {
		// an mtime ahead of now is a clock that moved, and the answer is still not old
		let age = SystemTime::now().duration_since(at).unwrap_or(Duration::ZERO);
		self.served.borrow_mut().push((kind, age));
	}

	/// What this is about to ask Google for, said before it asks. A report and never a refusal:
	/// what is left of the quota is not knowable from here, and a sweep stopped by a guess costs
	/// exactly as much as one Google stopped, minus what it would have found.
	pub(crate) fn announce(&self, what: &str, need: Need) {
		match need {
			Need::Exact(0) | Need::AtLeast(0) => eprintln!("{what}: answered in full from the work dir, nothing to buy"),
			_ if self.block_spend => eprintln!("{what}: {need} searches are not in the work dir, and `--block-spend` refuses them"),
			_ => eprintln!("{what}: {need} searches are not in the work dir and will be bought"),
		}
	}

	/// A path under the work dir for something derived from what is already there, named by a key its
	/// inputs decide. Nothing here expires either: a key covers everything the answer depends on, so a
	/// changed input is a different file rather than a stale one.
	pub(crate) fn derived(&self, sub: &str, name: &str) -> Result<PathBuf> {
		Ok(self.dir(sub)?.join(name))
	}

	/// The name of such a file. Twelve bytes: this is a cache key and not a signature.
	pub(crate) fn digest(parts: &[&str]) -> String {
		let mut h = Sha256::new();
		for p in parts {
			h.update(p.as_bytes());
			h.update([0]);
		}
		hex(&h.finalize()[..12])
	}

	fn dir(&self, sub: &str) -> Result<PathBuf> {
		let p = self.dir.join(sub);
		fs::create_dir_all(&p).wrap_err_with(|| format!("creating {}", p.display()))?;
		Ok(p)
	}

	/// A bulk archive, downloaded once. Named after the URL's last segment so the file on disk says
	/// what it is.
	///
	/// No age: the vintage is in the URL, so re-fetching would return the same bytes, and asking for
	/// a different vintage asks for a different file.
	pub fn archive(&self, url: &str) -> Result<PathBuf> {
		let name = url.rsplit('/').next().filter(|s| !s.is_empty()).ok_or_else(|| eyre::eyre!("no filename in {url}"))?;
		let dst = self.dir("data")?.join(name);
		if dst.exists() {
			return Ok(dst);
		}
		eprintln!("downloading {url}");
		let mut body = ureq::get(url).call().wrap_err_with(|| format!("GET {url}"))?.into_body().into_reader();
		let part = dst.with_extension("part");
		let mut out = fs::File::create(&part).wrap_err_with(|| format!("creating {}", part.display()))?;
		let n = std::io::copy(&mut body, &mut out)?;
		if n == 0 {
			bail!("{url} returned an empty body");
		}
		fs::rename(&part, &dst)?;
		eprintln!("  {} ({:.0} MB)", dst.display(), n as f64 / 1e6);
		Ok(dst)
	}

	/// A cached POST, and when the answer was written. The key is the request itself, so editing a
	/// query refetches and rerunning does not.
	///
	/// The key covers url and body but not headers, and a Places field mask is a header that changes
	/// the response shape. Two callers asking the same body under different masks would share one
	/// entry; today they do not, because they differ in the body as well.
	pub fn cached_post(&self, url: &str, body: &serde_json::Value, headers: &[(&str, &str)]) -> Result<(serde_json::Value, SystemTime)> {
		if !self.refresh
			&& let Some(hit) = self.cached(url, body)?
		{
			return Ok(hit);
		}
		if self.block_spend && url == crate::poi::SEARCH_TEXT {
			return Err(Blocked.into());
		}
		let canonical = serde_json::to_string(body)?;
		let dst = self.post_path(url, &canonical)?;
		let places = url == crate::poi::SEARCH_TEXT;
		let (status, text) = loop {
			let mut req = self.agent.post(url).header("Content-Type", "application/json");
			for (k, v) in headers {
				req = req.header(*k, *v);
			}
			if places {
				let at = self.spent.borrow().len();
				eyre::ensure!(!self.keys.is_empty(), "a Places search the work dir has no answer for needs a key in the config's `google_maps`");
				let Some(key) = self.keys.get(at) else {
					return Err(Exhausted(self.spent.borrow().clone()).into());
				};
				req = req.header("X-Goog-Api-Key", key);
			}
			let mut res = req.send(canonical.as_bytes()).wrap_err_with(|| format!("POST {url}"))?;
			let status = res.status();
			let mut text = String::new();
			res.body_mut().as_reader().read_to_string(&mut text)?;
			// the body of a refusal says which quota or which field, and it is never written to the cache:
			// one cached 429 would answer for that request forever
			if !status.is_success()
				&& places && let Some(why) = Self::spent(&text)
			{
				let mut spent = self.spent.borrow_mut();
				let tail = |k: &str| format!("…{}", &k[k.len().saturating_sub(4)..]);
				match self.keys.get(spent.len() + 1) {
					Some(next) => eprintln!("key {} spent: {why}; moving to {}", tail(&self.keys[spent.len()]), tail(next)),
					None => eprintln!("key {} spent: {why}; no key left", tail(&self.keys[spent.len()])),
				}
				spent.push(why);
				continue;
			}
			break (status, text);
		};
		if !status.is_success() {
			bail!("POST {url} -> {status}\n{}", text.trim());
		}
		self.billed.set(self.billed.get() + 1);
		let json: serde_json::Value = match serde_json::from_str(&text) {
			Ok(j) => j,
			Err(e) => {
				// billed already: kept beside the cache rather than in it, where it would answer for the request forever
				let kept = dst.with_extension("rejected");
				fs::write(&kept, &text)?;
				bail!("POST {url} returned non-JSON ({e}); the paid response is kept at {}", kept.display());
			}
		};
		// renamed into place, so an interrupted write never leaves a truncated answer for the next read to choke on
		let part = dst.with_extension("part");
		fs::write(&part, &text)?;
		fs::rename(&part, &dst)?;
		Ok((json, SystemTime::now()))
	}

	/// The response to this exact request and when it was written, if it has been made before.
	/// Reading the probe's orderings back this way is what keeps a refit from turning into a
	/// purchase.
	pub fn cached(&self, url: &str, body: &serde_json::Value) -> Result<Option<(serde_json::Value, SystemTime)>> {
		let dst = self.post_path(url, &serde_json::to_string(body)?)?;
		let Ok(s) = fs::read_to_string(&dst) else { return Ok(None) };
		let json = serde_json::from_str(&s).wrap_err_with(|| format!("cached response {} is not JSON", dst.display()))?;
		Ok(Some((json, mtime(&dst)?)))
	}

	fn post_path(&self, url: &str, canonical: &str) -> Result<PathBuf> {
		let key = hex(Sha256::digest(format!("{url}\n{canonical}").as_bytes()).as_slice());
		Ok(self.dir("data/places_cache")?.join(format!("{key}.json")))
	}

	/// A cached GET, and when the answer was written.
	pub fn cached_get(&self, url: &str, dst: &str) -> Result<(serde_json::Value, SystemTime)> {
		let dst = self.dir("data")?.join(dst);
		if let Ok(s) = fs::read_to_string(&dst) {
			let json = serde_json::from_str(&s).wrap_err_with(|| format!("cached response {} is not JSON", dst.display()))?;
			return Ok((json, mtime(&dst)?));
		}
		let text = ureq::get(url).call().wrap_err_with(|| format!("GET {url}"))?.into_body().read_to_string()?;
		let json = serde_json::from_str(&text).wrap_err_with(|| format!("GET {url} returned non-JSON: {}", &text[..text.len().min(400)]))?;
		fs::write(&dst, &text)?;
		Ok((json, SystemTime::now()))
	}

	/// What a refusal says about the key, if it says the key is done for the day. Anything else — a
	/// malformed query, a key with the wrong referrer, a per-minute limit — is `None` and travels as
	/// the body it arrived as.
	fn spent(body: &str) -> Option<Spent> {
		let v = serde_json::from_str::<serde_json::Value>(body).ok()?;
		let details = v["error"]["details"].as_array()?;
		if details.iter().any(|d| d["reason"].as_str() == Some("BILLING_DISABLED")) {
			return Some(Spent::Billing);
		}
		let m = details.iter().map(|d| &d["metadata"]).find(|m| m["quota_limit"].as_str() == Some(QUOTA))?;
		Some(Spent::Quota {
			limit: m["quota_limit_value"].as_str().unwrap_or("?").to_owned(),
			reset: m["window_start_time"].as_str().and_then(|s| s.parse::<u64>().ok()).map(|start| start + 86_400),
		})
	}
}

fn mtime(p: &Path) -> Result<SystemTime> {
	fs::metadata(p)?.modified().wrap_err_with(|| format!("{} has no mtime", p.display()))
}

fn days(d: Duration) -> f64 {
	d.as_secs_f64() / 86_400.
}

fn hex(bytes: &[u8]) -> String {
	bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The body a spent window actually returns, trimmed to the detail the message reads.
	const REFUSAL: &str = r#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","details":[
		{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"RATE_LIMIT_EXCEEDED","metadata":{
			"window_start_time":"1789369200","quota_limit_value":"100",
			"quota_metric":"places.googleapis.com/SearchTextRequest",
			"quota_limit":"SearchTextRequestPerDayPerProject"}}]}}"#;

	/// What an ended trial returns.
	const NO_BILLING: &str = r#"{"error":{"code":403,"status":"PERMISSION_DENIED","details":[
		{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"BILLING_DISABLED","domain":"googleapis.com",
			"metadata":{"consumer":"projects/1","service":"places.googleapis.com"}}]}}"#;

	/// A refusal moves to the next key only when it says this key is done for the day: rotating on a
	/// bad key or a bad query would spend the next project's quota on the same mistake. A malformed
	/// refusal must not take the process down on the way to reporting the real error.
	#[test]
	fn a_refusal_is_read_for_what_it_says() {
		let Some(Spent::Quota { limit, reset }) = Work::spent(REFUSAL) else {
			panic!("the refusal names the daily search quota")
		};
		assert_eq!(limit, "100");
		assert_eq!(reset, Some(1789369200 + 86_400));
		assert!(matches!(Work::spent(NO_BILLING), Some(Spent::Billing)));
		assert!(Work::spent(r#"{"error":{"code":400,"status":"INVALID_ARGUMENT","details":[{"reason":"API_KEY_INVALID"}]}}"#).is_none());
		assert!(Work::spent("not json at all").is_none());
		assert!(Work::spent(r#"{"error":{}}"#).is_none());
		assert!(Work::spent(r#"{"error":{"details":[{"metadata":{"quota_limit":"PlacesPerMinute"}}]}}"#).is_none());
	}

	/// A sweep wraps the refusal in a query and a tile on the way up, and a map falls back to the
	/// grid on exactly this downcast: lose it and the page errors out instead.
	#[test]
	fn a_spent_day_is_still_itself_under_the_context_a_sweep_adds() {
		let e = Err::<(), _>(eyre::Report::from(Exhausted(vec![Work::spent(REFUSAL).unwrap(), Spent::Billing])))
			.wrap_err("sweeping \"plombier\"")
			.wrap_err("plumbing in Lyon")
			.unwrap_err();
		assert!(e.downcast_ref::<Exhausted>().is_some());
		assert!(format!("{e:#}").contains("spent"), "the chain carries what the day was refused for: {e:#}");
	}

	/// Under `--block-spend` a Places miss is refused before it reaches the network, and what is on disk is still served.
	#[test]
	fn a_blocked_run_serves_the_cache_and_buys_nothing() {
		let dir = std::env::temp_dir().join(format!("gmaps_block_spend_{}", std::process::id()));
		let work = Work::at(&dir).block_spend(true);
		let (hit, miss) = (serde_json::json!({"textQuery": "kept"}), serde_json::json!({"textQuery": "never asked"}));
		let dst = work.post_path(crate::poi::SEARCH_TEXT, &serde_json::to_string(&hit).unwrap()).unwrap();
		fs::write(&dst, r#"{"places":[]}"#).unwrap();

		assert_eq!(work.cached_post(crate::poi::SEARCH_TEXT, &hit, &[]).unwrap().0, serde_json::json!({"places": []}));
		let e = work.cached_post(crate::poi::SEARCH_TEXT, &miss, &[]).unwrap_err();
		assert!(e.downcast_ref::<Blocked>().is_some(), "{e:#}");
		assert_eq!(work.billed(), 0);
		fs::remove_dir_all(&dir).unwrap();
	}
}
