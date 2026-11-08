#![doc = include_str!("../README.md")]
use std::rc::Rc;

use gmaps_optimal_placement_core::{
	Model, PoiOut,
	model::{Field, HAZE, colorise, field},
};
use js_sys::Function;
use wasm_bindgen::prelude::*;

#[cfg(feature = "export")]
mod export;

#[wasm_bindgen(module = "/src/paint.js")]
extern "C" {
	fn attach(map: &JsValue, on_view: &Function, on_poi: Option<Function>, on_over: Option<Function>, on_out: Option<Function>) -> JsValue;
	fn detach(s: &JsValue);
	fn geometry(s: &JsValue, ring_x: &[f64], ring_y: &[f64]);
	fn cells(s: &JsValue, colors: &[u8], shown: &[u8]);
	#[wasm_bindgen(js_name = field)]
	fn field_js(s: &JsValue, haze: &str, alpha: &[u8], segs: &[f64]);
	fn opacity(s: &JsValue, v: f64);
	#[wasm_bindgen(js_name = dots)]
	fn dots_js(s: &JsValue, json: &str);
	fn tiers(s: &JsValue, flags: &[u8]);
}

/// What is painted over one `google.maps.Map`: the heat, the pressure field and the competitor dots
/// of every study shown, as one set of cells. Every number is decided here; `paint.js` projects and
/// fills.
///
/// Both scales are relative to the cells in view, so nothing is drawn until the map has said what
/// that is.
pub struct Scene {
	js: JsValue,
	models: Vec<Rc<Model>>,
	c_lat: Vec<f64>,
	c_lng: Vec<f64>,
	imputed: Vec<u8>,
	view: Option<[f64; 4]>,
	heat: Option<(Vec<f64>, bool)>,
	press: Option<Vec<Vec<f64>>>,
	hide_imputed: bool,
	shown: Vec<u8>,
}

/// What a draw put on screen, for the legend.
pub struct Drawn {
	/// The heat's five ticks over the cells in view, and how many cells that is.
	pub heat: Option<([f64; 5], usize)>,
	/// The pressure values the iso-lines run at. `None` when nothing in view is under pressure.
	pub levels: Option<[f64; 3]>,
}

impl Scene {
	/// `on_view(west, south, east, north)` fires whenever the map settles, and once on attach if it
	/// already has bounds. The marker hooks are the Leptos map's: a click on a dot always opens its
	/// info window, with or without them.
	pub fn attach(map: &JsValue, on_view: &Function, on_poi: Option<Function>, on_over: Option<Function>, on_out: Option<Function>) -> Self {
		Self {
			js: attach(map, on_view, on_poi, on_over, on_out),
			models: Vec::new(),
			c_lat: Vec::new(),
			c_lng: Vec::new(),
			imputed: Vec::new(),
			view: None,
			heat: None,
			press: None,
			hide_imputed: false,
			shown: Vec::new(),
		}
	}

	/// Every canvas, listener and marker this put on the map, taken off it.
	pub fn detach(&self) {
		detach(&self.js);
	}

	/// The studies painted, replacing whatever was. Heat and field are cleared with them: their arrays
	/// were per cell of the old set.
	pub fn show(&mut self, models: Vec<Rc<Model>>) {
		let cat = |f: fn(&Model) -> &[f64]| models.iter().flat_map(|m| f(m).iter().copied()).collect::<Vec<f64>>();
		geometry(&self.js, &cat(|m| &m.ring_x), &cat(|m| &m.ring_y));
		self.c_lat = cat(|m| &m.c_lat);
		self.c_lng = cat(|m| &m.c_lng);
		self.imputed = models.iter().flat_map(|m| m.payload.imputed.iter().copied()).collect();
		self.shown = vec![0; self.imputed.len()];
		self.models = models;
		self.heat = None;
		self.press = None;
	}

	/// One array per study shown, in `show`'s order. `linear` is magnitude rather than rank.
	pub fn heat(&mut self, heat: Option<(&[&[f64]], bool)>) {
		self.heat = heat.map(|(per, linear)| {
			assert_eq!(per.len(), self.models.len(), "one heat array per study shown");
			(per.concat(), linear)
		});
	}

	/// Competitor pressure, one array per study shown. `None` takes the field off.
	pub fn field(&mut self, press: Option<&[&[f64]]>) {
		self.press = press.map(|per| {
			assert_eq!(per.len(), self.models.len(), "one pressure array per study shown");
			per.iter().map(|p| p.to_vec()).collect()
		});
	}

	pub fn view(&mut self, west_south_east_north: [f64; 4]) {
		self.view = Some(west_south_east_north);
	}

	pub fn hide_imputed(&mut self, on: bool) {
		self.hide_imputed = on;
	}

	pub fn opacity(&self, v: f64) {
		opacity(&self.js, v);
	}

	/// Which cells the heat painted, for a tooltip to find the one under the cursor.
	pub fn shown(&self) -> &[u8] {
		&self.shown
	}

	/// The marker set, replacing whatever is there; see [`dots`].
	pub fn dots(&self, json: &str) {
		dots_js(&self.js, json);
	}

	/// One flag per tier. An empty array shows every dot.
	pub fn tiers(&self, flags: &[u8]) {
		tiers(&self.js, flags);
	}

	/// Colour against the cells in view and hand the result to the canvases.
	pub fn draw(&mut self) -> Option<Drawn> {
		let [w, s, e, n] = self.view?;
		let in_view: Vec<bool> = self.c_lat.iter().zip(&self.c_lng).map(|(&lat, &lng)| lat >= s && lat <= n && lng >= w && lng <= e).collect();

		let heat = match &self.heat {
			Some((values, linear)) => {
				let c = colorise(values, &self.imputed, self.hide_imputed, *linear, &in_view);
				cells(&self.js, &c.colors, &c.shown);
				self.shown = c.shown;
				Some((c.ticks, c.count))
			}
			None => {
				cells(&self.js, &[], &[]);
				self.shown.fill(0);
				None
			}
		};

		let levels = match &self.press {
			Some(per) => {
				let all = per.concat();
				match field(&all, &in_view) {
					Some(Field { alpha, levels }) => {
						let segs: Vec<f64> = self.models.iter().zip(per).flat_map(|(m, p)| m.contours(p, &levels)).collect();
						field_js(&self.js, &format!("{},{},{}", HAZE[0], HAZE[1], HAZE[2]), &alpha, &segs);
						Some(levels)
					}
					None => {
						field_js(&self.js, "", &[], &[]);
						None
					}
				}
			}
			None => {
				field_js(&self.js, "", &[], &[]);
				None
			}
		};
		Some(Drawn { heat, levels })
	}
}

/// Every competitor of every study, as `paint.js`'s markers want them, in `models` order — which is
/// the index a marker hook reports. Size says the tier; `color` says whatever the caller argues by.
pub fn dots(models: &[&Model], color: impl Fn(&Model, &PoiOut, usize) -> String) -> String {
	let out: Vec<serde_json::Value> = models
		.iter()
		.filter_map(|m| Some((*m, m.payload.trade.as_ref()?)))
		.flat_map(|(m, trade)| {
			let color = &color;
			trade.pois.iter().map(move |p| {
				let ti = trade.tiers.iter().position(|t| t.name == p.poi.tier).expect("the model rejected a payload whose tiers disagree");
				let kind = match (p.poi.kind_label.as_str(), p.poi.kind.as_str()) {
					("", "") => "no category",
					("", k) => k,
					(l, _) => l,
				};
				serde_json::json!({
					"lat": p.poi.lat, "lng": p.poi.lng, "name": p.poi.name, "addr": p.poi.addr,
					"kind": kind, "tier": p.poi.tier, "rating": p.poi.rating, "n_rev": p.poi.n_rev,
					"web": p.poi.web, "tel": p.poi.tel, "ti": ti, "big": ti == 0, "color": color(m, p, ti),
				})
			})
		})
		.collect();
	serde_json::to_string(&out).expect("a Vec<Value> serialises")
}
