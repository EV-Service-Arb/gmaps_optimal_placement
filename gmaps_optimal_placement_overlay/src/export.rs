//! What `overlay.js` wraps: the studies a host asked for, fetched and cached, painted through
//! [`Scene`], and told back as events. Nothing here throws for a data problem — a payload that does
//! not arrive, does not parse or does not build a model is a `notice`.
//!
//! Events are emitted with no borrow held: a host listener may call straight back in.
use std::{
	cell::RefCell,
	collections::{HashMap, HashSet},
	rc::{Rc, Weak},
};

use gmaps_optimal_placement_core::{
	Model, Payload,
	model::{HAZE, LEVELS, LayerRef, LayerSpec, TierState, TradeLayer, fmt, haze, ramp},
};
use js_sys::{Function, Promise};
use serde::Deserialize;
use serde_json::json;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, future_to_promise};

use crate::Scene;

/// A trade stem, or none for the location alone, and a location stem.
type Key = (Option<String>, String);
type Event = (&'static str, String);
type Hooks = (Closure<dyn Fn(f64, f64, f64, f64)>, Closure<dyn Fn(usize)>);

#[wasm_bindgen]
pub struct Overlay {
	inner: Rc<RefCell<Inner>>,
}

struct Inner {
	scene: Scene,
	api: String,
	csrf: Function,
	emit: Function,
	cache: HashMap<Key, Rc<Model>>,
	/// Whose last payload carried a notice, so the one after that clears it says so.
	noticed: HashSet<Key>,
	/// The latest `show`; an older one still landing only fills the cache.
	generation: u32,
	shown: Vec<Key>,
	/// The shown studies that have a model, in `shown` order — the order `scene` was handed.
	models: Vec<(Key, Rc<Model>)>,
	heat: Option<String>,
	opts: Opts,
	field: bool,
	/// Which competitor the coverage layers are about, per study. The loudest until one is clicked.
	focus: HashMap<Key, usize>,
	dots: bool,
	hide_imputed: bool,
	/// Per model, at `opts`. Moves only with `opts` and the models, never with the view.
	press: Vec<Vec<f64>>,
	unmet: Vec<Vec<f64>>,
	legend: String,
	/// Kept alive while attached: `paint.js` calls them on settle and on a dot's click.
	hooks: Option<Hooks>,
}

/// `pressure()`'s argument. An omitted field is the payload's own.
#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Opts {
	lambda: Option<f64>,
	tiers: Option<HashMap<String, f64>>,
}

#[wasm_bindgen]
impl Overlay {
	#[wasm_bindgen(constructor)]
	pub fn new(map: JsValue, api: String, csrf: Function, emit: Function) -> Overlay {
		let inner = Rc::new_cyclic(|me: &Weak<RefCell<Inner>>| {
			let (v, p) = (me.clone(), me.clone());
			let on_view = Closure::<dyn Fn(f64, f64, f64, f64)>::new(move |w, s, e, n| {
				let Some(inner) = v.upgrade() else { return };
				run(&inner, |i| {
					i.scene.view([w, s, e, n]);
					i.repaint()
				});
			});
			let on_poi = Closure::<dyn Fn(usize)>::new(move |at: usize| {
				let Some(inner) = p.upgrade() else { return };
				run(&inner, |i| i.focus(at));
			});
			let scene = Scene::attach(&map, on_view.as_ref().unchecked_ref(), Some(on_poi.as_ref().clone().unchecked_into()), None, None);
			RefCell::new(Inner {
				scene,
				api,
				csrf,
				emit,
				cache: HashMap::new(),
				noticed: HashSet::new(),
				generation: 0,
				shown: Vec::new(),
				models: Vec::new(),
				heat: None,
				opts: Opts::default(),
				field: false,
				focus: HashMap::new(),
				dots: false,
				hide_imputed: false,
				press: Vec::new(),
				unmet: Vec::new(),
				legend: String::new(),
				hooks: Some((on_view, on_poi)),
			})
		});
		Overlay { inner }
	}

	/// Resolves once every study asked for has landed or been noticed. Never spends: GETs only.
	pub fn show(&self, trade: Option<String>, locations: Vec<String>) -> Promise {
		let inner = self.inner.clone();
		future_to_promise(async move {
			let keys: Vec<Key> = locations.into_iter().map(|l| (trade.clone(), l)).collect();
			let generation = {
				let mut i = inner.borrow_mut();
				i.generation += 1;
				i.shown = keys.clone();
				i.generation
			};
			let missing: Vec<Key> = keys.iter().filter(|k| !inner.borrow().cache.contains_key(*k)).cloned().collect();
			let fetches = js_sys::Array::new();
			for key in missing {
				let inner = inner.clone();
				fetches.push(&future_to_promise(async move {
					if let Err(e) = fetch(&inner, &key, false).await {
						let ev = inner.borrow_mut().notice(&key, Some(&e), None);
						emit(&inner, vec![ev]);
					}
					Ok(JsValue::UNDEFINED)
				}));
			}
			JsFuture::from(Promise::all(&fetches)).await?;
			if inner.borrow().generation == generation {
				run(&inner, Inner::apply);
			}
			Ok(JsValue::UNDEFINED)
		})
	}

	/// The heat layer by its `layers` id, or none. An id no shown study offers is the host's mistake.
	pub fn heat(&self, id: Option<String>) -> Result<(), JsError> {
		if let Some(id) = &id
			&& !self.inner.borrow().models.iter().any(|(_, m)| m.layers().iter().any(|l| slug(&l.name) == *id))
		{
			return Err(JsError::new(&format!("no shown study offers a layer {id:?}")));
		}
		run(&self.inner, |i| {
			i.heat = id;
			i.repaint()
		});
		Ok(())
	}

	/// `opts` is `pressure()`'s argument as JSON; `None` hides the field and keeps λ and the weights
	/// it last set, which the underserved layer still reads.
	pub fn pressure(&self, opts: Option<String>) -> Result<(), JsError> {
		let opts = opts
			.map(|o| serde_json::from_str::<Opts>(&o).map_err(|e| JsError::new(&format!("pressure({o}): {e}"))))
			.transpose()?;
		run(&self.inner, |i| {
			i.field = opts.is_some();
			if let Some(o) = opts {
				i.opts = o;
				i.recompute();
			}
			i.repaint()
		});
		Ok(())
	}

	pub fn competitors(&self, on: bool) {
		run(&self.inner, |i| {
			i.dots = on;
			i.place_dots();
			Vec::new()
		});
	}

	pub fn opacity(&self, x: f64) {
		self.inner.borrow().scene.opacity(x);
	}

	#[wasm_bindgen(js_name = hideImputed)]
	pub fn hide_imputed(&self, on: bool) {
		run(&self.inner, |i| {
			i.hide_imputed = on;
			i.scene.hide_imputed(on);
			i.repaint()
		});
	}

	/// Asks Google again for every shown study under a trade, and repaints. Rejects only when a
	/// request got no response at all.
	pub fn pull(&self) -> Promise {
		let inner = self.inner.clone();
		future_to_promise(async move {
			let keys: Vec<Key> = inner.borrow().shown.iter().filter(|k| k.0.is_some()).cloned().collect();
			for key in keys {
				fetch(&inner, &key, true).await.map_err(|e| JsValue::from(JsError::new(&e)))?;
			}
			run(&inner, Inner::apply);
			Ok(JsValue::UNDEFINED)
		})
	}

	pub fn dispose(&self) {
		let mut i = self.inner.borrow_mut();
		i.scene.detach();
		i.hooks = None;
	}
}

/// One study over the network into the cache. `Err` is a request that got no response; every other
/// failure is a notice.
async fn fetch(inner: &Rc<RefCell<Inner>>, key: &Key, post: bool) -> Result<(), String> {
	let (url, csrf) = {
		let i = inner.borrow();
		let l = String::from(js_sys::encode_uri_component(&key.1));
		let url = match &key.0 {
			Some(t) => format!("{}/payload/{}/{l}", i.api, String::from(js_sys::encode_uri_component(t))),
			None => format!("{}/place/{l}", i.api),
		};
		(url, i.csrf.clone())
	};
	let req = match post {
		true => {
			let token = csrf.call0(&JsValue::NULL).map_err(|e| format!("csrf() threw: {e:?}"))?;
			let token = token.as_string().ok_or_else(|| format!("csrf() returned {token:?}, not a string"))?;
			gloo_net::http::Request::post(&url).header("x-sa-csrf", &token)
		}
		false => gloo_net::http::Request::get(&url),
	};
	let res = req.send().await.map_err(|e| format!("{url}: {e}"))?;
	let text = res.text().await.map_err(|e| format!("{url}: {e}"))?;
	let built = match res.ok() {
		false => Err(format!("{} {}: {text}", res.status(), res.status_text())),
		true => serde_json::from_str::<Payload>(&text)
			.map_err(|e| format!("the payload did not parse — {e}"))
			.and_then(|p| Model::try_new(p).map_err(|e| format!("{e:#}"))),
	};
	let events = {
		let mut i = inner.borrow_mut();
		match built {
			Err(e) => vec![i.notice(key, Some(&e), None)],
			Ok(m) => match (&m.payload.notice, post) {
				// a refresh the quota or the work dir stopped leaves the study on what it had
				(Some(n), true) => vec![i.notice(key, Some(&format!("nothing was replaced — {}", n.text)), n.missing)],
				(n, _) => {
					let ev = match n {
						Some(n) => Some(i.notice(key, Some(&n.text), n.missing)),
						None => i.noticed.contains(key).then(|| i.notice(key, None, None)),
					};
					i.cache.insert(key.clone(), Rc::new(m));
					ev.into_iter().collect()
				}
			},
		}
	};
	emit(inner, events);
	Ok(())
}

impl Inner {
	fn notice(&mut self, key: &Key, text: Option<&str>, missing: Option<usize>) -> Event {
		match text {
			Some(_) => self.noticed.insert(key.clone()),
			None => self.noticed.remove(key),
		};
		("notice", json!({"location": key.1, "trade": key.0, "text": text, "missing": missing}).to_string())
	}

	/// The shown set, out of the cache and onto the map.
	fn apply(&mut self) -> Vec<Event> {
		self.models = self.shown.iter().filter_map(|k| Some((k.clone(), self.cache.get(k)?.clone()))).collect();
		self.scene.show(self.models.iter().map(|(_, m)| m.clone()).collect());
		self.recompute();
		self.place_dots();
		let mut out = vec![self.layers()];
		out.extend(self.repaint());
		out
	}

	fn tiers(&self, m: &Model) -> Vec<TierState> {
		let mut t = m.tier_states();
		if let Some(w) = &self.opts.tiers {
			for s in &mut t {
				if let Some(w) = w.get(&s.name) {
					s.weight = *w;
				}
			}
		}
		t
	}

	fn lambda(&self, m: &Model) -> Option<f64> {
		Some(self.opts.lambda.unwrap_or(m.payload.trade.as_ref()?.lambda_m))
	}

	fn recompute(&mut self) {
		(self.press, self.unmet) = self
			.models
			.iter()
			.map(|(_, m)| match (m.traded(), self.lambda(m)) {
				(Some(t), Some(lambda)) => {
					let press = t.pressure(lambda, &self.tiers(m));
					let unmet = t.unmet(&press);
					(press, unmet)
				}
				_ => (vec![0.; m.n()], vec![0.; m.n()]),
			})
			.unzip();
	}

	fn place_dots(&self) {
		let json = match self.dots {
			// blue, like the pressure they make; the first tier is direct competition
			true => crate::dots(&self.models.iter().map(|(_, m)| m.as_ref()).collect::<Vec<_>>(), |_, _, tier| match tier {
				0 => format!("rgb({},{},{})", HAZE[0], HAZE[1], HAZE[2]),
				_ => "#93c5fd".to_owned(),
			}),
			false => "[]".to_owned(),
		};
		self.scene.dots(&json);
	}

	/// A dot was clicked: `at` indexes every shown study's competitors, in order.
	fn focus(&mut self, mut at: usize) -> Vec<Event> {
		for (key, m) in &self.models {
			let n = m.payload.trade.as_ref().map_or(0, |t| t.pois.len());
			if at < n {
				self.focus.insert(key.clone(), at);
				return self.repaint();
			}
			at -= n;
		}
		unreachable!("paint.js only reports dots it was handed")
	}

	/// What the shown studies offer, as a union in payload order: trade layers, then location ones.
	fn layers(&self) -> Event {
		let mut seen = HashSet::new();
		let mut layers = Vec::new();
		for trade in [true, false] {
			for (_, m) in &self.models {
				for l in m.layers() {
					let id = slug(&l.name);
					if matches!(l.source, LayerRef::Trade(_)) == trade && seen.insert(id.clone()) {
						layers.push(json!({"id": id, "name": l.name, "note": l.note, "trade": trade}));
					}
				}
			}
		}
		let mut tiers: Vec<serde_json::Value> = Vec::new();
		let mut named = HashSet::new();
		for t in self.models.iter().filter_map(|(_, m)| m.payload.trade.as_ref()).flat_map(|t| &t.tiers) {
			if named.insert(t.name.clone()) {
				tiers.push(json!({"name": t.name, "weight": t.weight}));
			}
		}
		let lambda = self.models.iter().find_map(|(_, m)| Some(m.payload.trade.as_ref()?.lambda_m));
		("layers", json!({"layers": layers, "tiers": tiers, "lambda_m": lambda}).to_string())
	}

	fn repaint(&mut self) -> Vec<Event> {
		let spec = |m: &Model| -> Option<LayerSpec> { m.layers().into_iter().find(|l| Some(slug(&l.name)) == self.heat) };
		let values: Vec<Vec<f64>> = self
			.models
			.iter()
			.zip(&self.unmet)
			.map(|((key, m), unmet)| match spec(m).map(|s| s.source) {
				None => vec![0.; m.n()],
				Some(LayerRef::Study(i)) => m.layer(i).to_vec(),
				Some(LayerRef::Trade(t)) => {
					let traded = m.traded().expect("a model offers trade layers only with a trade");
					let lambda = self.lambda(m).expect("and so a λ");
					let of = self.focus.get(key).copied().unwrap_or(0); // the loudest name, as the Leptos map opens on
					traded.values(t, of, lambda, &self.tiers(m), unmet).into_owned()
				}
			})
			.collect();
		let picked = self.models.iter().find_map(|(_, m)| spec(m));
		let refs: Vec<&[f64]> = values.iter().map(Vec::as_slice).collect();
		self.scene.heat(picked.as_ref().map(|s| (refs.as_slice(), s.linear)));
		let traded = self.models.iter().any(|(_, m)| m.traded().is_some());
		let press: Vec<&[f64]> = self.press.iter().map(Vec::as_slice).collect();
		self.scene.field((self.field && traded).then_some(press.as_slice()));
		let Some(drawn) = self.scene.draw() else { return Vec::new() };

		let heat = picked.zip(drawn.heat).map(|(s, (ticks, count))| {
			let focus = matches!(s.source, LayerRef::Trade(TradeLayer::Share | TradeLayer::Seen))
				.then(|| {
					self.models.iter().filter_map(|(key, m)| {
						let t = m.payload.trade.as_ref()?;
						Some(t.pois.get(self.focus.get(key).copied().unwrap_or(0))?.poi.name.clone())
					})
				})
				.into_iter()
				.flatten()
				.collect::<Vec<_>>();
			let note = match focus.is_empty() {
				true => s.note.clone(),
				false => format!("{} Showing {}.", s.note, focus.join(", ")),
			};
			json!({
				"id": slug(&s.name), "name": s.name, "note": note,
				"scale": if s.linear { "linear" } else { "percentile" },
				"stops": (0..=10).map(|q| { let c = ramp(q as f64 / 10.); format!("rgb({:.0},{:.0},{:.0})", c[0], c[1], c[2]) }).collect::<Vec<_>>(),
				"ticks": match count { 0 => vec!["–".to_owned(); 5], _ => ticks.map(fmt).to_vec() },
			})
		});
		let pressure = (self.field && traded).then(|| {
			let lines = match drawn.levels {
				Some(l) => format!(
					"Lines at the {} percentiles: {}.",
					LEVELS.map(|q| format!("{:.0}th", q * 100.)).join(", "),
					l.map(fmt).join(" · ")
				),
				None => "Nothing in view is under pressure.".to_owned(),
			};
			json!({
				"stops": (0..=4).map(|q| format!("rgba({},{},{},{:.3})", HAZE[0], HAZE[1], HAZE[2], haze(q as f64 / 4.))).collect::<Vec<_>>(),
				"note": format!("Competitor pressure, Σ weight × exp(−distance/λ), as a percentile among the cells in view. {lines}"),
			})
		});
		let legend = json!({"heat": heat, "pressure": pressure}).to_string();
		match legend == self.legend {
			true => Vec::new(),
			false => {
				self.legend = legend.clone();
				vec![("legend", legend)]
			}
		}
	}
}

/// `f` under the borrow, its events after it.
fn run(inner: &Rc<RefCell<Inner>>, f: impl FnOnce(&mut Inner) -> Vec<Event>) {
	let events = f(&mut inner.borrow_mut());
	emit(inner, events);
}

fn emit(inner: &Rc<RefCell<Inner>>, events: Vec<Event>) {
	let emit = inner.borrow().emit.clone();
	for (kind, detail) in events {
		emit.call2(&JsValue::NULL, &JsValue::from_str(kind), &JsValue::from_str(&detail))
			.expect("dispatchEvent reports a listener's throw rather than raising it");
	}
}

/// Lowercase ASCII, every run of anything else one `-`, none at either end.
fn slug(name: &str) -> String {
	let mut out = String::new();
	for c in name.chars() {
		match c.is_ascii_alphanumeric() {
			true => out.push(c.to_ascii_lowercase()),
			false if !out.is_empty() && !out.ends_with('-') => out.push('-'),
			false => {}
		}
	}
	out.trim_end_matches('-').to_owned()
}
