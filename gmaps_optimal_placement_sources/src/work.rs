//! The work dir: bulk archives and what is derived from them, so a rerun costs nothing, and the
//! studies built over it. A billed answer is never fetched here: it is what the host [`Held`] for an
//! [`Ask`], bought by whoever buys.
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
use gmaps_optimal_placement_core::Ask;
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

/// The billed answers a study is built from, as its host holds them.
pub trait Held {
	/// The answer held for `ask`, and when it was fetched.
	fn held(&self, ask: &Ask) -> Result<Option<(serde_json::Value, SystemTime)>>;
}

pub struct Work {
	dir: PathBuf,
	held: Box<dyn Held>,
	age: Age,
	tiling: crate::poi::Tiling,
	/// An answer written before this is as good as none: a pull's, which asks again what it is built from.
	since: Option<SystemTime>,
	/// Age of every cached answer this run served, by kind.
	served: RefCell<Vec<(Kind, Duration)>>,
}
impl Work {
	pub fn at(dir: impl Into<PathBuf>, held: Box<dyn Held>) -> Self {
		Self {
			dir: dir.into(),
			held,
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

	pub fn path(&self) -> &Path {
		&self.dir
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

	/// What [`Held`] holds for `ask`, unless fetched before [`since`](Self::since).
	pub(crate) fn answer(&self, ask: &Ask) -> Result<Option<(serde_json::Value, SystemTime)>> {
		Ok(self.held.held(ask)?.filter(|(_, at)| self.since.is_none_or(|since| *at >= since)))
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
fn mtime(p: &Path) -> Result<SystemTime> {
	fs::metadata(p)?.modified().wrap_err_with(|| format!("{} has no mtime", p.display()))
}

fn days(d: Duration) -> f64 {
	d.as_secs_f64() / 86_400.
}

fn hex(bytes: &[u8]) -> String {
	bytes.iter().map(|b| format!("{b:02x}")).collect()
}
