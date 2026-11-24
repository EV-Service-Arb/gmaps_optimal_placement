#![doc = include_str!("../README.md")]
use std::{
	cell::RefCell,
	collections::HashMap,
	rc::{Rc, Weak},
};

use gmaps_optimal_placement_core::{
	Model, Payload,
	model::{Ages, HAZE, LEVELS, LayerRef, LayerSpec, Selection, Summary, TradeLayer, categories, fmt, haze, layer_id, ramp},
	payload::Scale,
};
use js_sys::Function;
use serde::{Deserialize, Serialize};
use tsify::Tsify;
use wasm_bindgen::prelude::*;

use crate::scene::Scene;

mod scene;

type Hooks = (Closure<dyn Fn(f64, f64, f64, f64)>, Closure<dyn Fn(usize)>);
/// What the host is told: what the shown payload is (on every `show`), what is drawn in view, and
/// why the map is less than asked.
#[derive(Serialize, Tsify)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum Event {
	Data(Option<Summary>),
	Legend(Legend),
	Notice(Notice),
}

/// What is drawn in view. `cells` 0, `levels` null and `in_view` 0 are each a layer that draws
/// nothing.
#[derive(Clone, Deserialize, PartialEq, Serialize, Tsify)]
pub struct Legend {
	pub heat: Option<Heat>,
	pub pressure: Option<Pressure>,
	pub competitors: Option<Competitors>,
	/// How old the answers naming the competitors shown are, as of the study's `show`; null without
	/// one shown.
	pub ages: Option<Ages>,
}

#[derive(Clone, Deserialize, PartialEq, Serialize, Tsify)]
pub struct Heat {
	/// A `Summary.layers[].id`.
	pub id: String,
	pub name: String,
	pub note: String,
	pub scale: Scale,
	pub stops: Vec<String>,
	pub cells: usize,
	pub ticks: Option<Vec<String>>,
	/// The competitor a coverage layer is about.
	pub focus: Option<String>,
}

#[derive(Clone, Deserialize, PartialEq, Serialize, Tsify)]
pub struct Pressure {
	pub stops: Vec<String>,
	pub note: String,
	pub levels: Option<Vec<Level>>,
}

#[derive(Clone, Deserialize, PartialEq, Serialize, Tsify)]
pub struct Level {
	pub q: f64,
	pub value: String,
}

#[derive(Clone, Deserialize, PartialEq, Serialize, Tsify)]
pub struct Competitors {
	pub in_view: usize,
}

/// Why the map is less than asked; a null `text` clears it.
#[derive(Serialize, Tsify)]
pub struct Notice {
	text: Option<String>,
	missing: Option<usize>,
}

#[wasm_bindgen]
pub struct Overlay {
	inner: Rc<RefCell<Inner>>,
}
#[wasm_bindgen]
impl Overlay {
	#[wasm_bindgen(constructor)]
	pub fn new(#[wasm_bindgen(unchecked_param_type = "google.maps.Map")] map: JsValue, #[wasm_bindgen(unchecked_param_type = "(event: Event) => void")] emit: Function) -> Overlay {
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
			let scene = Scene::attach(&map, on_view.as_ref().unchecked_ref(), on_poi.as_ref().unchecked_ref());
			RefCell::new(Inner {
				scene,
				emit,
				model: None,
				noticed: false,
				heat: None,
				tune: Tune::default(),
				now_s: 0.,
				field: false,
				focus: 0,
				dots: false,
				hide_imputed: false,
				press: Vec::new(),
				unmet: None,
				legend: None,
				hooks: Some((on_view, on_poi)),
			})
		});
		Overlay { inner }
	}

	/// A payload as placement wrote it, or none to take the study off the map. One that does not
	/// parse or build a model is a notice. `pulled` is a pull's answer: one carrying a notice was
	/// stopped part-way, and leaves the map on the study it had.
	pub fn show(&self, payload: Option<String>, pulled: bool) {
		run(&self.inner, |i| {
			let Some(payload) = payload else {
				return i.apply(None);
			};
			let built = serde_json::from_str::<Payload>(&payload)
				.map_err(|e| format!("the payload did not parse — {e}"))
				.and_then(|p| Model::try_new(p).map_err(|e| format!("{e:#}")));
			match built {
				Err(e) => vec![i.notice(Some(&e), None)],
				Ok(m) => match (&m.payload.notice, pulled) {
					(Some(n), true) => vec![i.notice(Some(&format!("nothing was replaced — {}", n.text)), n.missing)],
					(n, _) => {
						let mut out: Vec<Event> = match n {
							Some(n) => vec![i.notice(Some(&n.text), n.missing)],
							None => i.noticed.then(|| i.notice(None, None)).into_iter().collect(),
						};
						out.extend(i.apply(Some(Rc::new(m))));
						out
					}
				},
			}
		});
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
		if let Some(t) = self.inner.borrow().model.as_ref().and_then(|m| m.traded()) {
			let all = t.selection();
			let tiers: Vec<String> = all.tiers.into_iter().map(|s| s.name).collect();
			let terms: Vec<String> = t.trade.terms.iter().map(|x| x.text.clone()).collect();
			for (asked, known, what) in [
				(tune.tiers.as_ref().map(|t| t.keys().cloned().collect::<Vec<_>>()), &tiers, "tier"),
				(tune.terms.clone(), &terms, "search"),
				(tune.kinds.clone(), &all.kinds, "kind"),
			] {
				if let Some(stray) = asked.iter().flatten().find(|k| !known.contains(k)) {
					return Err(JsError::new(&format!("tune: the shown study has no {what} {stray:?}, only {known:?}")));
				}
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

	pub fn dispose(&self) {
		let mut i = self.inner.borrow_mut();
		i.scene.detach();
		i.hooks = None;
	}
}

/// `tune()`'s argument. A null is the payload's own; a tier left out keeps its own weight and shows.
#[derive(Clone, Default, Deserialize, Tsify)]
#[serde(deny_unknown_fields)]
pub struct Tune {
	lambda_m: Option<f64>,
	tiers: Option<HashMap<String, TierTune>>,
	/// The searches a competitor's contributions count on, by `Summary.trade.terms[].text`; null:
	/// every one.
	#[serde(default)]
	terms: Option<Vec<String>>,
	/// The Places kinds shown, by `Summary.trade.categories[].kind`; null: every one. A hidden kind
	/// leaves the marks and the pressure alike.
	#[serde(default)]
	kinds: Option<Vec<String>>,
}

#[derive(Clone, Deserialize, Tsify)]
#[serde(deny_unknown_fields)]
pub struct TierTune {
	weight: f64,
	show: bool,
}

struct Inner {
	scene: Scene,
	emit: Function,
	model: Option<Rc<Model>>,
	/// Whether the notice on screen is the shown study's, so the payload after it can clear it.
	noticed: bool,
	heat: Option<String>,
	tune: Tune,
	/// Unix seconds of the study's `show`, which the ages are read as of.
	now_s: f64,
	field: bool,
	/// Which competitor the coverage layers are about. The loudest until one is clicked.
	focus: usize,
	dots: bool,
	hide_imputed: bool,
	/// At `tune`. Moves only with `tune` and the model, never with the view.
	press: Vec<f64>,
	/// None without a demand.
	unmet: Option<Vec<f64>>,
	legend: Option<Legend>,
	/// Kept alive while attached: `paint.js` calls them on settle and on a dot's click.
	hooks: Option<Hooks>,
}
impl Inner {
	fn notice(&mut self, text: Option<&str>, missing: Option<usize>) -> Event {
		self.noticed = text.is_some();
		Event::Notice(Notice {
			text: text.map(str::to_owned),
			missing,
		})
	}

	fn apply(&mut self, model: Option<Rc<Model>>) -> Vec<Event> {
		let changed = match (&model, &self.model) {
			(Some(a), Some(b)) => !Rc::ptr_eq(a, b),
			(a, b) => a.is_some() != b.is_some(),
		};
		self.scene.show(model.iter().cloned().collect());
		if changed {
			self.focus = 0;
			self.now_s = js_sys::Date::now() / 1000.;
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

	/// The payload's selection under `tune`; `None` without a trade.
	fn selection(&self, m: &Model) -> Option<Selection> {
		let t = m.traded()?;
		let mut sel = t.selection();
		if let Some(w) = &self.tune.tiers {
			for s in &mut sel.tiers {
				if let Some(w) = w.get(&s.name) {
					s.weight = w.weight;
					s.show = w.show;
				}
			}
		}
		if let Some(on) = &self.tune.terms {
			sel.terms = t.trade.terms.iter().map(|x| on.contains(&x.text)).collect();
		}
		if let Some(on) = &self.tune.kinds {
			sel.kinds.retain(|k| on.contains(k));
		}
		Some(sel)
	}

	fn lambda(&self, m: &Model) -> Option<f64> {
		Some(self.tune.lambda_m.unwrap_or(m.payload.trade.as_ref()?.lambda_m))
	}

	fn recompute(&mut self) {
		(self.press, self.unmet) = match &self.model {
			Some(m) => match (m.traded(), self.lambda(m), self.selection(m)) {
				(Some(t), Some(lambda), Some(sel)) => {
					let press = t.pressure(lambda, &sel);
					let unmet = t.unmet(&press);
					(press, unmet)
				}
				_ => (vec![0.; m.n()], None),
			},
			None => (Vec::new(), None),
		};
	}

	/// Coloured by Google category, as gop's own map colours them, so the panel's category list is
	/// a legend for what is on the map; what the selection hides off it.
	fn place_dots(&self) {
		let Some((t, sel)) = self.model.as_ref().filter(|_| self.dots).and_then(|m| Some((m.traded()?, self.selection(m)?))) else {
			self.scene.dots("[]");
			return;
		};
		let cats = categories(t.trade);
		self.scene.dots(&scene::dots(t, &sel, |p| {
			cats.iter().find(|c| c.kind == p.poi.kind).expect("the list was built from these same pois").color.clone()
		}));
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
				ages: None,
			});
		};
		let spec: Option<LayerSpec> = m.layers().into_iter().find(|l| Some(layer_id(&l.name)) == self.heat);
		let values: Vec<f64> = match spec.as_ref().map(|s| s.source) {
			None => vec![0.; m.n()],
			Some(LayerRef::Study(i)) => m.layer(i).to_vec(),
			Some(LayerRef::Trade(TradeLayer::Unmet)) => self.unmet.clone().expect("the unmet layer is listed only with a demand"),
			Some(LayerRef::Trade(t)) => {
				let traded = m.traded().expect("a model offers trade layers only with a trade");
				let lambda = self.lambda(&m).expect("and so a λ");
				let sel = self.selection(&m).expect("and so a selection");
				traded.values(t, self.focus, lambda, &sel, &[]).into_owned() // unmet is read for Unmet alone, taken above
			}
		};
		let heat = [values.as_slice()];
		self.scene.heat(spec.as_ref().map(|s| (heat.as_slice(), s.scale)));
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
				scale: s.scale,
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
		let shown = m.traded().zip(self.selection(&m));
		let competitors = shown.as_ref().filter(|_| self.dots).map(|(t, sel)| Competitors {
			in_view: (0..t.trade.pois.len()).filter(|&i| t.counts(sel, i) && self.scene.sees(&t.trade.pois[i].poi)).count(),
		});
		let ages = shown.as_ref().and_then(|(t, sel)| t.ages(sel, self.now_s));
		self.say(Legend { heat, pressure, competitors, ages })
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

/// With no borrow held: the host's `emit` may call straight back in.
fn emit(inner: &Rc<RefCell<Inner>>, events: Vec<Event>) {
	let on = inner.borrow().emit.clone();
	for e in events {
		// through JSON, so a `None` reaches the host as the `null` its type says
		let e = js_sys::JSON::parse(&serde_json::to_string(&e).expect("what is told serialises")).expect("serde_json writes JSON");
		on.call1(&JsValue::NULL, &e).expect("the host's emit does not throw");
	}
}
