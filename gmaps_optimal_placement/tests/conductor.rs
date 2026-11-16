//! `serve --behind-conductor`, through the binary and over HTTP: what the conductor's forward meets.
//!
//! No Places key is configured, so a build that tried to buy anything would fail rather than spend.
//! The grid comes out of the real work dir, over the same few hundred Clermont cells `fixture_bbox`
//! reads, so the archive is downloaded at most once.
use std::{
	path::{Path, PathBuf},
	process::{Child, Command},
	time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::Engine;
use gmaps_optimal_placement::core::define::{Area, Category, Definition, Gcid, LatLng};
use sa_auth::{Assertion, Principal, Service, Signer, Study, User};

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
	fn start(name: &str) -> Self {
		let dir = std::env::temp_dir().join(format!("gmaps_conductor_{name}_{}", std::process::id()));
		if dir.exists() {
			std::fs::remove_dir_all(&dir).unwrap(); // a killed run's
		}
		for sub in ["trades", "countries", "site/mfe"] {
			std::fs::create_dir_all(dir.join(sub)).unwrap();
		}
		// no `google_maps`: there is no key to buy with
		std::fs::write(
			dir.join("config.nix"),
			r#"{ age = { communes = "1y"; inventory = "1w"; ordering = "1y"; }; sweep = { depth = 3; zoom = 12; }; queries_cache = "local"; }"#,
		)
		.unwrap();
		std::fs::write(dir.join("site/mfe/overlay.js"), "export async function overlay() {}").unwrap();
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
		let child = Command::new(env!("CARGO_BIN_EXE_gmaps_optimal_placement"))
			.arg("--config")
			.arg(dir.join("config.nix"))
			.args(["serve", "--behind-conductor", "--port", &port.to_string(), "-t"])
			.arg(dir.join("trades"))
			.arg("--countries")
			.arg(dir.join("countries"))
			.env("PANEL_ASSERTION_KEYS", panel.public())
			.env_remove("GMAPS_OPTIMAL_PLACEMENT_QUERIES_CACHE")
			.env_remove("GOOGLE_ADS_DEVELOPER_TOKEN")
			.env("LEPTOS_SITE_ROOT", dir.join("site"))
			.spawn()
			.unwrap();
		let mut served = Self { child, dir, port, panel };
		for _ in 0..240 {
			if served.get("/mfe/overlay.js", None).0 == 200 {
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
			sub: "0192".into(),
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

/// A few hundred Clermont cells, as the conductor would hand them in. Named per run: what is defined
/// is kept in the real work dir, beside the archive these share.
fn definition(study: Option<&str>) -> Definition {
	Definition {
		area: Area::Circle {
			slug: format!("clermont-centre-{}", std::process::id()),
			country: "FR".into(),
			center: LatLng { lat: 45.78, lng: 3.07 },
			radius_m: 1000.,
			margin_m: 200.,
		},
		category: study.map(|s| Category {
			trade: 1,
			country: "FR".into(),
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
	let s = Served::start("guard");
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
	assert_eq!(s.get("/mfe/overlay.js", None).0, 200, "the module is open");
}

/// A definition is addressed by its own hash, defining one buys nothing, and only a pull may spend.
#[test]
fn a_study_is_defined_then_read_and_only_a_pull_spends() {
	let s = Served::start("define");
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
	forget(&id);
}

fn forget(id: &str) {
	std::fs::remove_file(v_utils::xdg_cache_dir!("").join("studies").join(format!("{id}.json"))).unwrap();
}

/// A definition the served trades cannot take is refused when it is handed in, not when it is read.
#[test]
fn a_definition_that_does_not_hold_is_refused_on_put() {
	let s = Served::start("holds");
	let def = definition(Some("not_a_trade"));
	let (status, text) = s.signed("PUT", &format!("/studies/{}", def.id()), &[Study::Read], Some(&serde_json::to_string(&def).unwrap()));
	assert_eq!(status, 400);
	assert!(text.contains("not_a_trade"), "{text}");

	let bare = definition(None);
	let at = format!("/studies/{}", bare.id());
	assert_eq!(s.signed("PUT", &at, &[Study::Read], Some(&serde_json::to_string(&bare).unwrap())).0, 204);
	let (status, text) = s.signed("GET", &at, &[Study::Read], None);
	assert_eq!(status, 200, "{text}");
	let payload: serde_json::Value = serde_json::from_str(&text).unwrap();
	assert!(payload["trade"].is_null() && payload["notice"].is_null(), "an area on its own is the grid, and asks nothing");
	forget(&bare.id());
}
