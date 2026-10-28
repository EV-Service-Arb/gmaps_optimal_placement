//! The tab bar, the study picker and the key bindings behind them.
//!
//! A tab is one study's snapshot of the controls — λ, layer, tiers, pins, the open card — and the
//! `Model` those controls read. Switching writes the live signals back into the tab being left and
//! reads the next one into them; there is never a second `google.maps` instance.
//!
//! ```text
//!   GET /studies.json   trades, locations: the two axes the CLI was given
//!                       open:              the pairing it was named, if it was named one
//!        │
//!   ┌────┴──── t ─→ Pool::Study    a field per axis, what and where; Load spends the pair
//!   │  Picker                      ↑ also where a served product starts
//!   │                              the trade field ends in NO_TRADE — the city, unpriced
//!   └───────── f ─→ Pool::Open     filter the open tabs, switch to one
//!
//!   s / `/` / Ctrl+K ─→ #find      a place, by name or address; a hit lands as a click there
//! ```
//!
//! One field per axis rather than one list of every pairing: a study is a point on a product, and an
//! agglomeration times a trade list is a long list to read when what you know is one coordinate.
//! Both fields are live at once, so a pairing is two choices in either order — which is what a step
//! sequence could not do, having already spent the first choice by the time the second is on
//! screen. Neither field loads anything on its own; Load spends the pair, and marks whichever
//! field is still empty.
//!
//! [`NO_TRADE`] sits at the end of the trade field rather than at its head: a city on its own is the
//! cheap question, not the usual one, and the highlight opens on whatever a field's first row is.
//!
//! This is the only picker: `serve` does not run `fzf` over a directory, because choosing the first
//! study and choosing the fourth should not be two different motions.
//!
//! Chrome consumes `Ctrl+T`, `Ctrl+W` and `Ctrl+1..9` before a page sees them, so the defaults are
//! bare keys. The map affords it: its only text inputs are the picker and the place field, and the
//! keymap stands aside while either has the caret.
#[cfg(feature = "ssr")]
use std::path::PathBuf;

use ev_lib::uikit_leptos::{Button, Command, CommandEmpty, CommandInput, CommandItem, CommandList};
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::map::State;

/// The last row of the trade field: a location built from the statistical archive alone, which is
/// every layer a trade did not add and not one billed call.
pub const NO_TRADE: &str = "(no trade · grid only)";

/// Which set the picker is filtering, and so how many fields it puts up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pool {
	/// The product served, as one field per axis. Loading the pair opens the tab.
	Study,
	/// The open tabs; loading one switches to it.
	Open,
}
/// The bare keys, one per action. Digits jump to a tab by position and are not rebindable — there is
/// nothing to rebind them to.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)] // a binding added since the file was written was never rebound
pub struct Keymap {
	pub prev: String,
	pub next: String,
	pub close: String,
	pub open: String,
	pub find: String,
	pub place: String,
}
impl Default for Keymap {
	fn default() -> Self {
		Self {
			prev: "[".to_owned(),
			next: "]".to_owned(),
			close: "w".to_owned(),
			open: "t".to_owned(),
			find: "f".to_owned(),
			place: "s".to_owned(),
		}
	}
}
impl Keymap {
	/// In the order the settings panel lists them.
	fn rows(&self) -> [(&'static str, &str); 6] {
		[
			("previous tab", &self.prev),
			("next tab", &self.next),
			("close the tab", &self.close),
			("open a study", &self.open),
			("find an open tab", &self.find),
			("find a place", &self.place),
		]
	}

	pub fn set(&mut self, label: &str, key: String) {
		let slot = match label {
			"previous tab" => &mut self.prev,
			"next tab" => &mut self.next,
			"close the tab" => &mut self.close,
			"open a study" => &mut self.open,
			"find an open tab" => &mut self.find,
			"find a place" => &mut self.place,
			other => unreachable!("the panel only lists rows(): {other:?}"),
		};
		*slot = key;
	}
}

/// Config, not data: a keyboard layout is how this machine is driven, and it does not sit beside the
/// pins.
#[server(input = leptos::server_fn::codec::Json)]
pub async fn save_keys(keys: Keymap) -> Result<(), ServerFnError> {
	let json = serde_json::to_string_pretty(&keys).map_err(|e| ServerFnError::new(format!("{e}")))?;
	std::fs::write(path()?, json).map_err(|e| ServerFnError::new(format!("writing the key file: {e}")))
}

#[server]
pub async fn load_keys() -> Result<Option<Keymap>, ServerFnError> {
	match std::fs::read_to_string(path()?) {
		Ok(s) => Ok(Some(serde_json::from_str(&s).map_err(|e| ServerFnError::new(format!("parsing the key file: {e}")))?)),
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
		Err(e) => Err(ServerFnError::new(format!("reading the key file: {e}"))),
	}
}

#[cfg(feature = "ssr")]
fn path() -> Result<PathBuf, ServerFnError> {
	xdg::BaseDirectories::with_prefix("gmaps_optimal_placement")
		.place_config_file("keys.json")
		.map_err(|e| ServerFnError::new(format!("placing the key file: {e}")))
}

#[component]
pub fn TabBar(state: State) -> impl IntoView {
	view! {
		<div id="tabs">
			{move || {
				state
					.tabs
					.get()
					.into_iter()
					.enumerate()
					.map(|(i, t)| {
						view! {
							<span
								class="tab"
								class:on=move || state.active.get() == i
								on:click=move |_| crate::map::activate(state, i)
							>
								{t.label}
								<span
									class="x"
									title="close"
									on:click=move |ev| {
										ev.stop_propagation();
										crate::map::close(state, i);
									}
								>
									"●"
								</span>
							</span>
						}
					})
					.collect_view()
			}}
			<span
				class="tab add"
				title="open a study"
				on:click=move |_| state.picker.set(Some(Pool::Study))
			>
				"+"
			</span>
			<input
				id="find"
				type="text"
				placeholder="find a place"
				title="s, / or Ctrl+K"
				on:focus=move |ev| { event_target::<leptos::web_sys::HtmlInputElement>(&ev).select() }
				on:input=move |_| state.missed.set(None)
				on:keydown=move |ev| {
					let field = event_target::<leptos::web_sys::HtmlInputElement>(&ev);
					match ev.key().as_str() {
						"Enter" if !field.value().trim().is_empty() => {
							crate::map::find(state, field.value());
							field.blur().expect("the place field gives up focus");
						}
						"Escape" => {
							state.missed.set(None);
							field.blur().expect("the place field gives up focus");
						}
						_ => {}
					}
				}
			/>
			<div
				id="missed"
				hidden=move || state.missed.with(Option::is_none)
				prop:textContent=move || state.missed.get().map(|m| format!("⚠ {m}"))
			></div>
			<span
				id="gear"
				title="key bindings"
				on:click=move |_| {
					state.arming.set(None);
					state.settings.update(|v| *v = !*v);
				}
			>
				"⚙"
			</span>
		</div>
	}
}

/// The picker's frame. Tailwind over the kit's tokens, like the `Command`s inside it; `map.css` does
/// not reach in.
const PICKER: &str = "absolute top-12 left-1/2 z-9 flex -translate-x-1/2 items-start gap-2 rounded-lg border border-border bg-background p-2 text-ink shadow-lg";

/// `Pool::Study` is one uikit `Command` per axis and a Load: each field filters and picks on its own,
/// and only Load spends the pair. `Pool::Open` is one `Command` whose pick is the switch. `Escape`
/// is `map::key`'s, which the field lets through.
#[component]
pub fn Picker(state: State, pool: Pool) -> impl IntoView {
	let root = NodeRef::<leptos::html::Div>::new();
	// the overlay is modal, so typing goes to its first field
	Effect::new(move |_| {
		use leptos::wasm_bindgen::JsCast;
		if let Some(el) = root.get() {
			el.query_selector("input")
				.expect("a valid selector")
				.expect("every pool puts up a field")
				.unchecked_into::<leptos::web_sys::HtmlElement>()
				.focus()
				.expect("the picker's field takes focus");
		}
	});
	match pool {
		Pool::Study => {
			// the outer `None` is no pick; the inner one is NO_TRADE, the row the server has no file for
			let trade = RwSignal::new(None::<Option<String>>);
			let location = RwSignal::new(None::<String>);
			let tried = RwSignal::new(false);
			let load = move |_| match (trade.get_untracked(), location.get_untracked()) {
				(Some(trade), Some(location)) => {
					state.picker.set(None);
					crate::map::open(state, trade, location);
				}
				_ => tried.set(true),
			};
			view! {
				<div id="picker" class=PICKER node_ref=root>
					<Command class="w-64 border border-border">
						<CommandInput
							placeholder="what is being sold"
							invalid=Signal::derive(move || tried.get() && trade.with(Option::is_none))
						/>
						<CommandList>
							<CommandEmpty>"no such trade"</CommandEmpty>
							{move || {
								state
									.trades
									.get()
									.into_iter()
									.map(Some)
									.chain([None])
									.map(|t| {
										let name = match &t {
											Some(t) => t.clone(),
											None => NO_TRADE.to_owned(),
										};
										let at = t.clone();
										view! {
											<Pick
												name
												picked=Signal::derive(move || trade.with(|p| p.as_ref() == Some(&at)))
												on_select=Callback::new(move |_| trade.set(Some(t.clone())))
											/>
										}
									})
									.collect_view()
							}}
						</CommandList>
					</Command>
					<Command class="w-64 border border-border">
						<CommandInput
							placeholder="and where"
							invalid=Signal::derive(move || tried.get() && location.with(Option::is_none))
						/>
						<CommandList>
							<CommandEmpty>"no such location"</CommandEmpty>
							{move || {
								state
									.locations
									.get()
									.into_iter()
									.map(|l| {
										let at = l.clone();
										view! {
											<Pick
												name=l.clone()
												picked=Signal::derive(move || location.with(|p| p.as_ref() == Some(&at)))
												on_select=Callback::new(move |_| location.set(Some(l.clone())))
											/>
										}
									})
									.collect_view()
							}}
						</CommandList>
					</Command>
					<Button on_click=Callback::new(load)>"Load"</Button>
				</div>
			}
			.into_any()
		}
		Pool::Open => view! {
			<div id="picker" class=PICKER node_ref=root>
				<Command class="w-80 border border-border">
					<CommandInput placeholder="find an open tab" />
					<CommandList>
						<CommandEmpty>"no such tab"</CommandEmpty>
						{move || {
							state
								.tabs
								.get()
								.into_iter()
								.enumerate()
								.map(|(i, t)| {
									let label = t.label;
									view! {
										<CommandItem
											value=label.clone()
											on_select=Callback::new(move |_| {
												state.picker.set(None);
												crate::map::activate(state, i);
											})
										>
											<span class="truncate">{label.clone()}</span>
										</CommandItem>
									}
								})
								.collect_view()
						}}
					</CommandList>
				</Command>
			</div>
		}
		.into_any(),
	}
}

/// A row that stays marked once picked, whatever the query does after.
#[component]
fn Pick(name: String, picked: Signal<bool>, on_select: Callback<String>) -> impl IntoView {
	view! {
		<CommandItem value=name.clone() on_select>
			<span class="truncate">{name.clone()}</span>
			{move || {
				picked
					.get()
					.then(|| {
						view! {
							<svg
								xmlns="http://www.w3.org/2000/svg"
								viewBox="0 0 24 24"
								fill="none"
								stroke="currentColor"
								stroke-width="2"
								stroke-linecap="round"
								stroke-linejoin="round"
								class="ml-auto text-primary-ink"
								role="img"
								aria-label="picked"
							>
								<path d="M20 6 9 17l-5-5"></path>
							</svg>
						}
					})
			}}
		</CommandItem>
	}
}

/// Click a key, then press the new one — the press is read off the document, in `map::key`.
#[component]
pub fn Settings(state: State) -> impl IntoView {
	view! {
		<div id="settings" class="panel">
			<h4>"Keys"</h4>
			{move || {
				state
					.keys
					.get()
					.rows()
					.map(|(label, key)| {
						let key = key.to_owned();
						view! {
							<div class="row">
								<label>{label}</label>
								<button class="sec" on:click=move |_| state.arming.set(Some(label))>
									{move || {
										if state.arming.get() == Some(label) { "…".to_owned() } else { key.clone() }
									}}
								</button>
							</div>
						}
					})
					.collect_view()
			}}
			<div class="note">
				"1–9 jump to a tab, 0 to the last; / and Ctrl+K also find a place. Click a key, then press the new one."
			</div>
			<button on:click=move |_| {
				state.arming.set(None);
				state.settings.set(false);
			}>"Close"</button>
		</div>
	}
}
