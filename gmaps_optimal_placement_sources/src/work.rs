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
	path::{Path, PathBuf},
	time::{Duration, SystemTime},
};

use eyre::{Result, WrapErr, bail};
use sa_core::queries_cache::{Key, Scheme, Stored, gop, under};
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
	/// Configured rather than `ureq::get`: the default turns a 4xx into an error before the body is
	/// read, and the queries cache's error body is the only thing that says what went wrong.
	agent: ureq::Agent,
	age: Age,
	tiling: crate::poi::Tiling,
	/// An answer written before this is as good as none: a pull's, which asks again what it is built from.
	since: Option<SystemTime>,
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

	/// The answer to this exact Places search and when it was written, if the conductor has bought it:
	/// out of the work dir, else (or when that one is [stale](Self::stale)) out of the queries cache,
	/// mirrored. Reading the probe's orderings back this way is what keeps a refit from turning into a
	/// purchase.
	pub fn cached(&self, body: &serde_json::Value) -> Result<Option<(serde_json::Value, SystemTime)>> {
		let key = Key(Sha256::digest(gop(&serde_json::to_string(body)?).as_bytes()).into());
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
