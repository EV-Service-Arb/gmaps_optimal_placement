//! `serve`, through the binary and over HTTP: what the conductor meets.
//!
//! Each server runs in a work dir of its own beside the real one's archives, its queries cache that
//! dir alone, which the test answers into as the conductor would. The grid is the same few hundred
//! Clermont cells `fixture_bbox` reads, so the archive is downloaded at most once.
use std::{
	path::{Path, PathBuf},
	process::{Child, Command},
	time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::Engine;
use sa_auth::{Assertion, Principal, Service, Signer, Study, User};
use sa_core::{
	gbp::Country,
	geo::LatLng,
	placement::{Area, Ask, Asks, Category, Definition, Demand, Excluded, Gcid, Keyword},
	queries_cache::gop,
};
use sha2::{Digest, Sha256};

/// The server's work dir, under its dir.
const OWN: &str = "cache/gmaps_optimal_placement";

fn root() -> &'static Path {
	Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("crate lives in the workspace")
}

/// One server behind a conductor that signs with `panel`, over the French base alone.
struct Served {
	child: Child,
	dir: PathBuf,
	port: u16,
	panel: Signer,
}

impl Served {
	fn start(name: &str) -> Self {
		let dir = std::env::temp_dir().join(format!("gmaps_conductor_{name}_{}", std::process::id()));
		if dir.exists() {
			std::fs::remove_dir_all(&dir).unwrap(); // a killed run's
		}
		std::fs::create_dir_all(dir.join("countries")).unwrap();
		std::fs::write(
			dir.join("config.nix"),
			r#"{ age = { communes = "1y"; inventory = "1w"; ordering = "1y"; }; sweep = { depth = 3; zoom = 12; }; queries_cache = "local"; }"#,
		)
		.unwrap();
		std::fs::write(dir.join("countries/FR.nix"), format!("import {}", root().join("examples/countries/FR.nix").display())).unwrap();

		let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
		let panel: Signer = format!("k1:{}", base64::engine::general_purpose::STANDARD.encode([7u8; 32])).parse().unwrap();
		let mut cmd = Command::new(env!("CARGO_BIN_EXE_gmaps_optimal_placement"));
		cmd.arg("--config")
			.arg(dir.join("config.nix"))
			.args(["serve", "--port", &port.to_string(), "--countries"])
			.arg(dir.join("countries"))
			.env("PANEL_ASSERTION_KEYS", panel.public())
			.env("XDG_CACHE_HOME", dir.join("cache"))
			.env_remove("GMAPS_OPTIMAL_PLACEMENT_QUERIES_CACHE");
		// the test's answers are kept in a work dir of their own, and never answer for Google's
		let (real, own) = (v_utils::xdg_cache_dir!("").join("data"), dir.join(OWN).join("data"));
		std::fs::create_dir_all(own.join("places_cache")).unwrap();
		for e in std::fs::read_dir(&real).unwrap() {
			let e = e.unwrap();
			if e.file_name() != "places_cache" {
				std::os::unix::fs::symlink(e.path(), own.join(e.file_name())).unwrap();
			}
		}
		let child = cmd.spawn().unwrap();
		let mut served = Self { child, dir, port, panel };
		for _ in 0..240 {
			if served.get("/health", None).0 == 200 {
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

	/// `ask` answered with `answer`, as the conductor buys it into the queries cache.
	fn answer(&self, ask: &Ask, answer: serde_json::Value) {
		let key: String = Sha256::digest(gop(&ask.body)).iter().map(|b| format!("{b:02x}")).collect();
		std::fs::write(self.dir.join(OWN).join(format!("data/places_cache/{key}.json")), answer.to_string()).unwrap();
	}

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

	/// `method path`, signed for exactly that, for someone holding `held`.
	fn signed(&self, method: &str, path: &str, held: &[Study], body: Option<&str>) -> (u16, String) {
		self.send(method, path, Some(&self.assertion(method, path, held)), body)
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
			actor: None,
		}),
		method: method.into(),
		path: path.into(),
		exp: now + sa_auth::TTL,
	})
}

/// A few hundred cells of `country` around one point, as the conductor would hand them in.
fn definition(trade: Option<&str>, country: Country) -> Definition {
	Definition {
		area: Area::Circle {
			slug: format!("clermont-centre-{}", std::process::id()),
			country,
			center: LatLng { lat: 45.78, lng: 3.07 },
			radius_m: 1000.,
			margin_m: 200.,
		},
		category: trade.map(|t| Category {
			trade: 1,
			name: t.into(),
			country,
			primary: Gcid {
				id: "gcid:plumber".into(),
				name: "Plombier".into(),
			},
			additional: Vec::new(),
			keywords: vec![Keyword {
				text: "plombier".into(),
				weight: Some(1.),
			}],
			excluded: Excluded::default(),
			demand: Some(Demand {
				expr: "ind".into(),
				columns: Vec::new(),
				layers: Vec::new(),
			}),
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

/// The conductor signs every request for exactly itself.
#[test]
fn behind_the_conductor_only_an_assertion_gets_in() {
	let s = Served::start("guard");
	let path = "/catalog.json";
	assert_eq!(s.get(path, None).0, 401, "no assertion");
	let stranger: Signer = format!("k1:{}", base64::engine::general_purpose::STANDARD.encode([9u8; 32])).parse().unwrap();
	assert_eq!(
		s.get(path, Some(&sign(&stranger, "GET", path, &[Study::Read]))).0,
		401,
		"signed by a key the server does not hold"
	);
	assert_eq!(s.get(path, Some(&s.assertion("POST", "/studies", &[Study::Read]))).0, 401, "signed for another request");
	assert_eq!(s.get(path, Some(&s.assertion("GET", path, &[]))).0, 403, "signed, for someone who may not read studies");

	let (status, body) = s.signed("GET", path, &[Study::Read], None);
	assert_eq!(status, 200, "{body}");
	assert_eq!(serde_json::from_str::<serde_json::Value>(&body).unwrap(), serde_json::json!({"countries": ["FR"]}));
}

/// A study is read by its definition, opens on what is held, and says what it lacks.
#[test]
fn a_study_is_read_and_says_what_it_lacks() {
	let s = Served::start("read");
	let def = definition(Some("never_asked"), Country::FR);
	let body = serde_json::to_string(&def).unwrap();

	assert_eq!(s.signed("POST", "/studies", &[], Some(&body)).0, 403, "for someone who may not read studies");
	let (status, text) = s.signed("POST", "/studies", &[Study::Read], Some(&body));
	assert_eq!(status, 200, "a refusal is a notice, not an error: {text}");
	let payload: serde_json::Value = serde_json::from_str(&text).unwrap();
	assert!(payload["trade"].is_null(), "nothing that scores a competitor was held or bought");
	assert_eq!(payload["name"], format!("never_asked·FR·clermont-centre-{}", std::process::id()));
	let notice = payload["notice"]["text"].as_str().unwrap();
	assert!(payload["notice"]["missing"].as_u64().is_some_and(|n| n > 0), "the sweep it lacks is counted: {notice}");

	for path in ["/studies/asks", "/studies/pull"] {
		assert_eq!(s.signed("POST", path, &[Study::Read], Some(&body)).0, 403, "{path} is a pull's, and reading is not pulling");
	}
}

/// A definition in a country no base is served for is refused, and an area on its own is the grid.
#[test]
fn a_definition_that_does_not_hold_is_refused() {
	let s = Served::start("holds");
	let def = definition(Some("never_asked"), Country::GB);
	let (status, text) = s.signed("POST", "/studies", &[Study::Read], Some(&serde_json::to_string(&def).unwrap()));
	assert_eq!(status, 422, "{text}");
	let refused: serde_json::Value = serde_json::from_str(&text).unwrap();
	assert_eq!(refused["code"], "unstudied", "{text}");
	assert!(refused["error"].as_str().unwrap().contains("GB"), "{text}");

	let bare = definition(None, Country::FR);
	let (status, text) = s.signed("POST", "/studies", &[Study::Read], Some(&serde_json::to_string(&bare).unwrap()));
	assert_eq!(status, 200, "{text}");
	let payload: serde_json::Value = serde_json::from_str(&text).unwrap();
	assert!(payload["trade"].is_null(), "an area on its own is the grid, and asks nothing");
	assert!(payload["notice"].is_null(), "an area on its own is the grid, and asks nothing");
}

/// `def`'s asks, for someone who may pull.
fn asks(s: &Served, def: &Definition) -> Asks {
	let (status, text) = s.signed("POST", "/studies/asks", &[Study::Read, Study::Spend], Some(&serde_json::to_string(def).unwrap()));
	assert_eq!(status, 200, "{text}");
	serde_json::from_str(&text).unwrap()
}

/// A pull's searches are named a round at a time, each round what lies behind the last one's
/// answers, until none is lacking; the pull then builds out of them alone, and the next pull asks
/// everything again.
#[test]
fn a_pull_is_asked_what_lies_behind_each_answer_until_none_is_lacking() {
	let s = Served::start("asks");
	let def = definition(Some("never_asked"), Country::FR);
	let place = serde_json::json!({
		"id": "stub", "displayName": {"text": "Stub plombier"}, "rating": 4.5, "userRatingCount": 10,
		"location": {"latitude": 45.78, "longitude": 3.07},
	});

	let first = asks(&s, &def);
	assert!(!first.asks.is_empty(), "nothing is held yet");
	assert!(first.asks.iter().all(|a| !a.body.contains("pageToken")), "a first page down each branch: {:?}", first.asks);
	for (i, a) in first.asks.iter().enumerate() {
		let mut answer = serde_json::json!({ "places": [place] });
		if i == 0 {
			answer["nextPageToken"] = "more".into();
		}
		s.answer(a, answer);
	}

	let second = asks(&s, &def);
	assert_eq!(second.since, first.since, "one pull, until it ends whole");
	let [behind] = second.asks.as_slice() else {
		panic!("only the second page is behind what was answered: {:?}", second.asks)
	};
	assert!(behind.body.contains(r#""pageToken":"more""#), "{behind:?}");
	s.answer(behind, serde_json::json!({ "places": [] }));
	assert!(asks(&s, &def).asks.is_empty(), "the sweep is held whole");

	let (status, text) = s.signed("POST", "/studies/pull", &[Study::Read, Study::Spend], Some(&serde_json::to_string(&def).unwrap()));
	assert_eq!(status, 200, "{text}");
	let payload: serde_json::Value = serde_json::from_str(&text).unwrap();
	assert!(payload["notice"].is_null(), "built of what was answered: {text}");
	assert!(payload["trade"]["pois"].as_array().is_some_and(|p| p.len() == 1), "built of what was answered: {text}");

	let next = asks(&s, &def);
	assert!(next.since > first.since, "a whole pull's answers are what the next one asks again");
	assert_eq!(next.asks, first.asks);
}
