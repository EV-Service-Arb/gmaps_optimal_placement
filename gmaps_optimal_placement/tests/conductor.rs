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
use sa_auth::{Assertion, Principal, Service, Signer, Study, User};

fn root() -> &'static Path {
	Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("crate lives in the workspace")
}

/// One server behind a conductor that signs with `panel`, over a narrowed Clermont and two trades
/// whose queries nobody has ever asked Google.
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
		for sub in ["trades", "locations", "site/mfe"] {
			std::fs::create_dir_all(dir.join(sub)).unwrap();
		}
		// no `google_maps`: there is no key to buy with
		std::fs::write(
			dir.join("config.nix"),
			r#"{ age = { communes = "1y"; inventory = "1w"; ordering = "1y"; }; sweep = { depth = 3; root = 3; }; }"#,
		)
		.unwrap();
		std::fs::write(dir.join("site/mfe/overlay.js"), "export async function overlay() {}").unwrap();
		std::fs::write(
			dir.join("locations/Narrow.nix"),
			format!(
				"(import {}) // {{ area = {{ bbox = {{ lat = [ 45.770 45.790 ]; lon = [ 3.050 3.090 ]; }}; center = [ 45.78 3.07 ]; zoom = 14; }}; }}",
				root().join("examples/locations/Clermont-Ferrand.nix").display()
			),
		)
		.unwrap();
		for t in ["never_asked", "never_asked_either"] {
			std::fs::write(
				dir.join(format!("trades/{t}.nix")),
				format!(
					r#"loc: {{
						inherit (loc) area grid candidate country column layer;
						poi = {{ source = "google_places"; queries = [ "gmaps_optimal_placement {t}" ]; tier = [ {{ name = "any"; weight = 1.0; match = "."; }} ]; }};
						model = {{ demand = "ind"; lambda_m = 2000; }};
						rank = {{ nodes = 4; term = [ {{ text = "{t}"; weight = 1.0; }} ]; }};
					}}"#
				),
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
			.arg("-l")
			.arg(dir.join("locations"))
			.env("PANEL_ASSERTION_KEYS", panel.public())
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
		let agent: ureq::Agent = ureq::Agent::config_builder().http_status_as_error(false).build().into();
		let mut req = agent.get(format!("http://127.0.0.1:{}{path}", self.port));
		if let Some(a) = assertion {
			req = req.header(sa_auth::HEADER, a);
		}
		match req.call() {
			Ok(mut r) => (r.status().as_u16(), r.body_mut().read_to_string().unwrap()),
			Err(_) => (0, String::new()), // not listening yet
		}
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
	let path = "/studies.json";
	assert_eq!(s.get(path, None).0, 401, "no assertion");
	let stranger: Signer = format!("k1:{}", base64::engine::general_purpose::STANDARD.encode([9u8; 32])).parse().unwrap();
	assert_eq!(
		s.get(path, Some(&sign(&stranger, "GET", path, &[Study::Read]))).0,
		401,
		"signed by a key the server does not hold"
	);
	assert_eq!(s.get(path, Some(&s.assertion("GET", "/place/Narrow", &[Study::Read]))).0, 401, "signed for another request");
	assert_eq!(s.get(path, Some(&s.assertion("GET", path, &[]))).0, 403, "signed, for someone who may not read studies");

	let (status, body) = s.get(path, Some(&s.assertion("GET", path, &[Study::Read])));
	assert_eq!(status, 200, "{body}");
	let studies: serde_json::Value = serde_json::from_str(&body).unwrap();
	assert_eq!(
		studies["locations"][0],
		serde_json::json!({"name": "Narrow", "bbox": [3.05, 45.77, 3.09, 45.79], "country": "FR"})
	);
	assert_eq!(s.get("/mfe/overlay.js", None).0, 200, "the module is open");
}

/// A GET never spends. A pairing whose searches are not in the work dir opens on what is, and says
/// how many it is missing.
#[test]
fn a_get_names_what_it_did_not_buy() {
	let s = Served::start("missing");
	let path = "/payload/never_asked/Narrow";
	let (status, body) = s.get(path, Some(&s.assertion("GET", path, &[Study::Read])));
	assert_eq!(status, 200, "a refusal to buy is a notice, not an error: {body}");
	let payload: serde_json::Value = serde_json::from_str(&body).unwrap();
	assert!(payload["trade"].is_null(), "the work dir holds none of it");
	// one query over the default 3×3 root tiles, and nothing deeper is known to be needed yet
	assert_eq!(payload["notice"]["missing"], 9);
	let text = payload["notice"]["text"].as_str().unwrap();
	assert!(text.contains("at least 9 Places searches"), "{text}");
}
