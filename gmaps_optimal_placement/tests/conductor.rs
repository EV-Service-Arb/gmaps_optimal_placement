//! `serve --behind-conductor`, through the binary and over HTTP: what the conductor's forward meets.
//!
//! A server that spends buys from a stub standing in for Places, in a work dir of its own beside the
//! real one's archives; any other has no Places key, so a build that tried to buy would fail rather
//! than spend. The grid is the same few hundred Clermont cells `fixture_bbox` reads, so the archive
//! is downloaded at most once.
use std::{
	io::{BufRead, BufReader, Read, Write},
	path::{Path, PathBuf},
	process::{Child, Command},
	sync::{
		Arc,
		atomic::{AtomicUsize, Ordering},
	},
	time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::Engine;
use sa_auth::{Assertion, Principal, Service, Signer, Study, User};
use sa_core::{
	gbp::Country,
	geo::LatLng,
	placement::{Area, Category, Definition, Gcid},
};

fn root() -> &'static Path {
	Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("crate lives in the workspace")
}

/// One server behind a conductor that signs with `panel`, over the French base and two trades
/// nobody has ever studied.
struct Served {
	child: Child,
	dir: PathBuf,
	port: u16,
	panel: Signer,
}

impl Served {
	/// Spending on `places` when given one, and on nothing otherwise.
	fn start(name: &str, places: Option<&Stub>) -> Self {
		let dir = std::env::temp_dir().join(format!("gmaps_conductor_{name}_{}", std::process::id()));
		if dir.exists() {
			std::fs::remove_dir_all(&dir).unwrap(); // a killed run's
		}
		for sub in ["trades", "countries", "site/mfe"] {
			std::fs::create_dir_all(dir.join(sub)).unwrap();
		}
		// no `google` without a stub: there is no key to buy with
		let google = match places {
			Some(_) => r#"google = [ { name = "stub"; provides = { places = { key = "stub"; }; }; } ];"#,
			None => "",
		};
		std::fs::write(
			dir.join("config.nix"),
			format!(r#"{{ age = {{ communes = "1y"; inventory = "1w"; ordering = "1y"; }}; sweep = {{ depth = 3; zoom = 12; }}; queries_cache = "local"; {google} }}"#),
		)
		.unwrap();
		std::fs::write(dir.join("site/mfe/gmaps_optimal_placement_overlay.js"), "export default async function init() {}").unwrap();
		std::fs::write(dir.join("countries/FR.nix"), format!("import {}", root().join("examples/countries/FR.nix").display())).unwrap();
		for t in ["never_asked", "never_asked_either"] {
			std::fs::write(
				dir.join(format!("trades/{t}.nix")),
				r#"loc: {
					inherit (loc) area grid candidate country column layer searches;
					poi = { source = "google_places"; tier = [ { name = "any"; weight = 1.0; match = "."; } ]; };
					model = { demand = "ind"; lambda_m = 2000; };
					rank = { nodes = 4; term = [ { text = "plombier"; weight = 1.0; } ]; };
				}"#,
			)
			.unwrap();
		}

		let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
		let panel: Signer = format!("k1:{}", base64::engine::general_purpose::STANDARD.encode([7u8; 32])).parse().unwrap();
		let mut cmd = Command::new(env!("CARGO_BIN_EXE_gmaps_optimal_placement"));
		cmd.arg("--config")
			.arg(dir.join("config.nix"))
			.args(["serve", "--behind-conductor", "--port", &port.to_string(), "-t"])
			.arg(dir.join("trades"))
			.arg("--countries")
			.arg(dir.join("countries"))
			.env("PANEL_ASSERTION_KEYS", panel.public())
			.env_remove("GMAPS_OPTIMAL_PLACEMENT_QUERIES_CACHE")
			.env("LEPTOS_SITE_ROOT", dir.join("site"));
		if let Some(stub) = places {
			// the stub's answers are kept in a work dir of their own, and never answer for Google's
			let (real, own) = (v_utils::xdg_cache_dir!("").join("data"), dir.join("cache/gmaps_optimal_placement/data"));
			std::fs::create_dir_all(&own).unwrap();
			for e in std::fs::read_dir(&real).unwrap() {
				let e = e.unwrap();
				if e.file_name() != "places_cache" {
					std::os::unix::fs::symlink(e.path(), own.join(e.file_name())).unwrap();
				}
			}
			cmd.arg("--spend-real-quota-i-am-certain")
				.env("XDG_CACHE_HOME", dir.join("cache"))
				.env("GOP_PLACES_STUB", format!("http://{}/v1/places:searchText", stub.at));
		}
		let child = cmd.spawn().unwrap();
		let mut served = Self { child, dir, port, panel };
		for _ in 0..240 {
			if served.get("/mfe/gmaps_optimal_placement_overlay.js", None).0 == 200 {
				return served;
			}
			if let Some(exit) = served.child.try_wait().unwrap() {
				panic!("the server exited before it listened: {exit}");
			}
			std::thread::sleep(Duration::from_millis(250));
		}
		panic!("the server never came up on {port}");
	}

	fn assertion(&self, method: &str, path: &str, held: &[Study]) -> String {
		sign(&self.panel, method, path, held)
	}
}

fn sign(panel: &Signer, method: &str, path: &str, held: &[Study]) -> String {
	let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
	panel.sign(&Assertion {
		aud: Service::Placement,
		principal: Principal::User(User {
			sub: "0192a7c4-1f2e-7d3b-9c4a-5e6f7a8b9c0d".into(),
			email: "op@example.com".into(),
			email_verified: true,
			name: "Op".into(),
			permissions: held.iter().map(|p| p.as_str()).collect(),
		}),
		method: method.into(),
		path: path.into(),
		exp: now + sa_auth::TTL,
	})
}

impl Served {
	fn get(&self, path: &str, assertion: Option<&str>) -> (u16, String) {
		self.send("GET", path, assertion, None)
	}

	fn send(&self, method: &str, path: &str, assertion: Option<&str>, body: Option<&str>) -> (u16, String) {
		let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
		let mut req = ureq::http::Request::builder().method(method).uri(format!("http://127.0.0.1:{}{path}", self.port));
		if let Some(a) = assertion {
			req = req.header(sa_auth::HEADER, a);
		}
		if body.is_some() {
			req = req.header("content-type", "application/json");
		}
		match agent.run(req.body(body.unwrap_or_default().as_bytes().to_vec()).unwrap()) {
			Ok(mut r) => (r.status().as_u16(), r.body_mut().read_to_string().unwrap()),
			Err(_) => (0, String::new()), // not listening yet
		}
	}

	/// A pull of `at` for someone who may spend, `query` verbatim: its status, `x-billed` and body.
	fn pull(&self, at: &str, query: &str) -> (u16, Option<usize>, String) {
		let path = format!("{at}/pull");
		let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
		let req = ureq::http::Request::post(format!("http://127.0.0.1:{}{path}{query}", self.port))
			.header(sa_auth::HEADER, self.assertion("POST", &path, &[Study::Read, Study::Spend]))
			.body(Vec::new())
			.unwrap();
		let mut r = agent.run(req).unwrap();
		let billed = r.headers().get("x-billed").map(|v| v.to_str().unwrap().parse().unwrap());
		(r.status().as_u16(), billed, r.body_mut().read_to_string().unwrap())
	}

	/// `method path`, signed for exactly that, for someone holding `held`.
	fn signed(&self, method: &str, path: &str, held: &[Study], body: Option<&str>) -> (u16, String) {
		self.send(method, path, Some(&self.assertion(method, path, held)), body)
	}
}

/// A few hundred Clermont cells, as the conductor would hand them in. Named per run: what is defined
/// is kept in the real work dir, beside the archive these share.
fn definition(study: Option<&str>) -> Definition {
	Definition {
		area: Area::Circle {
			slug: format!("clermont-centre-{}", std::process::id()),
			country: Country::FR,
			center: LatLng { lat: 45.78, lng: 3.07 },
			radius_m: 1000.,
			margin_m: 200.,
		},
		category: study.map(|s| Category {
			trade: 1,
			country: Country::FR,
			study: s.into(),
			keywords: vec![format!("gmaps_optimal_placement {s}")],
			google_primary: Gcid {
				id: "gcid:plumber".into(),
				name: format!("gmaps_optimal_placement {s}"),
			},
			google_additional: Vec::new(),
		}),
	}
}

impl Drop for Served {
	fn drop(&mut self) {
		self.child.kill().unwrap();
		self.child.wait().unwrap();
		std::fs::remove_dir_all(&self.dir).unwrap();
	}
}

/// The forward signs every request for exactly itself; the module the conductor's page imports is
/// the one thing open.
#[test]
fn behind_the_conductor_only_an_assertion_gets_in() {
	let s = Served::start("guard", None);
	let path = "/trades.json";
	assert_eq!(s.get(path, None).0, 401, "no assertion");
	let stranger: Signer = format!("k1:{}", base64::engine::general_purpose::STANDARD.encode([9u8; 32])).parse().unwrap();
	assert_eq!(
		s.get(path, Some(&sign(&stranger, "GET", path, &[Study::Read]))).0,
		401,
		"signed by a key the server does not hold"
	);
	assert_eq!(s.get(path, Some(&s.assertion("GET", "/studies/x", &[Study::Read]))).0, 401, "signed for another request");
	assert_eq!(s.get(path, Some(&s.assertion("GET", path, &[]))).0, 403, "signed, for someone who may not read studies");

	let (status, body) = s.signed("GET", path, &[Study::Read], None);
	assert_eq!(status, 200, "{body}");
	assert_eq!(
		serde_json::from_str::<serde_json::Value>(&body).unwrap(),
		serde_json::json!({"trades": ["never_asked", "never_asked_either"], "countries": ["FR"]})
	);
	assert_eq!(s.get("/mfe/gmaps_optimal_placement_overlay.js", None).0, 200, "the module is open");
}

/// A definition is addressed by its own hash, defining one buys nothing, and only a pull may spend.
#[test]
fn a_study_is_defined_then_read_and_only_a_pull_spends() {
	let s = Served::start("define", None);
	let def = definition(Some("never_asked"));
	let (id, body) = (def.id(), serde_json::to_string(&def).unwrap());
	let at = format!("/studies/{id}");

	assert_eq!(s.signed("GET", &at, &[Study::Read], None).0, 404, "nothing is defined yet");
	let wrong = format!("/studies/{}", definition(Some("never_asked_either")).id());
	assert_eq!(s.signed("PUT", &wrong, &[Study::Read], Some(&body)).0, 400, "a body under someone else's hash");
	assert_eq!(s.signed("PUT", &at, &[], Some(&body)).0, 403, "defining is reading");
	let (status, text) = s.signed("PUT", &at, &[Study::Read], Some(&body));
	assert_eq!(status, 204, "{text}");

	let (status, text) = s.signed("GET", &at, &[Study::Read], None);
	assert_eq!(status, 200, "a refusal is a notice, not an error: {text}");
	let payload: serde_json::Value = serde_json::from_str(&text).unwrap();
	assert!(payload["trade"].is_null(), "nothing that scores a competitor was held or bought");
	assert_eq!(payload["name"], format!("never_asked·FR·clermont-centre-{}", std::process::id()));
	let notice = payload["notice"]["text"].as_str().unwrap();
	assert!(payload["notice"]["missing"].as_u64().is_some_and(|n| n > 0), "the sweep it lacks is counted: {notice}");

	let pull = format!("{at}/pull");
	assert_eq!(s.signed("POST", &pull, &[Study::Read], None).0, 403, "a pull spends, and reading is not spending");
	let (status, billed, text) = s.pull(&at, "?budget=1000");
	assert!(
		status >= 400 && billed == Some(0),
		"a server started without spending buys nothing, whatever the budget: {status} {text}"
	);
	forget(&id);
}

fn forget(id: &str) {
	std::fs::remove_file(v_utils::xdg_cache_dir!("").join("studies").join(format!("{id}.json"))).unwrap();
}

/// A definition the served trades cannot take is refused when it is handed in, not when it is read.
#[test]
fn a_definition_that_does_not_hold_is_refused_on_put() {
	let s = Served::start("holds", None);
	let def = definition(Some("not_a_trade"));
	let (status, text) = s.signed("PUT", &format!("/studies/{}", def.id()), &[Study::Read], Some(&serde_json::to_string(&def).unwrap()));
	assert_eq!(status, 422, "{text}");
	let refused: serde_json::Value = serde_json::from_str(&text).unwrap();
	assert_eq!(refused["code"], "unstudied", "{text}");
	assert!(refused["error"].as_str().unwrap().contains("not_a_trade"), "{text}");

	let bare = definition(None);
	let at = format!("/studies/{}", bare.id());
	assert_eq!(s.signed("PUT", &at, &[Study::Read], Some(&serde_json::to_string(&bare).unwrap())).0, 204);
	let (status, text) = s.signed("GET", &at, &[Study::Read], None);
	assert_eq!(status, 200, "{text}");
	let payload: serde_json::Value = serde_json::from_str(&text).unwrap();
	assert!(payload["trade"].is_null() && payload["notice"].is_null(), "an area on its own is the grid, and asks nothing");
	forget(&bare.id());
}

/// Places, as far as a sweep can tell: one rated competitor in every tile, and a second page behind
/// the first answer when `paged`. Counts what it served.
struct Stub {
	at: std::net::SocketAddr,
	served: Arc<AtomicUsize>,
}

impl Stub {
	fn start(paged: bool) -> Self {
		let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
		let (at, served) = (listener.local_addr().unwrap(), Arc::<AtomicUsize>::default());
		let counted = served.clone();
		// detached: it lives as long as the test binary, and answers nothing once its server is gone
		std::thread::spawn(move || {
			for conn in listener.incoming() {
				let mut conn = BufReader::new(conn.unwrap());
				let mut len = 0;
				loop {
					let mut line = String::new();
					conn.read_line(&mut line).unwrap();
					if line == "\r\n" {
						break;
					}
					if let Some((k, v)) = line.split_once(':')
						&& k.eq_ignore_ascii_case("content-length")
					{
						len = v.trim().parse().unwrap();
					}
				}
				conn.read_exact(&mut vec![0; len]).unwrap();
				let n = counted.fetch_add(1, Ordering::SeqCst);
				let mut answer = serde_json::json!({"places": [{
					"id": "stub", "displayName": {"text": "Stub plombier"}, "rating": 4.5, "userRatingCount": 10,
					"location": {"latitude": 45.78, "longitude": 3.07},
				}]});
				if paged && n == 0 {
					answer["nextPageToken"] = "more".into();
				}
				let body = answer.to_string();
				write!(
					conn.get_mut(),
					"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
					body.len()
				)
				.unwrap();
			}
		});
		Self { at, served }
	}

	fn served(&self) -> usize {
		self.served.load(Ordering::SeqCst)
	}
}

/// `study` defined on `s` under a Places query of its own: its path, and the searches its sweep
/// counts it lacks, as a read's notice says.
fn defined(s: &Served, study: &str) -> (String, usize) {
	let def = definition(Some(study));
	let at = format!("/studies/{}", def.id());
	let (status, text) = s.signed("PUT", &at, &[Study::Read], Some(&serde_json::to_string(&def).unwrap()));
	assert_eq!(status, 204, "{text}");
	let (status, text) = s.signed("GET", &at, &[Study::Read], None);
	assert_eq!(status, 200, "{text}");
	let missing = serde_json::from_str::<serde_json::Value>(&text).unwrap()["notice"]["missing"].as_u64().unwrap();
	(at, missing as usize)
}

/// A pull says up front how many searches it may buy, and one that needs more is refused before the
/// first is bought; what it did buy rides on its answer, for the conductor to charge.
#[test]
fn a_pull_buys_within_its_budget_and_says_what_it_bought() {
	let places = Stub::start(false);
	let s = Served::start("budget", Some(&places));
	let (at, need) = defined(&s, "never_asked");
	assert!(need > 0, "nothing is held yet");

	for malformed in ["", "?budget=-1"] {
		let (status, billed, text) = s.pull(&at, malformed);
		assert_eq!((status, billed), (400, None), "a budget is named, never assumed, and a pull not run bought nothing: {text}");
	}
	let (status, billed, text) = s.pull(&at, "?budget=0");
	assert_eq!((status, billed), (402, Some(0)), "{text}");
	assert_eq!(serde_json::from_str::<serde_json::Value>(&text).unwrap()["code"], "unaffordable", "{text}");
	assert_eq!(places.served(), 0, "refused before the first search");

	let (status, billed, text) = s.pull(&at, &format!("?budget={need}"));
	assert_eq!(status, 200, "{text}");
	assert_eq!(billed, Some(places.served()), "every search bought is charged, and nothing else");
	assert_eq!(places.served(), need, "a sweep that opens nothing past what it counted buys exactly that");
}

/// What a sweep counts up front is a floor — a second page, a tile at the cap — so the budget is
/// held at every search, not only before the first. A pull stopped part-way has paid for what it
/// bought, so the next buys only the rest; once one ends whole, the one after re-asks everything.
#[test]
fn a_pull_never_buys_past_its_budget_and_a_retry_buys_only_the_rest() {
	let places = Stub::start(true);
	let s = Served::start("over_budget", Some(&places));
	let (at, need) = defined(&s, "never_asked");
	let (status, billed, text) = s.pull(&at, &format!("?budget={need}"));
	assert_eq!(status, 402, "the first answer's second page is one search more than was counted: {text}");
	assert_eq!((billed, places.served()), (Some(need), need), "bought up to the budget and charged for it: {text}");

	let (status, billed, text) = s.pull(&at, "?budget=1000");
	assert_eq!(status, 200, "{text}");
	assert_eq!(
		(billed, places.served()),
		(Some(1), need + 1),
		"the retry buys the one search the stopped pull did not, and is charged for that alone"
	);

	let (status, billed, text) = s.pull(&at, "?budget=1000");
	assert_eq!(status, 200, "{text}");
	assert_eq!((billed, places.served()), (Some(need), 2 * need + 1), "after a whole pull, every tile is asked again");
}
