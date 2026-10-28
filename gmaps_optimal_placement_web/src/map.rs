//! The map island. Rust owns every number and the whole control surface; `map_core.js` owns
//! `google.maps` and the canvas loop, because that one needs the live projection every frame.
use std::rc::Rc;

use gmaps_optimal_placement_core::{
	Model,
	model::{Report, TierState, fmt, ramp},
};
pub use imp::{activate, close, find, open, persist_keys, scroll_pick};
use leptos::prelude::*;

use crate::{
	pins::Pin,
	tabs::{Keymap, Pool},
};

/// A Google category, as Google files it — not as the study's `tier` rules read it. The tiers are
/// the study's argument about what competes; this is the evidence they were argued from, and the
/// two disagreeing is worth being able to see.
#[derive(Clone, PartialEq)]
pub struct Category {
	/// Places `primaryType`, which the colour is keyed by.
	pub kind: String,
	/// The Business Profile category Google displays. A place it typed as nothing is still on the
	/// map and still says so.
	pub label: String,
	pub color: String,
	/// Per tier, in the payload's order, so the panel can count only what is on screen.
	pub counts: Vec<usize>,
}

/// One study, as the controls stood when its tab was last left. `activate` swaps this against the
/// live signals; the map instance underneath is never rebuilt.
#[derive(Clone)]
pub struct Tab {
	/// The two file stems, which is what `/payload/{trade}/{location}` is keyed by. No trade is a
	/// location on its own, under `/place/{location}`. What the pairing is *called* is the payload's
	/// to say.
	pub at: (Option<String>, String),
	/// The payload's own name once it has landed, the pairing until then.
	pub label: String,
	pub model: Option<Rc<Model>>,
	pub lambda: f64,
	pub layer: usize,
	pub focus: usize,
	pub hide_imputed: bool,
	pub tiers: Vec<TierState>,
	pub core: Vec<Pin>,
	pub temp: Vec<Pin>,
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
	pub trades: RwSignal<Vec<String>>,
	pub locations: RwSignal<Vec<String>>,
	pub picker: RwSignal<Option<Pool>>,
	pub settings: RwSignal<bool>,
	pub keys: RwSignal<Keymap>,
	/// Which binding the settings panel is waiting on. A clicked button does not take focus in every
	/// browser, so the next keypress is read off the document rather than off the button.
	pub arming: RwSignal<Option<&'static str>>,
	pub lambda: RwSignal<f64>,
	/// Global: how you look at a map, not a property of a study.
	pub opacity: RwSignal<f64>,
	pub layer: RwSignal<usize>,
	/// Which competitor the coverage layers are drawn about. Moved by clicking a marker; the
	/// inventory is sorted by review count, so the opening one is the loudest name on the map.
	pub focus: RwSignal<usize>,
	pub hide_imputed: RwSignal<bool>,
	pub tiers: RwSignal<Vec<TierState>>,
	/// Bumped by every recompute, so the cheap effects can depend on it without depending on λ.
	pub recomputed: RwSignal<u32>,
	/// Name, note and tier counts, settled once the payload lands.
	pub loaded: RwSignal<Option<Loaded>>,
	pub legend: RwSignal<Option<Legend>>,
	pub report: RwSignal<Option<Report>>,
	/// Which pin's card is open. `None` while a sweep or a comparison is shown.
	pub selected: RwSignal<Option<String>>,
	pub core: RwSignal<Vec<Pin>>,
	pub temp: RwSignal<Vec<Pin>>,
	pub tip: RwSignal<Option<Tip>>,
	pub banner: RwSignal<Option<String>>,
	/// Flips once `map_core` has a map instance: the effects that push arrays into it are no-ops
	/// before that, and this is what gives them their second run.
	pub mounted: RwSignal<bool>,
	pub next_id: StoredValue<u32>,
	/// The `⧉` glyph, flipped for a beat by a copy that landed or one that did not.
	pub copied: RwSignal<Option<bool>>,
	/// Why the last place search put no pin down. Under the field rather than in the banner, so the
	/// next keystroke takes it away.
	pub missed: RwSignal<Option<String>>,
}
impl State {
	fn new() -> Self {
		Self {
			heavy: StoredValue::new_local(Heavy::default()),
			tabs: RwSignal::new_local(Vec::new()),
			active: RwSignal::new(0),
			trades: RwSignal::new(Vec::new()),
			locations: RwSignal::new(Vec::new()),
			picker: RwSignal::new(None),
			settings: RwSignal::new(false),
			keys: RwSignal::new(Keymap::default()),
			arming: RwSignal::new(None),
			lambda: RwSignal::new(2000.),
			opacity: RwSignal::new(0.62),
			layer: RwSignal::new(0),
			focus: RwSignal::new(0),
			hide_imputed: RwSignal::new(false),
			tiers: RwSignal::new(Vec::new()),
			recomputed: RwSignal::new(0),
			loaded: RwSignal::new(None),
			legend: RwSignal::new(None),
			report: RwSignal::new(None),
			selected: RwSignal::new(None),
			core: RwSignal::new(Vec::new()),
			temp: RwSignal::new(Vec::new()),
			tip: RwSignal::new(None),
			banner: RwSignal::new(None),
			mounted: RwSignal::new(false),
			next_id: StoredValue::new(0),
			copied: RwSignal::new(None),
			missed: RwSignal::new(None),
		}
	}

	/// The green set: what the study seeded, minus what was hidden, plus what was promoted. Lettered
	/// in that order.
	pub fn lettered(&self) -> Vec<(String, Pin)> {
		self.core.get().into_iter().enumerate().map(|(i, p)| (letter(i), p)).collect()
	}

	pub fn pin(&self, id: &str) -> Option<Pin> {
		self.core
			.with_untracked(|v| v.iter().find(|p| p.id == id).cloned())
			.or_else(|| self.temp.with_untracked(|v| v.iter().find(|p| p.id == id).cloned()))
	}

	pub fn fresh_id(&self) -> String {
		let n = self.next_id.get_value();
		self.next_id.set_value(n + 1);
		format!("t{n}")
	}

	/// Open a pin's card.
	pub fn open(&self, id: &str) {
		self.copied.set(None);
		self.selected.set(Some(id.to_owned()));
		self.refresh_card();
	}

	/// The selected pin's report, against the controls as they stand. Reads nothing reactively, so
	/// the effect that keeps the card live cannot re-enter itself through `selected`.
	pub fn refresh_card(&self) {
		let Some(pin) = self.selected.get_untracked().and_then(|id| self.pin(&id)) else { return };
		let label = self
			.core
			.get_untracked()
			.iter()
			.position(|p| p.id == pin.id)
			.map(|i| format!("{} · {}", letter(i), pin.name))
			.or_else(|| pin.label.clone())
			.or_else(|| pin.address.clone());
		self.heavy.with_value(|h| {
			let Some(m) = &h.model else { return };
			self.report.set(Some(match m.traded() {
				Some(t) => t.site_report(
					pin.at,
					label.as_deref(),
					// a ranked pick is a cell centroid with a number for a label, and there is no trading
					// name to score
					pin.label.is_none().then_some(pin.name.as_str()),
					self.lambda.get_untracked(),
					&h.press,
					&self.tiers.get_untracked(),
				),
				None => m.cell_report(pin.at, label.as_deref()),
			}));
		});
	}

	fn drop_temp(&self) {
		self.temp.set(Vec::new());
		self.report.set(None);
		self.selected.set(None);
	}

	fn rank(&self) {
		let picks = self.heavy.with_value(|h| Some(h.model.as_ref()?.traded()?.rank_sites(self.lambda.get_untracked(), &h.press, 10)));
		let Some((picked, pool)) = picks else { return };
		self.temp.set(
			picked
				.iter()
				.enumerate()
				.map(|(k, r)| Pin {
					id: self.fresh_id(),
					name: String::new(),
					at: [r.lat, r.lng],
					label: Some((k + 1).to_string()),
					address: None,
					green: false,
					from_study: false,
				})
				.collect(),
		);
		self.selected.set(None);
		self.heavy.with_value(|h| {
			let Some(m) = &h.model else { return };
			self.report.set(Some(Report {
				title: format!("Top 10 sites · λ={:.1} km", self.lambda.get_untracked() / 1000.),
				rows: picked
					.iter()
					.enumerate()
					.map(|(k, r)| gmaps_optimal_placement_core::model::Row {
						label: format!("{}. {}", k + 1, m.payload.place[r.cell]),
						sub: None,
						value: fmt(r.score),
					})
					.collect(),
				note: format!("Greedy pick, kept ≥1.5 km apart, from {pool} candidate cells. Click a numbered pin for its full report. These are cell centroids, not addresses."),
			}));
		});
	}

	fn compare(&self) {
		let cands: Vec<gmaps_optimal_placement_core::Candidate> = self
			.core
			.get_untracked()
			.into_iter()
			.map(|p| gmaps_optimal_placement_core::Candidate { name: p.name, at: p.at })
			.collect();
		if cands.is_empty() {
			return;
		}
		self.selected.set(None);
		self.heavy.with_value(|h| {
			let Some(t) = h.model.as_ref().and_then(|m| m.traded()) else { return };
			self.report.set(Some(t.compare(&cands, self.lambda.get_untracked(), &h.press)));
		});
	}
}

#[derive(Default)]
pub struct Heavy {
	pub model: Option<Rc<Model>>,
	pub press: Vec<f64>,
	pub unmet: Vec<f64>,
	pub shown: Vec<u8>,
}

/// What the control panel shows for the active study. Rebuilt on every tab switch.
#[derive(Clone, PartialEq)]
pub struct Loaded {
	pub layers: Vec<(String, String)>,
	pub tier_counts: Vec<usize>,
	pub categories: Vec<Category>,
	pub imputed: usize,
	/// Days since each competitor was last answered for, and its tier — so the chart follows the tier toggles.
	pub ages: Vec<(f64, usize)>,
	pub queries: Vec<(String, usize)>,
	/// Whether this tab has a trade under it. Everything a slider moves is the trade's, so this is
	/// what the panel greys out on.
	pub traded: bool,
	/// Why a trade that was asked for is not under it — the payload's own words.
	pub notice: Option<String>,
}

#[derive(Clone, PartialEq)]
pub struct Legend {
	pub ticks: [f64; 5],
	pub count: usize,
	pub linear: bool,
	/// Set on the coverage layers: whose cloud is on screen.
	pub focus: Option<String>,
}

#[derive(Clone, PartialEq)]
pub struct Tip {
	pub text: String,
	pub x: f64,
	pub y: f64,
}

/// Upper edges in days, each a period a person names. The last bar holds everything older.
const AGE_EDGES: [(f64, &str); 11] = [
	(1., "1d"),
	(3., "3d"),
	(7., "1w"),
	(14., "2w"),
	(30., "1mo"),
	(61., "2mo"),
	(91., "3mo"),
	(182., "6mo"),
	(365., "1y"),
	(730., "2y"),
	(1460., "4y"),
];

/// Competitors per age bucket over a logged axis, so a month does not dwarf the day before it. A
/// bar is as wide as the period it covers, and greys as it gets older.
fn age_chart(ages: &[f64]) -> impl IntoView + use<> {
	let bucket = |d: f64| AGE_EDGES.iter().position(|(e, _)| d < *e).unwrap_or(AGE_EDGES.len() - 1);
	let mut counts = vec![0usize; bucket(ages.iter().copied().fold(0., f64::max)) + 1];
	for &d in ages {
		counts[bucket(d)] += 1;
	}
	let top = AGE_EDGES[counts.len() - 1].0;
	let x = move |d: f64| 100. * d.ln_1p() / top.ln_1p();
	let max = *counts.iter().max().expect("at least one bucket") as f64;
	let bars = counts
		.iter()
		.enumerate()
		.filter(|(_, n)| **n > 0)
		.map(|(i, &n)| {
			let (lo, hi) = (if i == 0 { 0. } else { AGE_EDGES[i - 1].0 }, AGE_EDGES[i].0);
			let chroma = 0.16 * (1. - x((lo + hi) / 2.) / 100.);
			let span = match i {
				0 => "under a day".to_owned(),
				_ if i == AGE_EDGES.len() - 1 => format!("over {}", AGE_EDGES[i - 1].1),
				_ => format!("{}–{}", AGE_EDGES[i - 1].1, AGE_EDGES[i].1),
			};
			view! {
				<div
					title=format!("{n} competitors, {span} old")
					style=format!(
						"left:{:.2}%;width:calc({:.2}% - 1px);height:{:.1}%;background:oklch(0.74 {chroma:.3} 235)",
						x(lo),
						x(hi) - x(lo),
						100. * n as f64 / max,
					)
				>
					{n}
				</div>
			}
		})
		.collect_view();
	let ticks = AGE_EDGES[..counts.len()]
		.iter()
		.enumerate()
		.filter(|(i, _)| [0, 2, 4, 8].contains(i) || *i == counts.len() - 1)
		.map(|(_, (e, label))| view! { <span style=format!("left:{:.2}%", x(*e))>{*label}</span> })
		.collect_view();
	view! { <div class="ages">{bars}{ticks}</div> }
}

#[island]
pub fn MapView() -> impl IntoView {
	let s = State::new();
	imp::wire(s);
	// no trade under the tab: λ, the tier weights and both sweeps are functions of a demand surface
	// and an inventory, and neither is on the page
	let traded = move || s.loaded.get().is_some_and(|l| l.traded);
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
				s.layer.set(event_target_value(&ev).parse().unwrap_or(0))
			}>
				{move || {
					s.loaded
						.get()
						.map(|l| {
							l.layers
								.into_iter()
								.enumerate()
								.map(|(i, (name, _))| {
									view! {
										<option value=i.to_string() prop:selected=move || s.layer.get() == i>
											{name}
										</option>
									}
								})
								.collect_view()
						})
				}}
			</select>

			<label>
				"Catchment radius λ — "
				<span>{move || format!("{:.1} km", s.lambda.get() / 1000.)}</span>
			</label>
			<input
				type="range"
				min="300"
				max="6000"
				step="100"
				prop:disabled=move || !traded()
				prop:value=move || s.lambda.get()
				on:input=move |ev| {
					if let Ok(v) = event_target_value(&ev).parse() {
						s.lambda.set(v);
					}
				}
			/>

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
				s.loaded
					.get()
					.map(|l| {
						s.tiers
							.get_untracked()
							.into_iter()
							.enumerate()
							.map(|(i, t)| {
								let (name, count) = (t.name.clone(), l.tier_counts[i]);
								view! {
									<div class="row">
										<input
											type="checkbox"
											id=format!("tier{i}")
											prop:checked=move || s.tiers.with(|v| v.get(i).is_some_and(|t| t.show))
											on:change=move |ev| {
												s.tiers.update(|v| v[i].show = event_target_checked(&ev));
											}
										/>
										<label for=format!("tier{i}")>{format!("{name} ({count})")}</label>
									</div>
									<label>
										{format!("weight of one {} — ", t.name)}
										<span>
											{move || {
												s.tiers
													.with(|v| {
														v.get(i).map_or_else(String::new, |t| format!("{:.2}", t.weight))
													})
											}}
										</span>
									</label>
									<input
										type="range"
										min="0"
										max="200"
										step="5"
										value=(t.weight * 100.).round().to_string()
										on:input=move |ev| {
											if let Ok(v) = event_target_value(&ev).parse::<f64>() {
												s.tiers.update(|t| t[i].weight = v / 100.);
											}
										}
									/>
								}
							})
							.collect_view()
					})
			}}

			// what Google filed each one as, which is what the markers are coloured by. The tiers above
			// are the study's reading of these, and a category split across two of them is the study
			// being asked to justify itself.
			{move || {
				let (loaded, tiers) = (s.loaded.get(), s.tiers.get());
				let rows: Vec<_> = loaded?
					.categories
					.into_iter()
					.filter_map(|c| {
						let n: usize = c
							.counts
							.iter()
							.zip(&tiers)
							.filter(|(_, t)| t.show)
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
				let (loaded, tiers) = (s.loaded.get()?, s.tiers.get());
				let ages: Vec<f64> = loaded
					.ages
					.into_iter()
					.filter(|(_, t)| tiers.get(*t).is_some_and(|t| t.show))
					.map(|(d, _)| d)
					.collect();
				let oldest = ages.iter().copied().reduce(f64::max)?;
				let open = show_ages.get();
				Some(
					view! {
						<h4 class="fold" on:click=move |_| show_ages.update(|v| *v = !*v)>
							{if open { "▾ " } else { "▸ " }}
							{format!("Data age · oldest {oldest:.0} d")}
						</h4>
						{open
							.then(move || {
								view! {
									{age_chart(&ages)}
									<button
										class="sec"
										title="re-asks Google for every cell — spends a lot of Maps API quota"
										on:click=move |_| imp::reload(s)
									>
										"⚠ Pull latest"
									</button>
								}
							})}
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
					"Hide imputed cells (" {move || s.loaded.get().map(|l| l.imputed.to_string())} ")"
				</label>
			</div>

			<button
				class="sec"
				style:display=move || {
					if s.loaded.get().is_some_and(|l| !l.queries.is_empty()) { "block" } else { "none" }
				}
				on:click=move |_| show_queries.update(|v| *v = !*v)
			>
				"Search queries"
			</button>
			{move || {
				show_queries
					.get()
					.then(|| {
						s.loaded
							.get()
							.map(|l| {
								l.queries
									.into_iter()
									.map(|(text, hits)| {
										view! {
											<div class="cat">
												<label>{format!("{text} ({hits})")}</label>
											</div>
										}
									})
									.collect_view()
							})
					})
			}}

			{move || {
				let l = s.loaded.get()?;
				let text = match (l.notice, l.traded) {
					(Some(n), _) => n,
					(None, false) => {
						"No trade — this is the statistical grid alone, and it cost no Places call. Open the same city under a trade for demand, the competitors on it, and the sweeps."
							.to_owned()
					}
					(None, true) => return None,
				};
				Some(view! { <p class="note">{text}</p> })
			}}

			<button prop:disabled=move || !traded() on:click=move |_| s.rank()>
				"Rank top 10 sites"
			</button>
			<button
				style:display=move || if s.core.get().is_empty() { "none" } else { "block" }
				prop:disabled=move || !traded()
				on:click=move |_| s.compare()
			>
				"Compare candidates"
			</button>
			<button class="sec" on:click=move |_| s.drop_temp()>
				"Clear pins & report"
			</button>

			<div id="bar" style=gradient()></div>
			<div id="ticks">
				{move || {
					s.legend.get().map(|l| l.ticks.map(|t| view! { <span>{fmt(t)}</span> }).collect_view())
				}}
			</div>
			<div class="note">
				{move || {
					let (spec, legend) = (s.loaded.get(), s.legend.get());
					let (Some(spec), Some(legend)) = (spec, legend) else { return None };
					let (name, note) = spec.layers.get(s.layer.get()).cloned()?;
					Some(
						view! {
							<b>{plain(&name)}</b>
							{legend.focus.map(|f| view! { <span class="k">{format!(" — {f}")}</span> })}
							<br />
							{note}
							<br />
							<span class="k">
								{format!(
									"{} cells · colour = {}",
									legend.count,
									if legend.linear { "linear" } else { "percentile" },
								)}
							</span>
						},
					)
				}}
			</div>
		</div>

		{move || s.report.get().map(|r| view! { <crate::pins::Card state=s report=r /> })}
	}
}
fn letter(i: usize) -> String {
	char::from_u32('A' as u32 + i as u32).map_or_else(|| (i + 1).to_string(), String::from)
}

/// The `★` is a hint in the dropdown, not part of the layer's name.
fn plain(name: &str) -> String {
	name.replace(" ★", "").trim().to_owned()
}

fn gradient() -> String {
	let stops: Vec<String> = (0..=10)
		.map(|q| {
			let c = ramp(q as f64 / 10.);
			format!("rgb({},{},{}) {}%", c[0].round(), c[1].round(), c[2].round(), q * 10)
		})
		.collect();
	format!("background:linear-gradient(90deg,{})", stops.join(","))
}

#[cfg(not(feature = "hydrate"))]
mod imp {
	use super::State;

	/// Server-side the island renders its empty shell; nothing recomputes until the wasm lands.
	pub fn wire(_: State) {}
	pub fn open(_: State, _: Option<String>, _: String) {}
	pub fn activate(_: State, _: usize) {}
	pub fn close(_: State, _: usize) {}
	pub fn persist_keys(_: State) {}
	pub fn scroll_pick(_: usize) {}
	pub fn find(_: State, _: String) {}
	pub fn reload(_: State) {}
}

#[cfg(feature = "hydrate")]
mod imp {
	use gmaps_optimal_placement_core::{
		model::{LayerRef, LayerSpec, TradeLayer, colorise},
		payload::Trade,
	};
	use leptos::prelude::*;
	use wasm_bindgen::{closure::WasmClosure, prelude::*};

	use super::{Category, Legend, State, Tab, Tip};
	use crate::{pins::Pin, tabs::Pool};

	const GREEN: &str = "#7cff8f";
	const YELLOW: &str = "#ffd400";

	/// Every category on the map, commonest first, each with the colour it is painted in.
	/// Golden-angle hues: neighbours in the list land far apart on the wheel however many a city
	/// turns out to have.
	fn categories(trade: &Trade) -> Vec<Category> {
		let mut out: Vec<Category> = Vec::new();
		for p in &trade.pois {
			let ti = trade.tiers.iter().position(|t| t.name == p.poi.tier).expect("the model rejected a payload whose tiers disagree");
			let i = out.iter().position(|c| c.kind == p.poi.kind).unwrap_or_else(|| {
				out.push(Category {
					kind: p.poi.kind.clone(),
					label: match (p.poi.kind_label.as_str(), p.poi.kind.as_str()) {
						("", "") => "no category".to_owned(),
						("", k) => k.to_owned(),
						(l, _) => l.to_owned(),
					},
					color: String::new(),
					counts: vec![0; trade.tiers.len()],
				});
				out.len() - 1
			});
			out[i].counts[ti] += 1;
		}
		out.sort_by_key(|c| std::cmp::Reverse(c.counts.iter().sum::<usize>()));
		for (i, c) in out.iter_mut().enumerate() {
			c.color = format!("hsl({:.0},72%,58%)", (i as f64 * 137.508) % 360.);
		}
		out
	}

	/// What a redraw needs, once the controls have moved. Split from the payload so the cheap effects
	/// never pay for a pressure sweep.
	fn recompute(s: State) {
		s.heavy.update_value(|h| {
			// a tab with no trade leaves nothing of the last one's sweep behind: the arrays are the
			// active study's or they are empty
			let Some(m) = h.model.clone() else { return };
			let Some(t) = m.traded() else {
				(h.press, h.unmet) = (Vec::new(), Vec::new());
				return;
			};
			h.press = t.pressure(s.lambda.get_untracked(), &s.tiers.get_untracked());
			h.unmet = t.unmet(&h.press);
		});
		s.recomputed.update(|n| *n += 1);
	}

	fn colours(s: State) -> Option<(Legend, Vec<u8>, Vec<u8>)> {
		let layers: Vec<LayerSpec> = s.heavy.with_value(|h| h.model.as_ref().map(|m| m.layers()))?;
		let spec = layers.get(s.layer.get_untracked())?;
		let of = s.focus.get_untracked();
		let (legend, colors, shown) = s.heavy.with_value(|h| {
			let m = h.model.as_ref().expect("layers only exist once the model does");
			// `layers()` only offers a trade layer where there is a trade, so the miss below is that
			// agreement having been broken rather than a state to draw something for
			let (v, focus) = match spec.source {
				LayerRef::Study(i) => (std::borrow::Cow::Borrowed(m.layer(i)), None),
				LayerRef::Trade(t) => {
					let traded = m.traded()?;
					(
						traded.values(t, of, s.lambda.get_untracked(), &s.tiers.get_untracked(), &h.press, &h.unmet),
						matches!(t, TradeLayer::Share | TradeLayer::Seen).then(|| traded.trade.pois[of].poi.name.clone()),
					)
				}
			};
			let c = colorise(&v, &m.payload.imputed, s.hide_imputed.get_untracked(), spec.linear);
			Some((
				Legend {
					ticks: c.ticks,
					count: c.count,
					linear: spec.linear,
					focus,
				},
				c.colors,
				c.shown,
			))
		})?;
		s.heavy.update_value(|h| h.shown = shown.clone());
		Some((legend, colors, shown))
	}

	#[wasm_bindgen(module = "/src/map_core.js")]
	extern "C" {
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
			on_poi: &js_sys::Function,
			on_over: &js_sys::Function,
		) -> JsValue;
		#[wasm_bindgen(js_name = cells)]
		fn cells_js(el: &web_sys::HtmlElement, ring_x: &[f64], ring_y: &[f64], colors: &[u8], shown: &[u8]);
		#[wasm_bindgen(js_name = recenter)]
		fn recenter_js(el: &web_sys::HtmlElement, lat: f64, lng: f64, zoom: u8);
		#[wasm_bindgen(js_name = opacity)]
		fn opacity_js(el: &web_sys::HtmlElement, v: f64);
		#[wasm_bindgen(js_name = competitors)]
		fn competitors_js(el: &web_sys::HtmlElement, json: &str);
		#[wasm_bindgen(js_name = showTiers)]
		fn show_tiers_js(el: &web_sys::HtmlElement, flags: &[u8]);
		#[wasm_bindgen(js_name = pins)]
		fn pins_js(el: &web_sys::HtmlElement, json: &str);
		#[wasm_bindgen(js_name = pan)]
		fn pan_js(el: &web_sys::HtmlElement, lat: f64, lng: f64);
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
			s.lambda.track();
			s.tiers.track();
			if s.heavy.with_value(|h| h.model.is_some()) {
				recompute(s);
			}
		});

		Effect::new(move |_| {
			s.recomputed.track();
			s.layer.track();
			s.focus.track();
			s.hide_imputed.track();
			if !s.mounted.get() {
				return;
			}
			let (Some((legend, colors, shown)), Some(el)) = (colours(s), host()) else { return };
			s.legend.set(Some(legend));
			s.heavy.with_value(|h| {
				let m = h.model.as_ref().expect("colours only returns once the model is in");
				cells_js(&el, &m.ring_x, &m.ring_y, &colors, &shown);
			});
		});

		// the open card is a live view of λ, not a snapshot of when it was clicked
		Effect::new(move |_| {
			s.recomputed.track();
			s.selected.track();
			s.refresh_card();
		});

		Effect::new(move |_| {
			let v = s.opacity.get();
			if let Some(el) = host().filter(|_| s.mounted.get()) {
				opacity_js(&el, v);
			}
		});

		Effect::new(move |_| {
			let flags: Vec<u8> = s.tiers.get().iter().map(|t| u8::from(t.show)).collect();
			if let Some(el) = host().filter(|_| s.mounted.get()) {
				show_tiers_js(&el, &flags);
			}
		});

		Effect::new(move |_| {
			let lettered = s.lettered();
			let mut out: Vec<serde_json::Value> = lettered
				.iter()
				.map(|(l, p)| serde_json::json!({"id": p.id, "lat": p.at[0], "lng": p.at[1], "color": GREEN, "label": l, "title": p.name}))
				.collect();
			out.extend(
				s.temp
					.get()
					.iter()
					.map(|p| serde_json::json!({"id": p.id, "lat": p.at[0], "lng": p.at[1], "color": YELLOW, "label": p.label, "title": p.label.as_ref().or(p.address.as_ref())})),
			);
			if let Some(el) = host().filter(|_| s.mounted.get()) {
				pins_js(&el, &serde_json::to_string(&out).expect("a Vec<Value> serialises"));
			}
		});
	}

	/// The two axes the CLI was given, and whatever pairing it already built.
	#[derive(serde::Deserialize)]
	struct Studies {
		trades: Vec<String>,
		locations: Vec<String>,
		open: Vec<(Option<String>, String)>,
	}

	/// The product, the saved keys, and a tab per pairing the CLI named.
	async fn boot(s: State) {
		let served = match get::<Studies>("/studies.json").await {
			Ok(d) => d,
			Err(e) => return s.banner.set(Some(format!("⚠ what is served never arrived — {e}"))),
		};
		s.trades.set(served.trades);
		s.locations.set(served.locations);
		match crate::tabs::load_keys().await {
			Ok(Some(k)) => s.keys.set(k),
			Ok(None) => {}
			Err(e) => s.banner.set(Some(format!("⚠ keys — {e}"))),
		}
		for at in served.open {
			add(s, at).await;
		}
		// a product was served rather than one pairing, so the first choice is made the same way every
		// later one is
		match s.tabs.with_untracked(Vec::is_empty) {
			true => s.picker.set(Some(Pool::Study)),
			false => adopt(s, 0),
		}
	}

	/// Fetch a study, build its `Model`, and give it a tab. Mounts the map on the first one.
	async fn add(s: State, at: (Option<String>, String)) -> Option<usize> {
		let tab = fetch(s, at, false).await?;
		let model = tab.model.clone().expect("fetched with its model");
		s.tabs.update(|v| v.push(tab));
		if !s.mounted.get_untracked() && !mount(s, &model).await {
			return None;
		}
		Some(s.tabs.with_untracked(Vec::len) - 1)
	}

	/// The active tab's pairing, every answer under it asked of Google again. A refresh the day's quota
	/// stopped leaves the tab on what it had, and says so.
	pub fn reload(s: State) {
		wasm_bindgen_futures::spawn_local(async move {
			let i = s.active.get_untracked();
			let Some(at) = s.tabs.with_untracked(|v| v.get(i).map(|t| t.at.clone())) else { return };
			let Some(tab) = fetch(s, at, true).await else { return };
			if let Some(n) = tab.model.as_ref().and_then(|m| m.payload.notice.clone()) {
				return s.banner.set(Some(format!("⚠ nothing was replaced — {n}")));
			}
			s.tabs.update(|v| v[i] = tab);
			s.active.set(usize::MAX); // `adopt` is unconditional; `activate` would see `i` as current
			adopt(s, i);
		});
	}

	/// A study, as a tab that is not open yet.
	async fn fetch(s: State, at: (Option<String>, String), refresh: bool) -> Option<Tab> {
		// a pairing the server has not built yet reads the grid archive and every POI page, which is
		// tens of seconds of nothing to look at
		let shown = match &at.0 {
			Some(t) => format!("{t} in {}", at.1),
			None => at.1.clone(),
		};
		s.banner.set(Some(format!("{} {shown} …", if refresh { "re-asking Google for" } else { "building" })));
		let location = js_sys::encode_uri_component(&at.1);
		let req = match &at.0 {
			Some(t) => {
				let url = format!("/payload/{}/{location}", js_sys::encode_uri_component(t));
				if refresh { gloo_net::http::Request::post(&url) } else { gloo_net::http::Request::get(&url) }
			}
			None => gloo_net::http::Request::get(&format!("/place/{location}")),
		};
		let payload = match send::<gmaps_optimal_placement_core::Payload>(req).await {
			Ok(p) => p,
			Err(e) => {
				s.banner.set(Some(format!("⚠ {shown} — {e}")));
				return None;
			}
		};
		let model = match gmaps_optimal_placement_core::Model::try_new(payload) {
			Ok(m) => std::rc::Rc::new(m),
			Err(e) => {
				s.banner.set(Some(format!("⚠ {shown} — {e}")));
				return None;
			}
		};
		s.banner.set(None);
		let core = crate::pins::seed(&model.payload).await.unwrap_or_else(|e| {
			s.banner.set(Some(format!("⚠ pins — {e}")));
			Vec::new()
		});
		Some(Tab {
			at,
			label: model.payload.name.clone(),
			// inert without a trade: the slider is disabled and nothing reads the number
			lambda: model.payload.trade.as_ref().map_or(2000., |t| t.lambda_m),
			layer: 0,
			focus: 0,
			hide_imputed: false,
			tiers: model.tier_states(),
			core,
			temp: Vec::new(),
			selected: None,
			model: Some(model),
		})
	}

	/// The live signals, back into the tab they belong to.
	fn stash(s: State) {
		let i = s.active.get_untracked();
		s.tabs.update(|v| {
			let Some(t) = v.get_mut(i) else { return };
			t.lambda = s.lambda.get_untracked();
			t.layer = s.layer.get_untracked();
			t.focus = s.focus.get_untracked();
			t.hide_imputed = s.hide_imputed.get_untracked();
			t.tiers = s.tiers.get_untracked();
			t.core = s.core.get_untracked();
			t.temp = s.temp.get_untracked();
			t.selected = s.selected.get_untracked();
		});
	}

	/// A tab, into the live signals. `press` and `unmet` are never stored — they are functions of λ
	/// and the tier weights, so `recompute` is all it takes to get them back.
	fn adopt(s: State, i: usize) {
		let Some(t) = s.tabs.with_untracked(|v| v.get(i).cloned()) else { return };
		let Some(m) = t.model else { return };
		s.active.set(i);
		s.heavy.update_value(|h| h.model = Some(m.clone()));
		s.lambda.set(t.lambda);
		s.layer.set(t.layer);
		s.focus.set(t.focus);
		s.hide_imputed.set(t.hide_imputed);
		s.tiers.set(t.tiers);
		s.core.set(t.core);
		s.temp.set(t.temp);
		s.report.set(None);
		s.selected.set(t.selected);
		s.loaded.set(Some(super::Loaded {
			layers: m.layers().into_iter().map(|l| (l.name, l.note)).collect(),
			tier_counts: m
				.payload
				.trade
				.iter()
				.flat_map(|t| t.tiers.iter().map(|x| t.pois.iter().filter(|p| p.poi.tier == x.name).count()))
				.collect(),
			categories: m.payload.trade.iter().flat_map(categories).collect(),
			imputed: m.payload.imputed.iter().filter(|&&i| i == 1).count(),
			ages: {
				let now = js_sys::Date::now() / 1000.;
				m.payload
					.trade
					.iter()
					.flat_map(|t| {
						t.pois.iter().map(move |p| {
							let tier = t.tiers.iter().position(|x| x.name == p.poi.tier).expect("the model rejected a payload whose tiers disagree");
							// an answer dated ahead of this clock is a clock that moved, and the answer is still not old
							(((now - p.poi.fetched_s as f64) / 86_400.).max(0.), tier)
						})
					})
					.collect()
			},
			queries: m.payload.trade.iter().flat_map(|t| t.queries.iter().map(|q| (q.text.clone(), q.hits))).collect(),
			traded: m.traded().is_some(),
			notice: m.payload.notice.clone(),
		}));
		// a tab carries its own, so switching away from a refusal does not leave it over the next map
		s.banner.set(m.payload.notice.clone().map(|n| format!("⚠ {n}")));
		document().set_title(&m.payload.name);
		// before the sweep, which is the one slow thing here and which no marker depends on
		if let Some(el) = host().filter(|_| s.mounted.get_untracked()) {
			competitors_js(&el, &competitors(&m));
			recenter_js(&el, m.payload.center[0], m.payload.center[1], m.payload.zoom);
		}
		recompute(s);
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
		s.active.set(usize::MAX); // `adopt` is unconditional; `activate` would see `next` as current
		adopt(s, next);
	}

	/// No tabs: the panel, the report and the map all say so.
	fn blank(s: State) {
		s.active.set(0);
		s.heavy.update_value(|h| h.model = None);
		s.loaded.set(None);
		s.legend.set(None);
		s.report.set(None);
		s.selected.set(None);
		s.core.set(Vec::new());
		s.temp.set(Vec::new());
		s.tiers.set(Vec::new());
		document().set_title("studies");
		if let Some(el) = host().filter(|_| s.mounted.get_untracked()) {
			cells_js(&el, &[], &[], &[], &[]);
			competitors_js(&el, "[]");
		}
	}

	/// The picker's cursor, kept inside its own scroll box. Rows are one line each — see `#picker li`
	/// — so the cursor's offset is its index times a row.
	pub fn scroll_pick(row: usize) {
		let Some(li) = document().query_selector("#picker .col.on li.cur").ok().flatten() else {
			return;
		};
		let Some(ul) = li.parent_element() else { return };
		let (h, seen) = (li.client_height(), ul.client_height());
		let (top, bottom) = (row as i32 * h, (row as i32 + 1) * h);
		if top < ul.scroll_top() {
			ul.set_scroll_top(top);
		} else if bottom > ul.scroll_top() + seen {
			ul.set_scroll_top(bottom - seen);
		}
	}

	#[derive(serde::Deserialize)]
	struct Found {
		at: [f64; 2],
		address: String,
	}

	/// Asked of the server, because what a search may spend is `--block-spend`'s to say. Searched in
	/// the open tab's frame, which is also what keeps the answer's cache key the same next time.
	pub fn find(s: State, q: String) {
		let Some(location) = s.tabs.with_untracked(|t| t.get(s.active.get_untracked()).map(|t| t.at.1.clone())) else {
			return s.missed.set(Some("open a location to search in first".to_owned()));
		};
		let url = format!("/find/{location}?q={}", js_sys::encode_uri_component(&q));
		leptos::task::spawn_local(async move {
			match get::<Option<Found>>(&url).await {
				Err(e) => s.missed.set(Some(e)),
				Ok(None) => s.missed.set(Some(format!("no place found for {q:?}"))),
				Ok(Some(f)) => {
					s.missed.set(None);
					if let Some(el) = host().filter(|_| s.mounted.get_untracked()) {
						pan_js(&el, f.at[0], f.at[1]);
					}
					click(s, f.at, Some(f.address));
				}
			}
		});
	}

	/// A yellow pin and its card. `address` is Google's, when a search rather than a click put it there.
	fn click(s: State, at: [f64; 2], address: Option<String>) {
		let id = s.fresh_id();
		s.temp.update(|v| {
			v.push(Pin {
				id: id.clone(),
				name: String::new(),
				at,
				label: None,
				address,
				green: false,
				from_study: false,
			})
		});
		s.open(&id);
	}

	fn focus_find() {
		let el: web_sys::HtmlElement = document().get_element_by_id("find").expect("the tab bar renders it").unchecked_into();
		el.focus().expect("the place field takes focus");
	}

	pub fn persist_keys(s: State) {
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
		if ev.ctrl_key() && !ev.meta_key() && !ev.alt_key() && k == "k" && s.picker.get_untracked().is_none() && !s.settings.get_untracked() {
			ev.prevent_default();
			return focus_find();
		}
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
		// a yellow pin is a click and nothing has been written down for it, so it undoes like one
		if k == "Backspace" {
			if let Some(p) = s.selected.get_untracked().and_then(|id| s.pin(&id)).filter(|p| !p.green) {
				ev.prevent_default();
				crate::pins::forget(s, p);
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
			_ if k == m.place || k == "/" => {
				focus_find();
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
		send(gloo_net::http::Request::get(url)).await
	}

	async fn send<T: serde::de::DeserializeOwned>(req: gloo_net::http::RequestBuilder) -> Result<T, String> {
		match req.send().await {
			Err(e) => Err(e.to_string()),
			Ok(r) if !r.ok() => Err(r.text().await.unwrap_or_else(|_| r.status_text())),
			Ok(r) => r.json::<T>().await.map_err(|e| format!("it did not parse — {e}")),
		}
	}

	/// The whole competitor inventory, as `map_core.js` wants it. Colour says Google's category, size
	/// says the study's tier. Two questions, and reading one off the other is exactly the mistake the
	/// category list is there to make visible.
	fn competitors(model: &gmaps_optimal_placement_core::Model) -> String {
		let Some(trade) = &model.payload.trade else { return "[]".to_owned() };
		let cats = categories(trade);
		let out: Vec<serde_json::Value> = trade
			.pois
			.iter()
			.map(|p| {
				let ti = trade.tiers.iter().position(|t| t.name == p.poi.tier).expect("the model rejected a payload whose tiers disagree");
				let c = cats.iter().find(|c| c.kind == p.poi.kind).expect("the list was built from these same pois");
				serde_json::json!({
					"lat": p.poi.lat, "lng": p.poi.lng, "name": p.poi.name, "addr": p.poi.addr,
					"kind": c.label, "tier": p.poi.tier, "rating": p.poi.rating, "n_rev": p.poi.n_rev,
					"web": p.poi.web, "tel": p.poi.tel, "ti": ti, "big": ti == 0, "color": c.color,
				})
			})
			.collect();
		serde_json::to_string(&out).expect("a Vec<Value> serialises")
	}

	/// The one `google.maps` instance, on the first study to arrive. Its callbacks read the live
	/// signals, so they outlive every tab switch.
	async fn mount(s: State, model: &gmaps_optimal_placement_core::Model) -> bool {
		let Some(el) = host() else {
			s.banner.set(Some("⚠ the map host element is missing".to_owned()));
			return false;
		};
		let on_click = leak(Closure::<dyn Fn(f64, f64)>::new(move |lat: f64, lng: f64| click(s, [lat, lng], None)));
		let on_move = leak(Closure::<dyn Fn(f64, f64, f64, f64)>::new(move |lat: f64, lng: f64, x: f64, y: f64| {
			let text = s.heavy.with_value(|h| {
				let m = h.model.as_ref()?;
				let i = m.cell_at(lat, lng, &h.shown)?;
				Some(m.tooltip(i, &h.press, &h.unmet))
			});
			s.tip.set(text.map(|text| Tip { text, x, y }));
		}));
		let on_out = leak(Closure::<dyn Fn()>::new(move || s.tip.set(None)));
		let on_pin = leak(Closure::<dyn Fn(String)>::new(move |id: String| s.open(&id)));
		// the marker order is the payload's own, so the index needs no lookup
		let on_poi = leak(Closure::<dyn Fn(usize)>::new(move |i: usize| s.focus.set(i)));
		// a marker eats the map's `mousemove`, so without this the tooltip a cell was showing simply
		// stops on whatever it last said
		let on_over = leak(Closure::<dyn Fn(usize, f64, f64)>::new(move |i: usize, x: f64, y: f64| {
			let text = s.heavy.with_value(|h| Some(h.model.as_ref()?.payload.trade.as_ref()?.pois[i].poi.name.clone()));
			s.tip.set(text.map(|text| Tip { text, x, y }));
		}));

		let (c, zoom) = (model.payload.center, model.payload.zoom);
		if let Some(msg) = mount_js(el, c[0], c[1], zoom, &on_click, &on_move, &on_out, &on_pin, &on_poi, &on_over).await.as_string() {
			s.banner.set(Some(msg));
			return false;
		}
		s.mounted.set(true);
		true
	}
}
