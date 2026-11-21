//! What the CLI holds of the billed answers: what the conductor bought, as its public queries cache
//! serves it (`sa_core::queries_cache`), mirrored in the work dir so a run goes on from the mirror
//! while the cache is out of reach, until it needs an answer the mirror lacks.
use std::{
	fs,
	path::{Path, PathBuf},
	time::SystemTime,
};

use eyre::{Result, WrapErr, bail};
use gmaps_optimal_placement::Held;
use gmaps_optimal_placement_core::{Ask, SEARCH_TEXT};
use sa_core::queries_cache::{Key, Scheme, Stored, gop, under};
use sha2::{Digest, Sha256};

/// The settings' `queries_cache`.
#[derive(Clone, Debug)]
pub enum QueriesCache {
	/// `local`: the mirror alone, shared with no other machine.
	Local,
	/// The conductor's queries cache at this URL (`<origin>/api/queries_cache`).
	Conductor(String),
}

pub struct Mirror {
	/// `<work dir>/data/places_cache`, one `<key>.json` per answer.
	dir: PathBuf,
	cache: QueriesCache,
	/// Configured rather than `ureq::get`: the default turns a 4xx into an error before the body is
	/// read, and the queries cache's error body is the only thing that says what went wrong.
	agent: ureq::Agent,
}
impl Mirror {
	pub fn new(work: &Path, cache: QueriesCache) -> Self {
		Self {
			dir: work.join("data/places_cache"),
			cache,
			agent: ureq::Agent::config_builder().http_status_as_error(false).build().new_agent(),
		}
	}

	/// `kept` written to `dst` as of when it was fetched, so its age reads the same from either.
	fn mirror(&self, dst: &Path, kept: Stored) -> Result<(serde_json::Value, SystemTime)> {
		let at = SystemTime::from(kept.fetched_at);
		fs::create_dir_all(&self.dir).wrap_err_with(|| format!("creating {}", self.dir.display()))?;
		let part = dst.with_extension("part");
		fs::write(&part, kept.answer.to_string())?;
		fs::File::options().write(true).open(&part)?.set_modified(at)?;
		fs::rename(&part, dst)?;
		Ok((kept.answer, at))
	}
}

impl Held for Mirror {
	fn held(&self, ask: &Ask) -> Result<Option<(serde_json::Value, SystemTime)>> {
		let key = Key(Sha256::digest(gop(SEARCH_TEXT, &ask.body).as_bytes()).into());
		let dst = self.dir.join(format!("{key}.json"));
		match fs::read_to_string(&dst) {
			Ok(s) => {
				let at = fs::metadata(&dst)?.modified().wrap_err_with(|| format!("{} has no mtime", dst.display()))?;
				return Ok(Some((serde_json::from_str(&s).wrap_err_with(|| format!("cached response {} is not JSON", dst.display()))?, at)));
			}
			Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
			Err(e) => return Err(e).wrap_err_with(|| format!("reading {}", dst.display())),
		}
		let QueriesCache::Conductor(cache) = &self.cache else { return Ok(None) };
		let url = format!("{cache}{}", under(Scheme::Gop, key));
		let mut res = self.agent.get(&url).call().wrap_err_with(|| format!("the queries cache at {cache} did not answer"))?;
		let text = res.body_mut().read_to_string().wrap_err_with(|| format!("GET {url}: the answer was cut off"))?;
		match res.status().as_u16() {
			200 => Ok(Some(self.mirror(&dst, serde_json::from_str(&text).wrap_err("the queries cache's answer")?)?)),
			404 => Ok(None),
			status => bail!("the queries cache refused a read: {status}: {text}"),
		}
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

#[cfg(test)]
mod tests {
	use super::*;

	/// With the conductor's queries cache out of reach, a run goes on from what the work dir mirrors
	/// and stops at the first answer it does not hold.
	#[test]
	fn an_unreachable_cache_serves_the_mirror_and_stops_at_a_miss() {
		let dir = std::env::temp_dir().join(format!("gmaps_queries_cache_{}", std::process::id()));
		let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap(); // dropped: nothing listens there
		let held = Mirror::new(&dir, QueriesCache::Conductor(format!("http://{closed}/api/queries_cache")));
		let ask = |q: &str| Ask {
			mask: String::new(),
			body: serde_json::json!({"textQuery": q}).to_string(),
		};
		let key = Key(Sha256::digest(gop(SEARCH_TEXT, &ask("kept").body).as_bytes()).into());
		std::fs::create_dir_all(&held.dir).unwrap();
		std::fs::write(held.dir.join(format!("{key}.json")), r#"{"places":[]}"#).unwrap();

		assert_eq!(held.held(&ask("kept")).unwrap().unwrap().0, serde_json::json!({"places": []}));
		let e = held.held(&ask("missing")).unwrap_err();
		assert!(format!("{e:#}").contains("queries cache"), "{e:#}");
		std::fs::remove_dir_all(&dir).unwrap();
	}
}
