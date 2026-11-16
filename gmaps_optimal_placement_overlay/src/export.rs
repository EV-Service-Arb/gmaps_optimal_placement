//! What `overlay.js` wraps: the one study a host asked for, fetched and cached, painted
//! through [`Scene`], and told back as events. Nothing here throws for a data problem — a payload
//! that does not arrive, does not parse or does not build a model is a `notice`.
//!
//! Every event describes what is on the map now, so none carries a key: a `show` overtaken by a later
//! one fills the cache and says nothing.
//!
//! Events are emitted with no borrow held: a host listener may call straight back in.
use std::{
	cell::RefCell,
	collections::HashMap,
	rc::{Rc, Weak},
};

use gmaps_optimal_placement_core::{
	Model, Payload,
	model::{HAZE, LEVELS, LayerRef, LayerSpec, Summary, TierState, TradeLayer, categories, fmt, haze, layer_id, ramp},
};
use js_sys::{Function, Promise};
use serde::Deserialize;
use serde_json::json;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

use crate::Scene;

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
	/// By definition id. A study once fetched is not fetched again this page session but by a pull.
	cache: HashMap<String, Rc<Model>>,
	/// The latest `show`; an older one still landing only fills the cache.
	generation: u32,
	shown: Option<String>,
	model: Option<Rc<Model>>,
	/// Whether the notice on screen is the shown study's, so the payload after it can clear it.
	noticed: bool,
	heat: Option<String>,
	tune: Tune,
	field: bool,
	/// Which competitor the coverage layers are about. The loudest until one is clicked.
	focus: usize,
	dots: bool,
	hide_imputed: bool,
	/// At `tune`. Moves only with `tune` and the model, never with the view.
	press: Vec<f64>,
	unmet: Vec<f64>,
	legend: String,
	/// Kept alive while attached: `paint.js` calls them on settle and on a dot's click.
	hooks: Option<Hooks>,
}

/// `tune()`'s argument. A null is the payload's own; a tier left out keeps its own weight and shows.
#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Tune {
	lambda_m: Option<f64>,
	tiers: Option<HashMap<String, TierTune>>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TierTune {
	weight: f64,
	show: bool,
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
				run(&inner, |i| {
					i.focus = at;
					i.repaint()
				});
			});
			let scene = Scene::attach(&map, on_view.as_ref().unchecked_ref(), Some(on_poi.as_ref().clone().unchecked_into()), None, None);
			RefCell::new(Inner {
				scene,
				api,
				csrf,
				emit,
				cache: HashMap::new(),
				generation: 0,
				shown: None,
				model: None,
				noticed: false,
				heat: None,
				tune: Tune::default(),
				field: false,
				focus: 0,
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

	/// The study the host defined, by its id, or none to take it off the map. Resolves once it has
	/// landed or been noticed; never spends.
	pub fn show(&self, id: Option<String>) -> Promise {
		let inner = self.inner.clone();
		future_to_promise(async move {
			let generation = {
				let mut i = inner.borrow_mut();
				i.generation += 1;
				i.shown = id.clone();
				i.generation
			};
			if let Some(id) = &id
				&& !inner.borrow().cache.contains_key(id)
				&& let Err(e) = fetch(&inner, id).await
			{
				let ev = inner.borrow_mut().notice(Some(&e), None);
				if inner.borrow().generation == generation {
					emit(&inner, vec![ev]);
				}
			}
			if inner.borrow().generation == generation {
				run(&inner, Inner::apply);
			}
			Ok(JsValue::UNDEFINED)
		})
	}

	/// The heat layer by its `data.layers` id, or none. An id the shown study lacks is the host's mistake.
	pub fn heat(&self, id: Option<String>) -> Result<(), JsError> {
		if let Some(id) = &id
			&& !self.inner.borrow().model.as_ref().is_some_and(|m| m.layers().iter().any(|l| layer_id(&l.name) == *id))
		{
			return Err(JsError::new(&format!("the shown study offers no layer {id:?}")));
		}
		run(&self.inner, |i| {
			i.heat = id;
			i.repaint()
		});
		Ok(())
	}

	/// `t` is `tune()`'s argument as JSON. A tier the shown study lacks is the host's mistake.
	pub fn tune(&self, t: String) -> Result<(), JsError> {
		let tune: Tune = serde_json::from_str(&t).map_err(|e| JsError::new(&format!("tune({t}): {e}")))?;
		if let (Some(m), Some(tiers)) = (&self.inner.borrow().model, &tune.tiers) {
			let known: Vec<String> = m.tier_states().into_iter().map(|s| s.name).collect();
			if let Some(stray) = tiers.keys().find(|k| !known.contains(k)) {
				return Err(JsError::new(&format!("tune: the shown study has no tier {stray:?}, only {known:?}")));
			}
		}
		run(&self.inner, |i| {
			i.tune = tune;
			i.recompute();
			i.place_dots();
			i.repaint()
		});
		Ok(())
	}

	pub fn pressure(&self, on: bool) {
		run(&self.inner, |i| {
			i.field = on;
			i.repaint()
		});
	}

	pub fn competitors(&self, on: bool) {
		run(&self.inner, |i| {
			i.dots = on;
			i.place_dots();
			i.repaint()
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

	/// Asks Google again for everything the shown study is built from, and repaints. Rejects only
	/// when a request got no response at all.
	pub fn pull(&self) -> Promise {
		let inner = self.inner.clone();
		future_to_promise(async move {
			let shown = inner.borrow().shown.clone();
			let Some(id) = shown else { return Err(JsError::new("pull with no study shown").into()) };
			if !inner.borrow().cache.contains_key(&id) {
				return Err(JsError::new("pull before the shown study has landed").into());
			}
			pull(&inner, &id).await.map_err(|e| JsValue::from(JsError::new(&e)))?;
			if inner.borrow().shown.as_deref() == Some(&id) {
				run(&inner, Inner::apply);
			}
			Ok(JsValue::UNDEFINED)
		})
	}

	pub fn dispose(&self) {
		let mut i = self.inner.borrow_mut();
		i.scene.detach();
		i.hooks = None;
	}
}

/// GET the study into the cache. `Err` is a request that got no response; every other failure is a
/// notice.
async fn fetch(inner: &Rc<RefCell<Inner>>, id: &str) -> Result<(), String> {
	let url = format!("{}/studies/{id}", inner.borrow().api);
	land(inner, id, gloo_net::http::Request::get(&url), false).await
}

async fn pull(inner: &Rc<RefCell<Inner>>, id: &str) -> Result<(), String> {
	let url = format!("{}/studies/{id}/pull", inner.borrow().api);
	land(inner, id, gloo_net::http::Request::post(&url).header("x-sa-csrf", &csrf(inner)?), true).await
}

fn csrf(inner: &Rc<RefCell<Inner>>) -> Result<String, String> {
	let csrf = inner.borrow().csrf.clone();
	let token = csrf.call0(&JsValue::NULL).map_err(|e| format!("csrf() threw: {e:?}"))?;
	token.as_string().ok_or_else(|| format!("csrf() returned {token:?}, not a string"))
}

/// One payload over the network into the cache, and the notice it carries said.
async fn land(inner: &Rc<RefCell<Inner>>, id: &str, req: gloo_net::http::RequestBuilder, pulled: bool) -> Result<(), String> {
	let res = req.send().await.map_err(|e| format!("{e}"))?;
	let text = res.text().await.map_err(|e| format!("{e}"))?;
	let built = match res.ok() {
		false => Err(format!("{} {}: {text}", res.status(), res.status_text())),
		true => serde_json::from_str::<Payload>(&text)
			.map_err(|e| format!("the payload did not parse — {e}"))
			.and_then(|p| Model::try_new(p).map_err(|e| format!("{e:#}"))),
	};
	let events = {
		let mut i = inner.borrow_mut();
		let current = i.shown.as_deref() == Some(id);
		let ev = match built {
			Err(e) => Some(i.notice(Some(&e), None)),
			Ok(m) => match (&m.payload.notice, pulled) {
				// a refresh the quota or the work dir stopped leaves the study on what it had
				(Some(n), true) => Some(i.notice(Some(&format!("nothing was replaced — {}", n.text)), n.missing)),
				(n, _) => {
					let ev = match n {
						Some(n) => Some(i.notice(Some(&n.text), n.missing)),
						None => i.noticed.then(|| i.notice(None, None)),
					};
					i.cache.insert(id.to_owned(), Rc::new(m));
					ev
				}
			},
		};
		ev.filter(|_| current).into_iter().collect()
	};
	emit(inner, events);
	Ok(())
}

impl Inner {
	fn notice(&mut self, text: Option<&str>, missing: Option<usize>) -> Event {
		self.noticed = text.is_some();
		("notice", json!({"text": text, "missing": missing}).to_string())
	}

	/// The shown study, out of the cache and onto the map.
	fn apply(&mut self) -> Vec<Event> {
		let model = self.shown.as_ref().and_then(|id| self.cache.get(id)).cloned();
		let changed = match (&model, &self.model) {
			(Some(a), Some(b)) => !Rc::ptr_eq(a, b),
			(a, b) => a.is_some() != b.is_some(),
		};
		self.scene.show(model.iter().cloned().collect());
		if changed {
			self.focus = 0;
		}
		self.model = model;
		if self
			.heat
			.as_ref()
			.is_some_and(|h| !self.model.as_ref().is_some_and(|m| m.layers().iter().any(|l| layer_id(&l.name) == *h)))
		{
			self.heat = None;
		}
		self.recompute();
		self.place_dots();
		let data = match &self.model {
			Some(m) => serde_json::to_string(&Summary::of(m)).expect("a summary serialises"),
			None => "null".to_owned(),
		};
		let mut out = vec![("data", data)];
		self.legend.clear(); // a new study's legend is said even when it reads the same
		out.extend(self.repaint());
		out
	}

	fn tiers(&self, m: &Model) -> Vec<TierState> {
		let mut t = m.tier_states();
		if let Some(w) = &self.tune.tiers {
			for s in &mut t {
				if let Some(w) = w.get(&s.name) {
					s.weight = w.weight;
					s.show = w.show;
				}
			}
		}
		t
	}

	fn lambda(&self, m: &Model) -> Option<f64> {
		Some(self.tune.lambda_m.unwrap_or(m.payload.trade.as_ref()?.lambda_m))
	}

	fn recompute(&mut self) {
		(self.press, self.unmet) = match &self.model {
			Some(m) => match (m.traded(), self.lambda(m)) {
				(Some(t), Some(lambda)) => {
					let press = t.pressure(lambda, &self.tiers(m));
					let unmet = t.unmet(&press);
					(press, unmet)
				}
				_ => (vec![0.; m.n()], vec![0.; m.n()]),
			},
			None => (Vec::new(), Vec::new()),
		};
	}

	/// Coloured by Google category, as gop's own map colours them, so the panel's category list is
	/// a legend for what is on the map; hidden tiers off it.
	fn place_dots(&self) {
		let Some(m) = self.model.as_ref().filter(|_| self.dots) else {
			self.scene.dots("[]");
			return;
		};
		let cats = m.payload.trade.iter().flat_map(categories).collect::<Vec<_>>();
		self.scene.dots(&crate::dots(&[m.as_ref()], |_, p, _| {
			cats.iter().find(|c| c.kind == p.poi.kind).expect("the list was built from these same pois").color.clone()
		}));
		self.scene.tiers(&self.tiers(m).iter().map(|t| u8::from(t.show)).collect::<Vec<_>>());
	}

	fn repaint(&mut self) -> Vec<Event> {
		let Some(m) = self.model.clone() else {
			self.scene.heat(None);
			self.scene.field(None);
			self.scene.draw();
			return self.say(json!({"heat": null, "pressure": null, "competitors": null}));
		};
		let spec: Option<LayerSpec> = m.layers().into_iter().find(|l| Some(layer_id(&l.name)) == self.heat);
		let values: Vec<f64> = match spec.as_ref().map(|s| s.source) {
			None => vec![0.; m.n()],
			Some(LayerRef::Study(i)) => m.layer(i).to_vec(),
			Some(LayerRef::Trade(t)) => {
				let traded = m.traded().expect("a model offers trade layers only with a trade");
				let lambda = self.lambda(&m).expect("and so a λ");
				traded.values(t, self.focus, lambda, &self.tiers(&m), &self.unmet).into_owned()
			}
		};
		let heat = [values.as_slice()];
		self.scene.heat(spec.as_ref().map(|s| (heat.as_slice(), s.linear)));
		let traded = m.traded().is_some();
		let press = [self.press.as_slice()];
		self.scene.field((self.field && traded).then_some(press.as_slice()));
		let Some(drawn) = self.scene.draw() else { return Vec::new() };

		let heat = spec.zip(drawn.heat).map(|(s, (ticks, cells))| {
			let focus = matches!(s.source, LayerRef::Trade(TradeLayer::Share | TradeLayer::Seen))
				.then(|| Some(m.payload.trade.as_ref()?.pois.get(self.focus)?.poi.name.clone()))
				.flatten();
			json!({
				"id": layer_id(&s.name), "name": s.name, "note": s.note,
				"scale": if s.linear { "linear" } else { "percentile" },
				"stops": (0..=10).map(|q| { let c = ramp(q as f64 / 10.); format!("rgb({:.0},{:.0},{:.0})", c[0], c[1], c[2]) }).collect::<Vec<_>>(),
				"cells": cells,
				"ticks": (cells > 0).then(|| ticks.map(fmt)),
				"focus": focus,
			})
		});
		let pressure = (self.field && traded).then(|| {
			json!({
				"stops": (0..=4).map(|q| format!("rgba({},{},{},{:.3})", HAZE[0], HAZE[1], HAZE[2], haze(q as f64 / 4.))).collect::<Vec<_>>(),
				"note": "Competitor pressure, Σ weight × exp(−distance/λ), as a percentile among the cells in view.",
				"levels": drawn.levels.map(|l| LEVELS.iter().zip(l).map(|(q, v)| json!({"q": q, "value": fmt(v)})).collect::<Vec<_>>()),
			})
		});
		let competitors = (self.dots && traded).then(|| {
			let shown = self.tiers(&m);
			let t = m.payload.trade.as_ref().expect("traded");
			let in_view = t
				.pois
				.iter()
				.filter(|p| shown.iter().any(|s| s.show && s.name == p.poi.tier) && self.scene.sees(p.poi.lat, p.poi.lng))
				.count();
			json!({"in_view": in_view})
		});
		self.say(json!({"heat": heat, "pressure": pressure, "competitors": competitors}))
	}

	/// The legend, unless it reads as the last one did.
	fn say(&mut self, legend: serde_json::Value) -> Vec<Event> {
		let legend = legend.to_string();
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
