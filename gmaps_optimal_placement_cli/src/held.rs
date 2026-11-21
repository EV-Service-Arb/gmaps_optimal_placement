//! What the CLI holds of the billed answers: a copy of prod's `panel.db`, the conductor's queries
//! cache (`sa_core::queries_cache`), opened read-only. Taken by site_conductor's `prod-restore`; the
//! CLI buys nothing, so it has nothing to give back.
use std::{
	path::Path,
	time::{Duration, SystemTime},
};

use eyre::{Result, WrapErr};
use gmaps_optimal_placement::Held;
use gmaps_optimal_placement_core::{Ask, SEARCH_TEXT};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use sa_core::queries_cache::{Key, Scheme, gop};
use sha2::{Digest, Sha256};

pub struct Copy(Connection);
impl Copy {
	pub fn open(path: &Path) -> Result<Self> {
		eyre::ensure!(
			path.is_file(),
			"no copy of prod's panel.db at {} (site_conductor's `nix run .#prod-restore` takes one)",
			path.display()
		);
		// immutable: a copy nobody writes, so no lock and no WAL index is asked of its directory
		let uri = format!(
			"file:{}?mode=ro&immutable=1",
			path.display().to_string().replace('%', "%25").replace('?', "%3f").replace('#', "%23")
		);
		let db = Connection::open_with_flags(uri, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI).wrap_err_with(|| format!("opening {}", path.display()))?;
		Ok(Self(db))
	}
}

impl Held for Copy {
	fn held(&self, ask: &Ask) -> Result<Option<(serde_json::Value, SystemTime)>> {
		let key = Key(Sha256::digest(gop(SEARCH_TEXT, &ask.body).as_bytes()).into());
		let row: Option<(String, i64)> = self
			.0
			.query_row(
				"SELECT answer, fetched_at FROM queries_cache WHERE scheme = ?1 AND key = ?2",
				(Scheme::Gop.as_str(), &key.0[..]),
				|r| Ok((r.get(0)?, r.get(1)?)),
			)
			.optional()
			.wrap_err_with(|| format!("reading {key} from the copy of prod's queries cache"))?;
		row.map(|(answer, us)| {
			let us = u64::try_from(us).wrap_err_with(|| format!("{key} was fetched before 1970: {us} µs"))?;
			Ok((
				serde_json::from_str(&answer).wrap_err_with(|| format!("the answer kept for {key} is not JSON"))?,
				SystemTime::UNIX_EPOCH + Duration::from_micros(us),
			))
		})
		.transpose()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A study's search is read off a file in the conductor's shape, with when it was fetched; one it
	/// never bought is none.
	#[test]
	fn a_copy_serves_what_it_holds_as_of_its_fetch() {
		let path = std::env::temp_dir().join(format!("gmaps_queries_cache_{}.db", std::process::id()));
		let ask = |q: &str| Ask {
			mask: String::new(),
			body: serde_json::json!({"textQuery": q}).to_string(),
		};
		let request = gop(SEARCH_TEXT, &ask("kept").body);
		let key = Key(Sha256::digest(request.as_bytes()).into());
		let fetched = SystemTime::UNIX_EPOCH + Duration::from_micros(1_790_000_000_123_456);
		{
			let db = Connection::open(&path).unwrap();
			db.execute_batch(
				"PRAGMA journal_mode = WAL;
				 CREATE TABLE queries_cache (scheme TEXT NOT NULL, key BLOB NOT NULL, answer TEXT NOT NULL, request TEXT, fetched_at INTEGER NOT NULL, PRIMARY KEY (scheme, key)) STRICT;",
			)
			.unwrap();
			db.execute("INSERT INTO queries_cache VALUES ('gop', ?1, '{\"places\":[]}', ?2, 1790000000123456)", (&key.0[..], &request))
				.unwrap();
		}
		let held = Copy::open(&path).unwrap();
		assert_eq!(held.held(&ask("kept")).unwrap(), Some((serde_json::json!({"places": []}), fetched)));
		assert_eq!(held.held(&ask("missing")).unwrap(), None);
		drop(held);
		for suffix in ["", "-wal", "-shm"] {
			match std::fs::remove_file(format!("{}{suffix}", path.display())) {
				Err(e) if e.kind() != std::io::ErrorKind::NotFound => panic!("removing the test's copy: {e}"),
				_ => {} // -wal and -shm go with the last writer
			}
		}
	}
}
