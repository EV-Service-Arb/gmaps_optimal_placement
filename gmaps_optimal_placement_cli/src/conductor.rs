//! The conductor (settings' `conductor`), which buys what a study lacks at the tokens of whoever is
//! signed in (site_conductor's docs/ARCHITECTURE.md, Placement), and the copy of its queries cache,
//! which takes what it bought.
//!
//! ```text
//! quote ─ GET /api/v1/tokens                     the price of a search, the balance
//! run   ─ POST /api/v1/placement/pull            the Definition, bought whole and charged; its end an event
//!       └ GET /api/queries_cache/gop/{key}       each answer it is built from ─▶ the copy
//! 401   ─ /mcp/{register,authorize,token}        signed in to in the browser, once
//! ```
use std::{
	cell::RefCell,
	fs,
	io::{Read, Write},
	net::{TcpListener, TcpStream},
	os::unix::{fs::OpenOptionsExt, process::CommandExt},
	path::PathBuf,
	process::{Child, Command, Stdio},
	rc::Rc,
	sync::Mutex,
	thread::sleep,
	time::{Duration, Instant, SystemTime},
};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use eyre::{Result, WrapErr, bail, ensure, eyre};
use gmaps_optimal_placement::{Held, Work};
use gmaps_optimal_placement_core::{Ask, Definition, SEARCH_TEXT};
use gmaps_optimal_placement_sources::work::Kind;
use gmaps_optimal_placement_web::{Buyer, Quote, Works};
use sa_core::queries_cache::{Key, PREFIX, Scheme, Stored, gop};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::held::{self, Answer};

/// How long a sign-in waits on the browser.
const SIGN_IN: Duration = Duration::from_secs(300);
/// Of a pull: the conductor's own bound on one is 10 min.
const PULL: Duration = Duration::from_secs(660);

pub struct Conductor {
	/// `None`: the settings name none.
	pub origin: Option<String>,
	/// Where the session's bearer is kept.
	pub session: PathBuf,
	/// The copy of its queries cache.
	pub copy: PathBuf,
	/// The work dir.
	pub dir: PathBuf,
	pub works: Works,
	/// One at a time: two would each write the copy the other read.
	pub turn: Mutex<()>,
}
impl Conductor {
	fn origin(&self) -> Result<&str> {
		self.origin
			.as_deref()
			.ok_or_else(|| eyre!("a study is bought by the conductor, and settings carry no `conductor`"))
	}

	/// As who is signed in, who is asked to when nobody is: a POST of `body`, or a GET.
	fn api(&self, path: &str, body: Option<&Definition>) -> Result<String> {
		let url = format!("{}{path}", self.origin()?);
		let mut signed_in = false;
		loop {
			let bearer = match fs::read_to_string(&self.session) {
				Ok(kept) => kept,
				Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
					signed_in = true;
					self.sign_in()?
				}
				Err(e) => return Err(e).wrap_err_with(|| format!("reading {}", self.session.display())),
			};
			let bearer = format!("Bearer {bearer}");
			let mut res = match body {
				Some(def) => agent(PULL).post(&url).header("authorization", &bearer).send_json(def),
				None => agent(Duration::from_secs(30)).get(&url).header("authorization", &bearer).call(),
			}
			.wrap_err_with(|| url.clone())?;
			if res.status() == 401 && !signed_in {
				fs::remove_file(&self.session).wrap_err_with(|| format!("removing {}", self.session.display()))?;
				continue;
			}
			let text = res.body_mut().read_to_string().wrap_err_with(|| url.clone())?;
			ensure!(res.status().is_success(), "{url}: {} {text}", res.status());
			return Ok(text);
		}
	}

	/// A session of this CLI's own, granted in the browser (OAuth 2.1, PKCE): its bearer, kept.
	fn sign_in(&self) -> Result<String> {
		#[derive(Deserialize)]
		struct Client {
			client_id: String,
		}
		#[derive(Deserialize)]
		struct Granted {
			access_token: String,
		}
		let base = format!("{}/mcp", self.origin()?);
		let listener = TcpListener::bind("127.0.0.1:0").wrap_err("binding the sign-in's loopback listener")?;
		let redirect = format!("http://{}/", listener.local_addr()?);
		let Client { client_id } = answered(
			agent(Duration::from_secs(30))
				.post(format!("{base}/register"))
				.send_json(serde_json::json!({ "client_name": "gmaps_optimal_placement", "redirect_uris": [redirect] })),
			"registering with the conductor",
		)?;
		let (verifier, state) = (random()?, random()?);
		let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(&verifier));
		let url = format!("{base}/authorize?response_type=code&client_id={client_id}&redirect_uri={redirect}&code_challenge={challenge}&code_challenge_method=S256&state={state}");
		eprintln!("signing in to the conductor: {url}");
		let mut opener = Command::new("xdg-open")
			.arg(&url)
			.stdin(Stdio::null())
			.process_group(0) // it may become the browser itself, which the terminal's SIGINT would end whole
			.spawn()
			.wrap_err("running xdg-open")?;
		let code = code(&listener, &state, &mut opener)?;
		let Granted { access_token } = answered(
			agent(Duration::from_secs(30)).post(format!("{base}/token")).send_form([
				("grant_type", "authorization_code"),
				("client_id", &client_id),
				("code", &code),
				("code_verifier", &verifier),
				("redirect_uri", &redirect),
			]),
			"signing in to the conductor",
		)?;
		fs::OpenOptions::new()
			.write(true)
			.create(true)
			.truncate(true)
			.mode(0o600)
			.open(&self.session)
			.and_then(|mut f| f.write_all(access_token.as_bytes()))
			.wrap_err_with(|| format!("writing {}", self.session.display()))?;
		Ok(access_token)
	}
}

impl Buyer for Conductor {
	fn quote(&self, def: &Definition) -> Result<Quote> {
		let study = gmaps_optimal_placement::load(def)?;
		let held = (self.works)()?;
		study.asks(&held)?;
		let searches = study.asks(&(self.works)()?.since(Some(SystemTime::now())))?.len();
		let tokens: Tokens = serde_json::from_str(&self.api("/api/v1/tokens", None)?).wrap_err("the conductor's word on the tokens held")?;
		Ok(Quote {
			searches,
			price: tokens.prices.placement_search,
			balance: tokens.balance,
			oldest_days: held.age(Kind::Inventory).map(|a| a.oldest.as_secs_f64() / 86_400.),
		})
	}

	fn run(&self, def: &Definition) -> Result<()> {
		let turn = self.turn.lock().map_err(|_| eyre!("an earlier run panicked: what it left of the copy is not known"))?;
		let ended = self.api("/api/v1/placement/pull", Some(def))?;
		let said = |field: &str| ended.lines().find_map(|l| l.strip_prefix(field));
		match said("event: ") {
			Some("pulled") => {}
			Some("refused") => bail!("the conductor refused to run {}: {}", def.name(), said("data: ").unwrap_or_default()),
			_ => bail!("the conductor's pull of {} ended without a word; run again, it buys what this did not", def.name()),
		}
		let got = Rc::<RefCell<Vec<Answer>>>::default();
		let work = Work::at(
			&self.dir,
			Box::new(Bought {
				origin: self.origin()?.to_owned(),
				got: got.clone(),
			}),
		);
		let taken = gmaps_optimal_placement::load(def)?.asks(&work);
		// what did arrive is kept, whether the rest did or not
		held::keep(&self.copy, &got.borrow())?;
		drop(turn);
		let lacking = taken
			.wrap_err_with(|| {
				format!(
					"{} is bought and the conductor's; a restore of its copy ({}) brings what did not arrive",
					def.name(),
					self.copy.display()
				)
			})?
			.len();
		ensure!(
			lacking == 0,
			"the conductor bought {}, and {lacking} of the searches it is built from here are not among what it holds: it runs another sweep than this build",
			def.name()
		);
		Ok(())
	}
}

/// The caller's balance and what a search costs them, of what the conductor says of their tokens.
#[derive(Deserialize)]
struct Tokens {
	balance: f64,
	prices: Prices,
}
#[derive(Deserialize)]
struct Prices {
	#[serde(rename = "placement.search")]
	placement_search: f64,
}

/// What the conductor holds, each answer noted as it is read.
struct Bought {
	origin: String,
	got: Rc<RefCell<Vec<Answer>>>,
}
impl Held for Bought {
	fn held(&self, ask: &Ask) -> Result<Option<(serde_json::Value, SystemTime)>> {
		let request = gop(SEARCH_TEXT, &ask.body);
		let url = format!("{}{PREFIX}/{}/{}", self.origin, Scheme::Gop, Key::of(&request));
		let began = Instant::now();
		let mut res = loop {
			let res = agent(Duration::from_secs(30)).get(&url).call().wrap_err_with(|| url.clone())?;
			// the edge bounds how often one client reads
			if res.status() != 429 {
				break res;
			}
			ensure!(began.elapsed() < Duration::from_secs(60), "{url}: refused as too many for a minute");
			sleep(Duration::from_secs(1));
		};
		if res.status() == 404 {
			return Ok(None);
		}
		ensure!(res.status().is_success(), "{url}: {}", res.status());
		let Stored { answer, fetched_at } = res.body_mut().read_json().wrap_err_with(|| url.clone())?;
		self.got.borrow_mut().push(Answer {
			request,
			answer: answer.to_string(),
			fetched_us: fetched_at.as_microsecond(),
		});
		Ok(Some((answer, fetched_at.into())))
	}
}

/// A refusal is an answer, its status read.
fn agent(timeout: Duration) -> ureq::Agent {
	ureq::Agent::config_builder().http_status_as_error(false).timeout_global(Some(timeout)).build().into()
}

fn answered<T: for<'de> Deserialize<'de>>(res: Result<ureq::http::Response<ureq::Body>, ureq::Error>, what: &'static str) -> Result<T> {
	let mut res = res.wrap_err(what)?;
	let text = res.body_mut().read_to_string().wrap_err(what)?;
	ensure!(res.status().is_success(), "{what}: {} {text}", res.status());
	serde_json::from_str(&text).wrap_err(what)
}

fn random() -> Result<String> {
	let mut bytes = [0; 32];
	fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).wrap_err("reading /dev/urandom")?;
	Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// The code the conductor sends the browser back with, to the one request that carries `state`.
fn code(listener: &TcpListener, state: &str, opener: &mut Child) -> Result<String> {
	let began = Instant::now();
	listener.set_nonblocking(true)?;
	loop {
		match listener.accept() {
			Ok((mut browser, _)) =>
				if let Some(code) = redirected(&mut browser, state)? {
					return Ok(code);
				},
			Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
				if let Some(status) = opener.try_wait().wrap_err("waiting for xdg-open")? {
					ensure!(status.success(), "xdg-open: {status}");
				}
				ensure!(began.elapsed() < SIGN_IN, "nobody signed in to the conductor within {} s", SIGN_IN.as_secs());
				sleep(Duration::from_millis(100));
			}
			Err(e) => return Err(e).wrap_err("the sign-in's loopback listener"),
		}
	}
}

/// `None` for a request that is not the redirect: a browser asks for more than it was sent to.
fn redirected(browser: &mut TcpStream, state: &str) -> Result<Option<String>> {
	browser.set_nonblocking(false)?;
	browser.set_read_timeout(Some(Duration::from_secs(5)))?;
	let (mut head, mut chunk) = (Vec::new(), [0; 1024]);
	while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 16 << 10 {
		match browser.read(&mut chunk) {
			Ok(n) if n > 0 => head.extend_from_slice(&chunk[..n]),
			_ => return Ok(None), // a connection opened ahead, or given up
		}
	}
	let head = String::from_utf8_lossy(&head);
	let query = head.split(' ').nth(1).and_then(|target| target.split_once("/?")).map(|(_, q)| q).unwrap_or_default();
	let param = |name: &str| query.split('&').find_map(|p| p.strip_prefix(name)?.strip_prefix('='));
	let ours = param("state") == Some(state);
	let (status, said) = match ours {
		true => ("200 OK", "Back to the map: this page can be closed."),
		false => ("404 Not Found", ""),
	};
	browser
		.write_all(
			format!(
				"HTTP/1.1 {status}\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{said}",
				said.len()
			)
			.as_bytes(),
		)
		.wrap_err("answering the browser")?;
	if !ours {
		return Ok(None);
	}
	if let Some(e) = param("error") {
		bail!("the conductor's sign-in was refused: {e}");
	}
	Ok(Some(param("code").ok_or_else(|| eyre!("the conductor sent no code back"))?.to_owned()))
}
