//! Pins, their lifecycle, and the card that is now their control surface.
//!
//! Two lifetimes, kept apart: `core` is green, lettered and outlives the session; `temp` is yellow
//! and does not. The one `pins[]` array the old page had mixed them, which is why "clear pins" took
//! the study's own candidates with it, and why a free-click pin was unreachable the moment the card
//! moved on.
//!
//! ```text
//!   study.nix   candidate = [ … ]         ← committed, arguable, in git
//!        │
//!        │   $XDG_DATA_HOME/gmaps_optimal_placement/pins-<study>.json
//!        ├── minus  hidden   ← ✕ on a config pin
//!        └── plus   added    ← ✓ on a yellow pin
//!        ▼
//!   effective green set → lettered A,B,C…
//! ```
//!
//! The study file stays the committed seed and the XDG file is a diff over it: candidates are part
//! of the study's question, so they do not move out wholesale — but every one is removable and every
//! promotion lands outside the study file. Data, not cache: a promoted candidate is a decision, and
//! cache is what cleaners delete.
#[cfg(feature = "ssr")]
use std::path::PathBuf;

use gmaps_optimal_placement_core::{Candidate, Payload, model::Report};
pub use imp::{copy, forget, keep};
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::map::State;

#[component]
pub fn Card(state: State, report: Report) -> impl IntoView {
	let pin = move || state.selected.get().and_then(|id| state.pin(&id));
	view! {
		<div class="panel" id="report">
			<div class="acts">
				{move || {
					pin()
						.map(|p| {
							let (promote, at, drop) = (p.clone(), p.at, p.clone());
							view! {
								{(!p.green)
									.then(|| {
										view! {
											<button
												class="act go"
												title="keep this one"
												on:click=move |_| keep(state, promote.clone())
											>
												"✓"
											</button>
										}
									})}
								<button
									class="act"
									title="copy a Google Maps link"
									on:click=move |_| copy(state, at)
								>
									{move || match state.copied.get() {
										None => "⧉",
										Some(true) => "✓",
										Some(false) => "✕",
									}}
								</button>
								<button
									class="act no"
									title="drop this pin"
									on:click=move |_| forget(state, drop.clone())
								>
									"✕"
								</button>
							}
						})
				}}
			</div>
			<h4>{report.title}</h4>
			<table>
				{report
					.rows
					.into_iter()
					.map(|r| {
						view! {
							<tr>
								<td class="k">
									{r.label}
									{r
										.sub
										.map(|s| {
											view! {
												<br />
												<span>"  " {s}</span>
											}
										})}
								</td>
								<td>{r.value}</td>
							</tr>
						}
					})
					.collect_view()}
			</table>
			<div class="note">{report.note}</div>
		</div>
	}
}

/// JSON in: the default form encoding cannot carry a struct of two lists.
#[server(input = leptos::server_fn::codec::Json)]
pub async fn save_pins(study: String, pins: PinFile) -> Result<(), ServerFnError> {
	let json = serde_json::to_string_pretty(&pins).map_err(|e| ServerFnError::new(format!("{e}")))?;
	std::fs::write(path(&study)?, json).map_err(|e| ServerFnError::new(format!("writing the pin file for {study:?}: {e}")))
}

/// Keyed by the study's name, so editing λ or a layer keeps your pins.
#[server]
pub async fn load_pins(study: String) -> Result<Option<PinFile>, ServerFnError> {
	match std::fs::read_to_string(path(&study)?) {
		Ok(s) => Ok(Some(
			serde_json::from_str(&s).map_err(|e| ServerFnError::new(format!("parsing the pin file for {study:?}: {e}")))?,
		)),
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
		Err(e) => Err(ServerFnError::new(format!("reading the pin file for {study:?}: {e}"))),
	}
}

/// The green set the map opens with: the study's seed, minus `hidden`, plus `added`.
pub async fn seed(payload: &Payload) -> Result<Vec<Pin>, String> {
	let diff = load_pins(payload.name.clone()).await.map_err(|e| e.to_string())?.unwrap_or_default();
	let mut out: Vec<Pin> = payload.candidates.iter().filter(|c| !diff.hidden.contains(&c.name)).map(|c| green(c, true)).collect();
	out.extend(diff.added.iter().map(|c| green(c, false)));
	Ok(out)
}

/// The diff over the study's own `candidate` list.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PinFile {
	pub added: Vec<Candidate>,
	/// Candidate names the study declares and this machine does not want to see.
	pub hidden: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
	/// A green pin is keyed by its name, a yellow one by a counter. Stable for the pin's life, which
	/// is what the card and the marker click agree on.
	pub id: String,
	pub name: String,
	/// [lat, lon]
	pub at: [f64; 2],
	/// Yellow only: the sweep's rank, or nothing for a free click.
	pub label: Option<String>,
	/// Yellow only: Google's address for a pin a place search put down. Not `name`, which is the
	/// trading name the what-if is scored under.
	pub address: Option<String>,
	pub green: bool,
	/// Named by the study file, rather than promoted here. Only these need confirming before a `✕`.
	pub from_study: bool,
}
fn green(c: &Candidate, from_study: bool) -> Pin {
	Pin {
		id: c.name.clone(),
		name: c.name.clone(),
		at: c.at,
		label: None,
		address: None,
		green: true,
		from_study,
	}
}

#[cfg(feature = "ssr")]
fn path(study: &str) -> Result<PathBuf, ServerFnError> {
	xdg::BaseDirectories::with_prefix("gmaps_optimal_placement")
		.place_data_file(format!("pins-{study}.json"))
		.map_err(|e| ServerFnError::new(format!("placing the pin file for {study:?}: {e}")))
}

#[cfg(not(feature = "hydrate"))]
mod imp {
	use super::{Pin, State};

	pub fn copy(_: State, _: [f64; 2]) {}
	pub fn keep(_: State, _: Pin) {}
	pub fn forget(_: State, _: Pin) {}
}

#[cfg(feature = "hydrate")]
mod imp {
	use gmaps_optimal_placement_core::Candidate;
	use leptos::prelude::*;
	use wasm_bindgen::prelude::*;

	use super::{Pin, PinFile, State, save_pins};

	/// What the effective set implies for the file. Recomputed from the pins rather than patched, so the
	/// file can never disagree with what is on screen.
	fn diff(payload: &gmaps_optimal_placement_core::Payload, core: &[Pin]) -> PinFile {
		PinFile {
			added: core.iter().filter(|p| !p.from_study).map(|p| Candidate { name: p.name.clone(), at: p.at }).collect(),
			hidden: payload.candidates.iter().map(|c| c.name.clone()).filter(|n| !core.iter().any(|p| &p.name == n)).collect(),
		}
	}

	/// The coordinates, as the link that opens them where a decision actually gets made.
	pub fn copy(state: State, at: [f64; 2]) {
		let url = format!("https://www.google.com/maps/search/?api=1&query={},{}", at[0], at[1]);
		leptos::task::spawn_local(async move {
			let ok = wasm_bindgen_futures::JsFuture::from(window().navigator().clipboard().write_text(&url)).await.is_ok();
			state.copied.set(Some(ok));
			if ok {
				// a rejection stays on screen: there is nothing in the paste buffer to go looking for
				let reset = Closure::once_into_js(move || state.copied.set(None));
				let _ = window().set_timeout_with_callback_and_timeout_and_arguments_0(reset.unchecked_ref(), 1200);
			}
		});
	}

	/// Yellow to green. The default name is the address a search found it at, or else whatever the
	/// grid calls the cell it landed in — the one label the map already has for the place.
	pub fn keep(state: State, pin: Pin) {
		let default = pin.address.clone().or_else(|| {
			state.heavy.with_value(|h| {
				let m = h.model.as_ref()?;
				let (mx, my) = m.to_local(pin.at[0], pin.at[1]);
				Some(m.payload.place[m.nearest_cell(mx, my)].clone())
			})
		});
		let Ok(Some(name)) = window().prompt_with_message_and_default("Name this candidate", &default.unwrap_or_default()) else {
			return;
		};
		let name = name.trim().to_owned();
		if name.is_empty() {
			return;
		}
		if state.core.get_untracked().iter().any(|p| p.name == name) {
			return state.banner.set(Some(format!("⚠ there is already a candidate called {name:?}")));
		}
		state.temp.update(|v| v.retain(|p| p.id != pin.id));
		state.core.update(|v| {
			v.push(Pin {
				id: name.clone(),
				name,
				at: pin.at,
				label: None,
				address: None,
				green: true,
				from_study: false,
			})
		});
		persist(state);
		let id = state.core.get_untracked().last().expect("just pushed").id.clone();
		state.open(&id);
	}

	/// A yellow pin is a click; one the study file named is an argument, so that one is confirmed.
	pub fn forget(state: State, pin: Pin) {
		if pin.from_study
			&& !window()
				.confirm_with_message(&format!("{:?} is a candidate in the study file. Hide it on this machine?", pin.name))
				.unwrap_or(false)
		{
			return;
		}
		if pin.green {
			state.core.update(|v| v.retain(|p| p.id != pin.id));
			persist(state);
		} else {
			state.temp.update(|v| v.retain(|p| p.id != pin.id));
		}
		state.selected.set(None);
		state.report.set(None);
	}

	fn persist(state: State) {
		let Some((study, file)) = state.heavy.with_value(|h| {
			let m = h.model.as_ref()?;
			Some((m.payload.name.clone(), diff(&m.payload, &state.core.get_untracked())))
		}) else {
			return;
		};
		leptos::task::spawn_local(async move {
			if let Err(e) = save_pins(study, file).await {
				state.banner.set(Some(format!("⚠ the pin file was not written — {e}")));
			}
		});
	}
}
