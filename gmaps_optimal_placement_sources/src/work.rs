//! The work dir: bulk archives and API responses, so a rerun costs nothing. A billed response is
//! the conductor's queries cache's to keep (`sa_core::queries_cache`), so it is bought once wherever
//! this runs; the work dir mirrors what the cache served, and runs on from that mirror while the
//! cache is out of reach, until a response it does not hold is needed. A Places search is never
//! bought here: the conductor buys what a study lacks ([`sa_core::placement::Ask`]), and this reads it.
//!
//! Nothing here expires. An answer past the age its kind allows is still served and its age is
//! reported, because the alternative — refetching — spends a day of quota to learn the same thing,
//! and the quota is the scarce side. What the ages buy is a map that says how old it is.
use std::{
	cell::RefCell,
	fs,
	io::Read,
	path::{Path, PathBuf},
	time::{Duration, SystemTime},
};

use eyre::{Result, WrapErr, bail};
use sa_core::{
	prices::Item,
	queries_cache::{Claim, ClaimBody, Key, Put, Scheme, Stored, gop, under},
	usage::{Bill, Billed},
};
use sha2::{Digest, Sha256};

const YEAR: Duration = Duration::from_secs(365 * 86_400);
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
#[derive(Clone, Copy, Debug, Default)]
pub struct Age {
	pub inventory: Duration = Self::INVENTORY,
	pub ordering: Duration = Self::ORDERING,
	pub communes: Duration = Self::COMMUNES,
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

/// A run that buys nothing met keyword volumes the work dir has no answer for.
#[derive(Debug)]
pub struct Blocked {
	pub missing: usize,
}

/// Oldest and median age of the answers one run served for a kind.
pub struct Aged {
	pub oldest: Duration,
	pub median: Duration,
	/// The oldest is past what the config allows. Reported, never acted on.
	pub stale: bool,
}

/// Where the responses a run pays for are cached: the settings' `queries_cache`.
#[derive(Clone, Debug)]
pub enum QueriesCache {
	/// `local`: the work dir alone, shared with no other machine.
	Local,
	/// The conductor's queries cache at this URL (`<origin>/api/queries_cache`); the work dir
	/// mirrors it.
	Conductor(String),
}

pub struct Work {
	dir: PathBuf,
	cache: QueriesCache,
	/// Configured rather than `ureq::post`: the default turns a 4xx into an error before the body is
	/// read, and a provider's error body is the only thing that says what went wrong.
	agent: ureq::Agent,
	age: Age,
	tiling: crate::poi::Tiling,
	/// An answer written before this is as good as none: a pull's, which asks again what it is built from.
	since: Option<SystemTime>,
	/// Buy the keyword volumes the work dir has no answer for; off, every such question is refused.
	spend: bool,
	/// Age of every cached answer this run served, by kind.
	served: RefCell<Vec<(Kind, Duration)>>,
}
impl Work {
	pub fn at(dir: impl Into<PathBuf>, cache: QueriesCache) -> Self {
		Self {
			dir: dir.into(),
			cache,
			agent: ureq::Agent::config_builder().http_status_as_error(false).build().new_agent(),
			age: Age::default(),
			tiling: crate::poi::Tiling::default(),
			since: None,
			spend: false,
			served: RefCell::default(),
		}
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

	/// Answers written before `since` are lacking.
	pub fn since(mut self, since: Option<SystemTime>) -> Self {
		self.since = since;
		self
	}

	pub fn spend(mut self, yes: bool) -> Self {
		self.spend = yes;
		self
	}

	/// Whether an answer written `at` is as good as none.
	pub(crate) fn stale(&self, at: SystemTime) -> bool {
		self.since.is_some_and(|since| at < since)
	}

	pub fn path(&self) -> &Path {
		&self.dir
	}

	pub fn queries_cache(&self) -> &QueriesCache {
		&self.cache
	}

	/// Answers served for `kind`, paid or not — one per page, which is one unit of
	/// `SearchTextRequestPerDayPerProject` whatever the mask.
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

	/// An answer was served, and this is when it was written. Called by whoever knows what the call
	/// was about — the cache itself only sees a URL and a body.
	pub fn record(&self, kind: Kind, at: SystemTime) {
		// an mtime ahead of now is a clock that moved, and the answer is still not old
		let age = SystemTime::now().duration_since(at).unwrap_or(Duration::ZERO);
		self.served.borrow_mut().push((kind, age));
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

	/// A cached POST of keyword volumes, and when the answer was written. The key is the request
	/// itself, so editing a query refetches and rerunning does not. `refused` names what in a 200 is
	/// a refusal rather than an answer: one kept would answer for its request everywhere, forever.
	pub fn cached_post(
		&self,
		item: Item,
		url: &str,
		body: &serde_json::Value,
		headers: &[(&str, &str)],
		refused: &dyn Fn(&serde_json::Value) -> Option<String>,
	) -> Result<(serde_json::Value, SystemTime)> {
		if let Some(hit) = self.cached(url, body)?
			&& !self.stale(hit.1)
		{
			return Ok(hit);
		}
		assert_ne!(url, crate::poi::SEARCH_TEXT, "a Places search is the conductor's to buy");
		if !self.spend {
			return Err(Blocked { missing: 1 }.into());
		}
		let canonical = serde_json::to_string(body)?;
		let request = request(url, &canonical);
		let key = Key(Sha256::digest(request.as_bytes()).into());
		let dst = self.post_path(key)?;
		if let QueriesCache::Conductor(_) = self.cache {
			let fresh_since = self.since.map(jiff::Timestamp::try_from).transpose().wrap_err("a pull dated before 1970")?;
			loop {
				match self.ask_cache("POST", &format!("{}/claim", under(Scheme::Gop, key)), &serde_json::to_vec(&ClaimBody { fresh_since })?)? {
					(200, text) => match serde_json::from_str(&text).wrap_err("the queries cache's claim")? {
						Claim::Stored(kept) => return self.mirror(&dst, kept),
						Claim::Held { .. } => std::thread::sleep(Duration::from_millis(500)),
						Claim::Granted { .. } => break,
					},
					(status, text) => bail!("the queries cache refused a claim: {status}: {text}"),
				}
			}
		}
		let bought = self.buy(url, &canonical, headers, refused, &dst);
		if bought.is_err()
			&& let QueriesCache::Conductor(_) = self.cache
			&& let Err(e) = self.ask_cache("DELETE", &format!("{}/claim", under(Scheme::Gop, key)), &[])
		{
			// the buy's error is the one to tell; the claim lapses with its lease anyway
			eprintln!("releasing the queries cache's claim after a failed buy: {e:#}");
		}
		let (answer, text) = bought?;
		if let QueriesCache::Conductor(_) = self.cache {
			let put = serde_json::to_vec(&Put {
				answer: answer.clone(),
				request,
				bought: Some(Billed {
					item,
					bill: Bill::Ours,
					account: None,
				}),
			})?;
			match self.ask_cache("PUT", &under(Scheme::Gop, key), &put)? {
				(204, _) => {}
				(status, text) => bail!("the queries cache refused a paid answer: {status}: {text}"),
			}
		}
		// renamed into place, so an interrupted write never leaves a truncated answer for the next read
		// to choke on; and written only once the cache has it, or a rerun would read it here and never
		// tell the cache
		let part = dst.with_extension("part");
		fs::write(&part, &text)?;
		fs::rename(&part, &dst)?;
		Ok((answer, SystemTime::now()))
	}

	/// `canonical` POSTed to `url`: the answer, and its text. What is paid for and not an answer is
	/// kept beside `dst`.
	fn buy(&self, url: &str, canonical: &str, headers: &[(&str, &str)], refused: &dyn Fn(&serde_json::Value) -> Option<String>, dst: &Path) -> Result<(serde_json::Value, String)> {
		let mut req = self.agent.post(url).header("Content-Type", "application/json");
		for (k, v) in headers {
			req = req.header(*k, *v);
		}
		let mut res = req.send(canonical.as_bytes()).wrap_err_with(|| format!("POST {url}"))?;
		let status = res.status();
		let mut text = String::new();
		res.body_mut().as_reader().read_to_string(&mut text)?;
		if !status.is_success() {
			bail!("POST {url} -> {status}\n{}", text.trim());
		}
		// billed already, so kept beside the cache rather than in it, where it would answer for the request forever
		let kept = dst.with_extension("rejected");
		let json: serde_json::Value = match serde_json::from_str(&text) {
			Ok(j) => j,
			Err(e) => {
				fs::write(&kept, &text)?;
				bail!("POST {url} returned non-JSON ({e}); the paid response is kept at {}", kept.display());
			}
		};
		if let Some(why) = refused(&json) {
			fs::write(&kept, &text)?;
			bail!("{why}; the paid response is kept at {}", kept.display());
		}
		Ok((json, text))
	}

	/// The response to this exact request and when it was written, if it has been made before: out
	/// of the work dir, else (or when that one is [stale](Self::stale)) out of the queries cache,
	/// mirrored. Reading the probe's orderings back this way is what keeps a refit from turning into a
	/// purchase.
	pub fn cached(&self, url: &str, body: &serde_json::Value) -> Result<Option<(serde_json::Value, SystemTime)>> {
		let key = Key(Sha256::digest(request(url, &serde_json::to_string(body)?).as_bytes()).into());
		let dst = self.post_path(key)?;
		let mirrored = match fs::read_to_string(&dst) {
			Ok(s) => Some((serde_json::from_str(&s).wrap_err_with(|| format!("cached response {} is not JSON", dst.display()))?, mtime(&dst)?)),
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
			Err(e) => return Err(e).wrap_err_with(|| format!("reading {}", dst.display())),
		};
		if mirrored.as_ref().is_some_and(|(_, at)| !self.stale(*at)) {
			return Ok(mirrored);
		}
		if let QueriesCache::Local = self.cache {
			return Ok(mirrored);
		}
		match self.ask_cache("GET", &under(Scheme::Gop, key), &[])? {
			(200, text) => Ok(Some(self.mirror(&dst, serde_json::from_str(&text).wrap_err("the queries cache's answer")?)?)),
			// a mirror the cache never held: kept here before the cache was
			(404, _) => Ok(mirrored),
			(status, text) => bail!("the queries cache refused a read: {status}: {text}"),
		}
	}

	/// `kept` written to `dst` as of when it was fetched, so its age reads the same from either.
	fn mirror(&self, dst: &Path, kept: Stored) -> Result<(serde_json::Value, SystemTime)> {
		let at = SystemTime::from(kept.fetched_at);
		let part = dst.with_extension("part");
		fs::write(&part, kept.answer.to_string())?;
		fs::File::options().write(true).open(&part)?.set_modified(at)?;
		fs::rename(&part, dst)?;
		Ok((kept.answer, at))
	}

	/// A request `under` the conductor's queries cache: its status and body.
	fn ask_cache(&self, method: &str, under: &str, body: &[u8]) -> Result<(u16, String)> {
		let QueriesCache::Conductor(cache) = &self.cache else {
			unreachable!("asked only of a conductor's cache")
		};
		let url = format!("{cache}{under}");
		let req = ureq::http::Request::builder()
			.method(method)
			.uri(&url)
			.header("content-type", "application/json")
			.body(body.to_vec())?;
		let mut res = self.agent.run(req).wrap_err_with(|| format!("the queries cache at {cache} did not answer"))?;
		let text = res.body_mut().read_to_string().wrap_err_with(|| format!("{method} {url}: the answer was cut off"))?;
		Ok((res.status().as_u16(), text))
	}

	fn post_path(&self, key: Key) -> Result<PathBuf> {
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
}

/// What a request's key digests: [`gop`] for a Places search, the conductor's key for what it buys; the URL and body for keyword volumes, which only placement keys.
fn request(url: &str, canonical: &str) -> String {
	if url == crate::poi::SEARCH_TEXT { gop(canonical) } else { format!("{url}\n{canonical}") }
}
impl std::fmt::Display for Blocked {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let s = if self.missing == 1 { "" } else { "s" };
		write!(f, "at least {} paid answer{s} the work dir does not hold, and this run buys none", self.missing)
	}
}
impl std::error::Error for Blocked {}
impl std::fmt::Display for Aged {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "oldest {:.1} d, median {:.1} d", days(self.oldest), days(self.median))
	}
}
impl std::str::FromStr for QueriesCache {
	type Err = eyre::Report;

	fn from_str(s: &str) -> Result<Self> {
		match s.trim() {
			"local" => Ok(Self::Local),
			url if url.starts_with("http://") || url.starts_with("https://") => Ok(Self::Conductor(url.trim_end_matches('/').to_owned())),
			other => bail!("queries_cache is `local` or the conductor's `<origin>/api/queries_cache`, not {other:?}"),
		}
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

	/// Without `spend` a miss is refused before it reaches the network, and what is on disk is still served.
	#[test]
	fn a_blocked_run_serves_the_cache_and_buys_nothing() {
		const IDEAS: &str = "https://api.dataforseo.invalid/v3/keywords_data/google_ads/keywords_for_keywords/live";
		let item = Item::DataForSeo(sa_core::prices::DataForSeoTask::KeywordsForKeywords);
		let dir = std::env::temp_dir().join(format!("gmaps_no_spend_{}", std::process::id()));
		let work = Work::at(&dir, QueriesCache::Local);
		let (hit, miss) = (serde_json::json!({"keywords": ["kept"]}), serde_json::json!({"keywords": ["never asked"]}));
		let key = Key(Sha256::digest(gop(IDEAS, &serde_json::to_string(&hit).unwrap())).into());
		let dst = work.post_path(key).unwrap();
		fs::write(&dst, r#"{"tasks":[]}"#).unwrap();

		assert_eq!(work.cached_post(item, IDEAS, &hit, &[], &|_| None).unwrap().0, serde_json::json!({"tasks": []}));
		let e = work.cached_post(item, IDEAS, &miss, &[], &|_| None).unwrap_err();
		assert!(e.downcast_ref::<Blocked>().is_some(), "{e:#}");
		fs::remove_dir_all(&dir).unwrap();
	}
}
