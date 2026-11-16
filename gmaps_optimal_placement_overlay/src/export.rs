//! What a host page imports: the one study it asked for, fetched and cached, painted through
//! [`Scene`], and told back through its three callbacks. Nothing here throws for a data problem — a payload
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
	payload::Scale,
};
use js_sys::{Function, Promise};
use serde::{Deserialize, Serialize};
use tsify::Tsify;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::future_to_promise;

use crate::Scene;

/// What a host is told, each through its own callback.
enum Event {
	Data(Option<Summary>),
	Legend(Legend),
	Notice(Notice),
}

/// What is drawn in view. `cells` 0, `levels` null and `in_view` 0 are each a layer that draws
/// nothing.
#[derive(Clone, PartialEq, Serialize, Tsify)]
pub struct Legend {
	heat: Option<Heat>,
	pressure: Option<Pressure>,
	competitors: Option<Competitors>,
}

#[derive(Clone, PartialEq, Serialize, Tsify)]
pub struct Heat {
	/// A `Summary.layers[].id`.
	id: String,
	name: String,
	note: String,
	scale: Scale,
	stops: Vec<String>,
	cells: usize,
	ticks: Option<Vec<String>>,
	/// The competitor a coverage layer is about.
	focus: Option<String>,
}

#[derive(Clone, PartialEq, Serialize, Tsify)]
pub struct Pressure {
	stops: Vec<String>,
	note: String,
	levels: Option<Vec<Level>>,
}

#[derive(Clone, PartialEq, Serialize, Tsify)]
pub struct Level {
	q: f64,
	value: String,
}

#[derive(Clone, PartialEq, Serialize, Tsify)]
pub struct Competitors {
	in_view: usize,
}

/// Why the map is less than asked; a null `text` clears it.
#[derive(Serialize, Tsify)]
pub struct Notice {
	text: Option<String>,
	missing: Option<usize>,
}
type Hooks = (Closure<dyn Fn(f64, f64, f64, f64)>, Closure<dyn Fn(usize)>);

#[wasm_bindgen]
pub struct Overlay {
	inner: Rc<RefCell<Inner>>,
}

struct Inner {
	scene: Scene,
	api: String,
	csrf: Function,
	on_data: Function,
	on_legend: Function,
	on_notice: Function,
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
	legend: Option<Legend>,
	/// Kept alive while attached: `paint.js` calls them on settle and on a dot's click.
	hooks: Option<Hooks>,
}

/// `tune()`'s argument. A null is the payload's own; a tier left out keeps its own weight and shows.
#[derive(Clone, Default, Deserialize, Tsify)]
#[serde(deny_unknown_fields)]
pub struct Tune {
	lambda_m: Option<f64>,
	tiers: Option<HashMap<String, TierTune>>,
}

#[derive(Clone, Deserialize, Tsify)]
#[serde(deny_unknown_fields)]
pub struct TierTune {
	weight: f64,
	show: bool,
}

#[wasm_bindgen]
impl Overlay {
	/// Over the host's map; `api` is where placement's operations are, `csrf` the token a pull
	/// carries.
	#[wasm_bindgen(constructor)]
	pub fn new(
		#[wasm_bindgen(unchecked_param_type = "google.maps.Map")] map: JsValue,
		api: String,
		#[wasm_bindgen(unchecked_param_type = "() => string")] csrf: Function,
		#[wasm_bindgen(unchecked_param_type = "(data: Summary | null) => void")] on_data: Function,
		#[wasm_bindgen(unchecked_param_type = "(legend: Legend) => void")] on_legend: Function,
		#[wasm_bindgen(unchecked_param_type = "(notice: Notice) => void")] on_notice: Function,
	) -> Overlay {
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
				on_data,
				on_legend,
				on_notice,
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
				legend: None,
				hooks: Some((on_view, on_poi)),
			})
		});
		Overlay { inner }
	}

	/// The study the host defined, by its id, or none to take it off the map. Resolves once it has
	/// landed or been noticed; never spends.
	#[wasm_bindgen(unchecked_return_type = "Promise<void>")]
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

	/// The heat layer by its `Summary.layers` id, or none. An id the shown study lacks is the host's mistake.
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

	/// A tier the shown study lacks is the host's mistake.
	pub fn tune(&self, #[wasm_bindgen(unchecked_param_type = "Tune")] t: JsValue) -> Result<(), JsError> {
		let t = String::from(js_sys::JSON::stringify(&t).map_err(|e| JsError::new(&format!("tune: {e:?}")))?);
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
	#[wasm_bindgen(unchecked_return_type = "Promise<void>")]
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
		Event::Notice(Notice {
			text: text.map(str::to_owned),
			missing,
		})
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
		let mut out = vec![Event::Data(self.model.as_ref().map(|m| Summary::of(m)))];
		self.legend = None; // a new study's legend is said even when it reads the same
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
			return self.say(Legend {
				heat: None,
				pressure: None,
				competitors: None,
			});
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
			Heat {
				id: layer_id(&s.name),
				name: s.name,
				note: s.note,
				scale: if s.linear { Scale::Linear } else { Scale::Percentile },
				stops: (0..=10)
					.map(|q| {
						let c = ramp(q as f64 / 10.);
						format!("rgb({:.0},{:.0},{:.0})", c[0], c[1], c[2])
					})
					.collect(),
				cells,
				ticks: (cells > 0).then(|| ticks.map(fmt).to_vec()),
				focus,
			}
		});
		let pressure = (self.field && traded).then(|| Pressure {
			stops: (0..=4).map(|q| format!("rgba({},{},{},{:.3})", HAZE[0], HAZE[1], HAZE[2], haze(q as f64 / 4.))).collect(),
			note: "Competitor pressure, Σ weight × exp(−distance/λ), as a percentile among the cells in view.".to_owned(),
			levels: drawn.levels.map(|l| LEVELS.iter().zip(l).map(|(&q, v)| Level { q, value: fmt(v) }).collect()),
		});
		let competitors = (self.dots && traded).then(|| {
			let shown = self.tiers(&m);
			let t = m.payload.trade.as_ref().expect("traded");
			let in_view = t
				.pois
				.iter()
				.filter(|p| shown.iter().any(|s| s.show && s.name == p.poi.tier) && self.scene.sees(p.poi.lat, p.poi.lng))
				.count();
			Competitors { in_view }
		});
		self.say(Legend { heat, pressure, competitors })
	}

	/// The legend, unless it reads as the last one did.
	fn say(&mut self, legend: Legend) -> Vec<Event> {
		match self.legend.as_ref() == Some(&legend) {
			true => Vec::new(),
			false => {
				self.legend = Some(legend.clone());
				vec![Event::Legend(legend)]
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
	for e in events {
		let (on, json) = {
			let i = inner.borrow();
			match &e {
				Event::Data(d) => (i.on_data.clone(), serde_json::to_string(d)),
				Event::Legend(l) => (i.on_legend.clone(), serde_json::to_string(l)),
				Event::Notice(n) => (i.on_notice.clone(), serde_json::to_string(n)),
			}
		};
		// through JSON, so a `None` reaches the host as the `null` its type says
		let detail = js_sys::JSON::parse(&json.expect("what is told serialises")).expect("serde_json writes JSON");
		on.call1(&JsValue::NULL, &detail).expect("a host's callback does not throw");
	}
}
