//! The map island. Rust owns every number and the whole control surface. `map_core.js` owns the
//! `google.maps` instance and the pins; what is painted over it is
//! `gmaps_optimal_placement_overlay::Overlay`'s, the renderer the conductor's map paints through too,
//! and the legend is what it tells.
use std::rc::Rc;

use gmaps_optimal_placement_core::{
	Model,
	model::{Report, Summary, fmt},
};
use gmaps_optimal_placement_overlay::Legend;
pub use imp::{activate, close, open};
use leptos::prelude::*;

use crate::{
	pins::Pin,
	tabs::{Keymap, Pool},
};

/// One study, as the controls stood when its tab was last left. `activate` swaps this against the
/// live signals; the map instance underneath is never rebuilt.
#[derive(Clone)]
pub struct Tab {
	/// The two labels, which is what `/payload/{category}/{location}` is keyed by. No category is a
	/// location on its own, under `/place/{location}`.
	#[cfg(feature = "hydrate")]
	pub at: (Option<String>, String),
	/// The payload's own name.
	pub label: String,
	#[cfg(feature = "hydrate")]
	pub model: Rc<Model>,
	/// As served: what the overlay is shown.
	#[cfg(feature = "hydrate")]
	pub json: Rc<str>,
	/// A `Summary.layers[].id`.
	#[cfg(feature = "hydrate")]
	pub layer: Option<String>,
	#[cfg(feature = "hydrate")]
	pub hide_imputed: bool,
	#[cfg(feature = "hydrate")]
	pub tiers: Vec<String>,
	#[cfg(feature = "hydrate")]
	pub pins: Vec<Pin>,
	#[cfg(feature = "hydrate")]
	pub selected: Option<String>,
}

/// Everything the view reads and the effects write. Copy, so a closure captures it by value.
#[derive(Clone, Copy)]
pub struct State {
	pub heavy: StoredValue<Heavy, LocalStorage>,
	/// `Model` is an `Rc`, so the tabs cannot live in a `Send` signal.
	pub tabs: RwSignal<Vec<Tab>, LocalStorage>,
	pub active: RwSignal<usize>,
	/// The two axes served, whether or not any pairing of them has a tab.
	pub categories: RwSignal<Vec<String>>,
	pub locations: RwSignal<Vec<String>>,
	pub picker: RwSignal<Option<Pool>>,
	pub settings: RwSignal<bool>,
	pub keys: RwSignal<Keymap>,
	/// Which binding the settings panel is waiting on. A clicked button does not take focus in every
	/// browser, so the next keypress is read off the document rather than off the button.
	pub arming: RwSignal<Option<&'static str>>,
	/// Global: how you look at a map, not a property of a study.
	pub opacity: RwSignal<f64>,
	/// The competitor-pressure field over the heat. Global, like the opacity.
	pub field: RwSignal<bool>,
	/// The competitor dots. Global, like the field.
	pub dots: RwSignal<bool>,
	pub layer: RwSignal<Option<String>>,
	pub hide_imputed: RwSignal<bool>,
	/// The tiers whose marks are shown.
	pub tiers: RwSignal<Vec<String>>,
	/// Bumped by every study the overlay is shown, so what is pushed into it is pushed again even
	/// where the next tab's control reads the same.
	#[cfg(feature = "hydrate")]
	pub shown: RwSignal<u32>,
	pub summary: RwSignal<Option<Summary>>,
	pub legend: RwSignal<Option<Legend>>,
	pub report: RwSignal<Option<Report>>,
	/// Which pin's card is open. `None` while a sweep is shown.
	pub selected: RwSignal<Option<String>>,
	pub pins: RwSignal<Vec<Pin>>,
	pub tip: RwSignal<Option<Tip>>,
	pub banner: RwSignal<Option<String>>,
	/// Flips once `map_core` has a map instance and the overlay is on it.
	#[cfg(feature = "hydrate")]
	pub mounted: RwSignal<bool>,
	pub next_id: StoredValue<u32>,
	/// The `⧉` glyph, flipped for a beat by a copy that landed or one that did not.
	pub copied: RwSignal<Option<bool>>,
}
impl State {
	fn new() -> Self {
		Self {
			heavy: StoredValue::new_local(Heavy::default()),
			tabs: RwSignal::new_local(Vec::new()),
			active: RwSignal::new(0),
			categories: RwSignal::new(Vec::new()),
			locations: RwSignal::new(Vec::new()),
			picker: RwSignal::new(None),
			settings: RwSignal::new(false),
			keys: RwSignal::new(Keymap::default()),
			arming: RwSignal::new(None),
			opacity: RwSignal::new(0.62),
			field: RwSignal::new(false),
			dots: RwSignal::new(true),
			layer: RwSignal::new(None),
			hide_imputed: RwSignal::new(false),
			tiers: RwSignal::new(Vec::new()),
			#[cfg(feature = "hydrate")]
			shown: RwSignal::new(0),
			summary: RwSignal::new(None),
			legend: RwSignal::new(None),
			report: RwSignal::new(None),
			selected: RwSignal::new(None),
			pins: RwSignal::new(Vec::new()),
			tip: RwSignal::new(None),
			banner: RwSignal::new(None),
			#[cfg(feature = "hydrate")]
			mounted: RwSignal::new(false),
			next_id: StoredValue::new(0),
			copied: RwSignal::new(None),
		}
	}

	pub fn pin(&self, id: &str) -> Option<Pin> {
		self.pins.with_untracked(|v| v.iter().find(|p| p.id == id).cloned())
	}

	pub fn fresh_id(&self) -> String {
		let n = self.next_id.get_value();
		self.next_id.set_value(n + 1);
		format!("t{n}")
	}

	fn drop_pins(&self) {
		self.pins.set(Vec::new());
		self.report.set(None);
		self.selected.set(None);
	}

	fn rank(&self) {
		let picks = self.heavy.with_value(|h| h.model.as_ref()?.traded()?.rank_sites(10));
		let Some((picked, pool)) = picks else { return };
		self.pins.set(
			picked
				.iter()
				.enumerate()
				.map(|(k, r)| Pin {
					id: self.fresh_id(),
					at: [r.lat, r.lng],
					label: Some((k + 1).to_string()),
				})
				.collect(),
		);
		self.selected.set(None);
		self.heavy.with_value(|h| {
			let Some(m) = &h.model else { return };
			self.report.set(Some(Report {
				title: "Top 10 sites".to_owned(),
				rows: picked
					.iter()
					.enumerate()
					.map(|(k, r)| gmaps_optimal_placement_core::model::Row {
						label: format!("{}. {}", k + 1, m.payload.place[r.cell]),
						sub: None,
						value: fmt(r.score),
					})
					.collect(),
				note: format!("Greedy pick, kept ≥1.5 km apart, from {pool} cells tried. Click a numbered pin for its full report. These are cell centroids, not addresses."),
			}));
		});
	}

	#[cfg(feature = "hydrate")]
	/// Open a pin's card.
	pub fn open(&self, id: &str) {
		self.copied.set(None);
		self.selected.set(Some(id.to_owned()));
		self.refresh_card();
	}

	#[cfg(feature = "hydrate")]
	/// The selected pin's report. Reads nothing reactively, so the effect that opens the card cannot
	/// re-enter itself through `selected`.
	pub fn refresh_card(&self) {
		let Some(pin) = self.selected.get_untracked().and_then(|id| self.pin(&id)) else { return };
		self.heavy.with_value(|h| {
			let Some(m) = &h.model else { return };
			self.report.set(Some(match m.traded() {
				// a ranked pick is a cell centroid, and there is no business to score at one
				Some(t) => t.site_report(pin.at, pin.label.as_deref(), pin.label.is_none().then_some("")),
				None => m.cell_report(pin.at, pin.label.as_deref()),
			}));
		});
	}
}

/// The active study, which the reports and the tooltip read.
#[derive(Default)]
pub struct Heavy {
	pub model: Option<Rc<Model>>,
	/// Which cells the tooltip answers for: every one, less the imputed while they are hidden.
	#[cfg(feature = "hydrate")]
	pub shown: Vec<u8>,
}

#[derive(Clone, PartialEq)]
pub struct Tip {
	pub text: String,
	pub x: f64,
	pub y: f64,
}

#[island]
pub fn MapView() -> impl IntoView {
	let s = State::new();
	imp::wire(s);
	// no trade under the tab: the field, the dots and the sweep are its competitors' and its demand's
	let traded = move || s.summary.with(|l| l.as_ref().is_some_and(|l| l.trade.is_some()));
	let show_queries = RwSignal::new(false);
	let show_categories = RwSignal::new(true);
	let show_ages = RwSignal::new(false);

	view! {
		<crate::tabs::TabBar state=s />
		<div id="map"></div>
		{move || s.picker.get().map(|pool| view! { <crate::tabs::Picker state=s pool=pool /> })}
		{move || s.settings.get().then(|| view! { <crate::tabs::Settings state=s /> })}
		{move || {
			s.tip
				.get()
				.map(|t| {
					view! {
						<div id="tip" style=format!("display:block;left:{}px;top:{}px", t.x + 14., t.y + 14.)>
							{t.text}
						</div>
					}
				})
		}}
		{move || s.banner.get().map(|b| view! { <div id="banner">{b}</div> })}

		<div class="panel" id="ctl">
			<h4>"Layer"</h4>
			<select on:change=move |ev| {
				s.layer.set(Some(event_target_value(&ev)))
			}>
				{move || {
					s.summary
						.get()
						.map(|l| {
							l.layers
								.into_iter()
								.map(|row| {
									let id = row.id.clone();
									view! {
										<option
											value=row.id
											prop:selected=move || s.layer.with(|l| l.as_ref() == Some(&id))
										>
											{row.name}
										</option>
									}
								})
								.collect_view()
						})
				}}
			</select>

			<label>
				"Overlay opacity — " <span>{move || format!("{:.0}%", s.opacity.get() * 100.)}</span>
			</label>
			<input
				type="range"
				min="0"
				max="100"
				step="5"
				prop:value=move || s.opacity.get() * 100.
				on:input=move |ev| {
					if let Ok(v) = event_target_value(&ev).parse::<f64>() {
						s.opacity.set(v / 100.);
					}
				}
			/>

			{move || {
				Some(
					s
						.summary
						.get()?
						.trade?
						.tiers
						.into_iter()
						.enumerate()
						.map(|(i, t)| {
							let (label, shown, name) = (
								format!("{} ({})", t.name, t.fetched_s.len()),
								t.name.clone(),
								t.name,
							);
							view! {
								<div class="row">
									<input
										type="checkbox"
										id=format!("tier{i}")
										prop:checked=move || s.tiers.with(|v| v.contains(&shown))
										on:change=move |ev| {
											s.tiers
												.update(|v| match event_target_checked(&ev) {
													true => v.push(name.clone()),
													false => v.retain(|t| *t != name),
												});
										}
									/>
									<label for=format!("tier{i}")>{label}</label>
								</div>
							}
						})
						.collect_view(),
				)
			}}

			// what Google filed each one as, which is what the dots are coloured by. The tiers above
			// are the study's reading of these, and a category split across two of them is the study
			// being asked to justify itself.
			{move || {
				let (summary, tiers) = (s.summary.get(), s.tiers.get());
				let trade = summary?.trade?;
				let rows: Vec<_> = trade
					.categories
					.into_iter()
					.filter_map(|c| {
						let n: usize = c
							.counts
							.iter()
							.zip(&trade.tiers)
							.filter(|(_, t)| tiers.contains(&t.name))
							.map(|(n, _)| n)
							.sum();
						(n > 0)
							.then(|| {
								view! {
									<div class="cat">
										<span class="sw" style=format!("background:{}", c.color)></span>
										<label>{format!("{} ({n})", c.label)}</label>
									</div>
								}
							})
					})
					.collect();
				let open = show_categories.get();
				(!rows.is_empty())
					.then(|| {
						view! {
							<h4 class="fold" on:click=move |_| show_categories.update(|v| *v = !*v)>
								{if open { "▾ " } else { "▸ " }}
								"Google categories"
							</h4>
							{open.then_some(rows)}
						}
					})
			}}

			{move || {
				let ages = s.legend.get()?.ages?;
				let open = show_ages.get();
				Some(
					view! {
						<h4 class="fold" on:click=move |_| show_ages.update(|v| *v = !*v)>
							{if open { "▾ " } else { "▸ " }}
							{format!("Data age · oldest {:.0} d", ages.oldest_days)}
						</h4>
						{open.then(|| age_chart(&ages))}
					},
				)
			}}

			<div class="row">
				<input
					type="checkbox"
					id="hideImp"
					prop:checked=move || s.hide_imputed.get()
					on:change=move |ev| s.hide_imputed.set(event_target_checked(&ev))
				/>
				<label for="hideImp">
					"Hide imputed cells (" {move || s.summary.get().map(|l| l.imputed.to_string())} ")"
				</label>
			</div>
			<div class="row">
				<input
					type="checkbox"
					id="dots"
					prop:disabled=move || !traded()
					prop:checked=move || s.dots.get()
					on:change=move |ev| s.dots.set(event_target_checked(&ev))
				/>
				<label for="dots">
					"Competitors"
					{move || {
						s.legend.get().and_then(|l| l.competitors).map(|c| format!(" ({} in view)", c.in_view))
					}}
				</label>
			</div>
			<div class="row">
				<input
					type="checkbox"
					id="field"
					prop:disabled=move || !traded()
					prop:checked=move || s.field.get()
					on:change=move |ev| s.field.set(event_target_checked(&ev))
				/>
				<label for="field">"Competitor pressure field"</label>
			</div>

			<button
				class="sec"
				style:display=move || {
					if s
						.summary
						.with(|l| {
							l.as_ref().and_then(|l| l.trade.as_ref()).is_some_and(|t| !t.queries.is_empty())
						})
					{
						"block"
					} else {
						"none"
					}
				}
				on:click=move |_| show_queries.update(|v| *v = !*v)
			>
				"Search queries"
			</button>
			{move || {
				show_queries
					.get()
					.then(|| {
						s.summary
							.get()
							.and_then(|l| l.trade)
							.map(|t| {
								t.queries
									.into_iter()
									.map(|q| {
										view! {
											<div class="cat">
												<label>{format!("{} ({})", q.text, q.hits)}</label>
											</div>
										}
									})
									.collect_view()
							})
					})
			}}

			{move || {
				let notice = s
					.heavy
					.with_value(|h| {
						h.model.as_ref().map(|m| m.payload.notice.as_ref().map(|n| n.text.clone()))
					});
				let text = match (s.summary.get()?, notice.flatten()) {
					(_, Some(n)) => n,
					(l, None) if l.trade.is_none() => {
						"No category — this is the statistical grid alone, and it cost no Places call. Open the same city under a category for demand, the competitors on it, and the sweeps."
							.to_owned()
					}
					_ => return None,
				};
				Some(view! { <p class="note">{text}</p> })
			}}

			<button prop:disabled=move || !traded() on:click=move |_| s.rank()>
				"Rank top 10 sites"
			</button>
			<button class="sec" on:click=move |_| s.drop_pins()>
				"Clear pins & report"
			</button>

			{move || {
				let heat = s.legend.get()?.heat?;
				Some(
					view! {
						<div id="bar" style=gradient(&heat.stops)></div>
						<div id="ticks">
							{heat.ticks.into_iter().flatten().map(|t| view! { <span>{t}</span> }).collect_view()}
						</div>
						<div class="note">
							<b>{plain(&heat.name)}</b>
							{heat.focus.map(|f| view! { <span class="k">{format!(" — {f}")}</span> })}
							<br />
							{heat.note}
							<br />
							<span class="k">
								{format!(
									"{} cells in view · colour = {}",
									heat.cells,
									match heat.scale {
										gmaps_optimal_placement_core::Scale::Linear => "linear",
										gmaps_optimal_placement_core::Scale::Percentile => "percentile",
									},
								)}
							</span>
						</div>
					},
				)
			}}
			{move || {
				let p = s.legend.get()?.pressure?;
				let levels = p.levels?;
				Some(
					view! {
						<div id="haze" style=gradient(&p.stops)></div>
						<div class="note">
							{p.note} " Lines at the 50th, 75th, 90th: "
							{levels.into_iter().map(|l| l.value).collect::<Vec<_>>().join(" · ")}
						</div>
					},
				)
			}}
		</div>

		{move || s.report.get().map(|r| view! { <crate::pins::Card state=s report=r /> })}
	}
}

/// The `★` is a hint in the dropdown, not part of the layer's name.
fn plain(name: &str) -> String {
	name.replace(" ★", "").trim().to_owned()
}

fn gradient(stops: &[String]) -> String {
	assert!(stops.len() > 1, "a ramp has two ends");
	let at = |i: usize| 100. * i as f64 / (stops.len() - 1) as f64;
	let stops: Vec<String> = stops.iter().enumerate().map(|(i, c)| format!("{c} {:.0}%", at(i))).collect();
	format!("background:linear-gradient(90deg,{})", stops.join(","))
}

/// Competitors per age bucket, as the overlay laid the bars out.
fn age_chart(ages: &gmaps_optimal_placement_core::model::Ages) -> impl IntoView + use<> {
	let bars = ages
		.bars
		.iter()
		.map(|b| {
			view! {
				<div
					title=format!("{} competitors, {} old", b.count, b.span)
					style=format!(
						"left:{:.2}%;width:calc({:.2}% - 1px);height:{:.1}%;background:oklch(0.74 {:.3} 235)",
						b.left,
						b.width,
						b.height,
						0.16 * b.fresh,
					)
				>
					{b.count}
				</div>
			}
		})
		.collect_view();
	let ticks = ages
		.ticks
		.iter()
		.map(|t| view! { <span style=format!("left:{:.2}%", t.left)>{t.label.clone()}</span> })
		.collect_view();
	view! { <div class="ages">{bars}{ticks}</div> }
}

#[cfg(not(feature = "hydrate"))]
mod imp {
	use super::State;

	/// Server-side the island renders its empty shell; nothing runs until the wasm lands.
	pub fn wire(_: State) {}
	pub fn open(_: State, _: Option<String>, _: String) {}
	pub fn activate(_: State, _: usize) {}
	pub fn close(_: State, _: usize) {}
}

#[cfg(feature = "hydrate")]
mod imp {
	use std::rc::Rc;

	use gmaps_optimal_placement_core::model::Summary;
	use gmaps_optimal_placement_overlay::{Legend, Overlay};
	use leptos::prelude::*;
	use wasm_bindgen::{closure::WasmClosure, prelude::*};

	use super::{State, Tab, Tip};
	use crate::{pins::Pin, tabs::Pool};

	const YELLOW: &str = "#ffd400";

	thread_local! {
		/// Once the map is mounted. Not in `Heavy`: its `emit` runs inside every call made of it.
		static OVERLAY: std::cell::OnceCell<Overlay> = const { std::cell::OnceCell::new() };
	}

	/// Whatever is on the map, once it is.
	fn overlay(f: impl FnOnce(&Overlay)) {
		OVERLAY.with(|o| o.get().map(f));
	}

	/// What the overlay tells. What the shown payload is and why it is short are the payload's, which
	/// the island holds already.
	#[derive(serde::Deserialize)]
	#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
	enum Told {
		Data(serde::de::IgnoredAny),
		Legend(Legend),
		Notice(serde::de::IgnoredAny),
	}

	fn shown(s: State) {
		let hide = s.hide_imputed.get_untracked();
		s.heavy.update_value(|h| {
			h.shown = h
				.model
				.as_ref()
				.map_or_else(Vec::new, |m| m.payload.imputed.iter().map(|&i| u8::from(!(hide && i == 1))).collect())
		});
	}

	#[wasm_bindgen(module = "/src/map_core.js")]
	extern "C" {
		/// The map, or a banner string saying why there is none.
		#[wasm_bindgen(js_name = mount)]
		async fn mount_js(
			el: web_sys::HtmlElement,
			lat: f64,
			lng: f64,
			zoom: u8,
			on_click: &js_sys::Function,
			on_move: &js_sys::Function,
			on_out: &js_sys::Function,
			on_pin: &js_sys::Function,
		) -> JsValue;
		#[wasm_bindgen(js_name = recenter)]
		fn recenter_js(el: &web_sys::HtmlElement, lat: f64, lng: f64, zoom: u8);
		#[wasm_bindgen(js_name = pins)]
		fn pins_js(el: &web_sys::HtmlElement, json: &str);
	}

	fn host() -> Option<web_sys::HtmlElement> {
		document().get_element_by_id("map").and_then(|e| e.dyn_into().ok())
	}

	/// Leaked once: the map holds these for the page's whole life, and the island never unmounts.
	fn leak<T: ?Sized + WasmClosure>(c: Closure<T>) -> js_sys::Function {
		c.into_js_value().unchecked_into()
	}

	pub fn wire(s: State) {
		Effect::new(move |ran: Option<()>| {
			if ran.is_some() {
				return;
			}
			leptos::task::spawn_local(async move { boot(s).await });
		});

		let on_key = leak(Closure::<dyn Fn(web_sys::KeyboardEvent)>::new(move |ev: web_sys::KeyboardEvent| key(s, &ev)));
		document().add_event_listener_with_callback("keydown", &on_key).expect("the document takes a keydown listener");

		Effect::new(move |_| {
			let tiers = s.tiers.get();
			s.shown.track();
			if !s.heavy.with_value(|h| h.model.as_ref().is_some_and(|m| m.traded().is_some())) {
				return;
			}
			let tune = js_sys::JSON::parse(&serde_json::json!({ "tiers": tiers }).to_string()).expect("serde_json writes JSON");
			overlay(|o| {
				if let Err(e) = o.tune(tune) {
					s.banner.set(Some(format!("⚠ {}", String::from(JsValue::from(e).unchecked_into::<js_sys::Error>().message()))));
				}
			});
		});

		Effect::new(move |_| {
			let layer = s.layer.get();
			s.shown.track();
			overlay(|o| {
				if let Err(e) = o.heat(layer) {
					s.banner.set(Some(format!("⚠ {}", String::from(JsValue::from(e).unchecked_into::<js_sys::Error>().message()))));
				}
			});
		});

		Effect::new(move |_| {
			let on = s.hide_imputed.get();
			s.shown.track();
			shown(s);
			overlay(|o| o.hide_imputed(on));
		});

		Effect::new(move |_| {
			let (field, dots, opacity) = (s.field.get(), s.dots.get(), s.opacity.get());
			s.mounted.track();
			overlay(|o| {
				o.pressure(field);
				o.competitors(dots);
				o.opacity(opacity);
			});
		});

		Effect::new(move |_| {
			s.selected.track();
			s.refresh_card();
		});

		Effect::new(move |_| {
			let out: Vec<serde_json::Value> = s
				.pins
				.get()
				.iter()
				.map(|p| serde_json::json!({"id": p.id, "lat": p.at[0], "lng": p.at[1], "color": YELLOW, "label": p.label, "title": p.label}))
				.collect();
			if let Some(el) = host().filter(|_| s.mounted.get()) {
				pins_js(&el, &serde_json::to_string(&out).expect("a Vec<Value> serialises"));
			}
		});
	}

	/// The two axes the CLI was given.
	#[derive(serde::Deserialize)]
	struct Axes {
		categories: Vec<String>,
		locations: Vec<String>,
	}

	/// The axes and the saved keys; the first study is picked the way every later one is.
	async fn boot(s: State) {
		let served = match get::<Axes>("/studies.json").await {
			Ok(d) => d,
			Err(e) => return s.banner.set(Some(format!("⚠ what is served never arrived — {e}"))),
		};
		s.categories.set(served.categories);
		s.locations.set(served.locations);
		match crate::tabs::load_keys().await {
			Ok(Some(k)) => s.keys.set(k),
			Ok(None) => {}
			Err(e) => s.banner.set(Some(format!("⚠ keys — {e}"))),
		}
		s.picker.set(Some(Pool::Study));
	}

	/// Fetch a study, build its `Model`, and give it a tab. Mounts the map on the first one.
	async fn add(s: State, at: (Option<String>, String)) -> Option<usize> {
		let tab = fetch(s, at).await?;
		let model = tab.model.clone();
		s.tabs.update(|v| v.push(tab));
		if !s.mounted.get_untracked() && !mount(s, &model).await {
			return None;
		}
		Some(s.tabs.with_untracked(Vec::len) - 1)
	}

	/// A study, as a tab that is not open yet.
	async fn fetch(s: State, at: (Option<String>, String)) -> Option<Tab> {
		// a pairing the server has not built yet reads the grid archive and every POI page, which is
		// tens of seconds of nothing to look at
		let shown = match &at.0 {
			Some(t) => format!("{t} in {}", at.1),
			None => at.1.clone(),
		};
		s.banner.set(Some(format!("building {shown} …")));
		let location = js_sys::encode_uri_component(&at.1);
		let url = match &at.0 {
			Some(t) => format!("/payload/{}/{location}", js_sys::encode_uri_component(t)),
			None => format!("/place/{location}"),
		};
		let built = text(&url).await.and_then(|json| {
			let payload = serde_json::from_str(&json).map_err(|e| format!("it did not parse — {e}"))?;
			let model = gmaps_optimal_placement_core::Model::try_new(payload).map_err(|e| format!("{e:#}"))?;
			Ok((Rc::<str>::from(json), Rc::new(model)))
		});
		let (json, model) = match built {
			Ok(b) => b,
			Err(e) => {
				s.banner.set(Some(format!("⚠ {shown} — {e}")));
				return None;
			}
		};
		s.banner.set(None);
		let summary = Summary::of(&model);
		Some(Tab {
			at,
			label: model.payload.name.clone(),
			layer: summary.layers.first().map(|l| l.id.clone()),
			hide_imputed: false,
			tiers: model.traded().map_or_else(Vec::new, |t| t.selection().tiers),
			pins: Vec::new(),
			selected: None,
			json,
			model,
		})
	}

	/// The live signals, back into the tab they belong to.
	fn stash(s: State) {
		let i = s.active.get_untracked();
		s.tabs.update(|v| {
			let Some(t) = v.get_mut(i) else { return };
			t.layer = s.layer.get_untracked();
			t.hide_imputed = s.hide_imputed.get_untracked();
			t.tiers = s.tiers.get_untracked();
			t.pins = s.pins.get_untracked();
			t.selected = s.selected.get_untracked();
		});
	}

	/// A tab, into the live signals and onto the overlay.
	fn adopt(s: State, i: usize) {
		let t = s.tabs.with_untracked(|v| v[i].clone());
		s.active.set(i);
		s.heavy.update_value(|h| h.model = Some(t.model.clone()));
		overlay(|o| o.show(Some(t.json.to_string()), false));
		s.layer.set(t.layer);
		s.hide_imputed.set(t.hide_imputed);
		s.tiers.set(t.tiers);
		s.pins.set(t.pins);
		s.report.set(None);
		s.selected.set(t.selected);
		s.summary.set(Some(Summary::of(&t.model)));
		s.shown.update(|n| *n += 1);
		// a tab carries its own, so switching away from a refusal does not leave it over the next map
		s.banner.set(t.model.payload.notice.as_ref().map(|n| format!("⚠ {}", n.text)));
		document().set_title(&t.model.payload.name);
		if let Some(el) = host().filter(|_| s.mounted.get_untracked()) {
			recenter_js(&el, t.model.payload.center[0], t.model.payload.center[1], t.model.payload.zoom);
		}
	}

	pub fn activate(s: State, i: usize) {
		if i == s.active.get_untracked() {
			return;
		}
		stash(s);
		adopt(s, i);
	}

	/// An already-open study is switched to rather than fetched twice.
	///
	/// Not `leptos::task::spawn_local`: the picker closes itself on the way in, and a task owned by a
	/// component that is being disposed is dropped rather than run.
	pub fn open(s: State, trade: Option<String>, location: String) {
		wasm_bindgen_futures::spawn_local(async move {
			let at = (trade, location);
			if let Some(i) = s.tabs.with_untracked(|v| v.iter().position(|t| t.at == at)) {
				return activate(s, i);
			}
			stash(s);
			if let Some(i) = add(s, at).await {
				adopt(s, i);
			}
		});
	}

	pub fn close(s: State, i: usize) {
		let active = s.active.get_untracked();
		// the tab being closed is discarded, so only a different one is worth writing back
		if i != active {
			stash(s);
		}
		s.tabs.update(|v| {
			v.remove(i);
		});
		let left = s.tabs.with_untracked(Vec::len);
		if left == 0 {
			return blank(s);
		}
		let next = if i < active { active - 1 } else { active.min(left - 1) };
		adopt(s, next);
	}

	/// No tabs: the panel, the report and the map all say so.
	fn blank(s: State) {
		s.active.set(0);
		s.heavy.update_value(|h| *h = super::Heavy::default());
		overlay(|o| o.show(None, false));
		s.summary.set(None);
		s.legend.set(None);
		s.report.set(None);
		s.selected.set(None);
		s.pins.set(Vec::new());
		s.tiers.set(Vec::new());
		s.layer.set(None);
		document().set_title("studies");
	}

	/// A pin and its card.
	fn click(s: State, at: [f64; 2]) {
		let id = s.fresh_id();
		s.pins.update(|v| v.push(Pin { id: id.clone(), at, label: None }));
		s.open(&id);
	}

	fn persist_keys(s: State) {
		let keys = s.keys.get_untracked();
		leptos::task::spawn_local(async move {
			if let Err(e) = crate::tabs::save_keys(keys).await {
				s.banner.set(Some(format!("⚠ the key file was not written — {e}")));
			}
		});
	}

	/// Digits jump by position; everything else is looked up in the keymap. A modifier or a text
	/// input means the key was not meant for the map.
	fn key(s: State, ev: &web_sys::KeyboardEvent) {
		let k = ev.key();
		if ev.ctrl_key() || ev.meta_key() || ev.alt_key() {
			return;
		}
		if k == "Escape" {
			s.arming.set(None);
			s.picker.set(None);
			s.settings.set(false);
			return;
		}
		if s.settings.get_untracked() {
			// a rebind takes one character; anything else leaves the row armed
			if let Some(label) = s.arming.get_untracked().filter(|_| k.chars().count() == 1) {
				ev.prevent_default();
				s.arming.set(None);
				s.keys.update(|m| m.set(label, k));
				persist_keys(s);
			}
			return;
		}
		if s.picker.get_untracked().is_some() {
			return;
		}
		if ev
			.target()
			.and_then(|t| t.dyn_into::<web_sys::Element>().ok())
			.is_some_and(|t| matches!(t.tag_name().as_str(), "INPUT" | "SELECT" | "TEXTAREA"))
		{
			return;
		}
		if k == "Backspace" {
			if let Some(id) = s.selected.get_untracked() {
				ev.prevent_default();
				crate::pins::forget(s, id);
			}
			return;
		}
		let n = s.tabs.with_untracked(Vec::len);
		if let Ok(d) = k.parse::<usize>() {
			let i = if d == 0 { n.checked_sub(1) } else { (d <= n).then(|| d - 1) };
			if let Some(i) = i {
				ev.prevent_default();
				activate(s, i);
			}
			return;
		}
		let m = s.keys.get_untracked();
		let at = s.active.get_untracked();
		let ours = match k {
			_ if k == m.open => {
				s.picker.set(Some(Pool::Study));
				true
			}
			_ if k == m.find => {
				s.picker.set(Some(Pool::Open));
				true
			}
			_ if n == 0 => false,
			_ if k == m.prev => {
				activate(s, (at + n - 1) % n);
				true
			}
			_ if k == m.next => {
				activate(s, (at + 1) % n);
				true
			}
			_ if k == m.close => {
				close(s, at);
				true
			}
			_ => false,
		};
		// a field focused by a keydown receives that key's character, so without this the key that
		// opened it is the field's first character
		if ours {
			ev.prevent_default();
		}
	}

	async fn get<T: serde::de::DeserializeOwned>(url: &str) -> Result<T, String> {
		serde_json::from_str(&text(url).await?).map_err(|e| format!("it did not parse — {e}"))
	}

	async fn text(url: &str) -> Result<String, String> {
		match gloo_net::http::Request::get(url).send().await {
			Err(e) => Err(e.to_string()),
			Ok(r) if !r.ok() => Err(r.text().await.unwrap_or_else(|_| r.status_text())),
			Ok(r) => r.text().await.map_err(|e| format!("it never arrived whole — {e}")),
		}
	}

	/// The one `google.maps` instance and the overlay on it, on the first study to arrive. Their
	/// callbacks read the live signals, so they outlive every tab switch.
	async fn mount(s: State, model: &gmaps_optimal_placement_core::Model) -> bool {
		let Some(el) = host() else {
			s.banner.set(Some("⚠ the map host element is missing".to_owned()));
			return false;
		};
		let on_click = leak(Closure::<dyn Fn(f64, f64)>::new(move |lat: f64, lng: f64| click(s, [lat, lng])));
		let on_move = leak(Closure::<dyn Fn(f64, f64, f64, f64)>::new(move |lat: f64, lng: f64, x: f64, y: f64| {
			let text = s.heavy.with_value(|h| {
				let m = h.model.as_ref()?;
				let i = m.cell_at([lat, lng], &h.shown)?;
				Some(m.tooltip(i))
			});
			s.tip.set(text.map(|text| Tip { text, x, y }));
		}));
		let on_out = leak(Closure::<dyn Fn()>::new(move || s.tip.set(None)));
		let on_pin = leak(Closure::<dyn Fn(String)>::new(move |id: String| s.open(&id)));
		let emit = leak(Closure::<dyn Fn(JsValue)>::new(move |e: JsValue| {
			let e = String::from(js_sys::JSON::stringify(&e).expect("the overlay tells plain data"));
			match serde_json::from_str::<Told>(&e).expect("the overlay tells its own Event") {
				Told::Legend(l) => s.legend.set(Some(l)),
				Told::Data(_) | Told::Notice(_) => {}
			}
		}));

		let (c, zoom) = (model.payload.center, model.payload.zoom);
		let map = mount_js(el, c[0], c[1], zoom, &on_click, &on_move, &on_out, &on_pin).await;
		if let Some(msg) = map.as_string() {
			s.banner.set(Some(msg));
			return false;
		}
		OVERLAY.with(|o| assert!(o.set(Overlay::new(map, emit)).is_ok(), "mounted once"));
		s.mounted.set(true);
		true
	}
}
