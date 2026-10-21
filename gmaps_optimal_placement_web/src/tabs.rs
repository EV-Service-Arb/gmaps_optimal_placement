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
//!   ┌────┴──── t ─→ Pool::Study    two boxes, what and where; Enter picks, Ctrl-Enter loads
//!   │  Picker                      ↑ also where a served product starts
//!   │                              the trade field ends in NO_TRADE — the city, unpriced
//!   └───────── f ─→ Pool::Open     filter the open tabs, switch to one
//!
//!   s / `/` / Ctrl+K ─→ #find      a place, by name or address; a hit lands as a click there
//! ```
//!
//! One box per axis rather than one list of every pairing: a study is a point on a product, and an
//! agglomeration times a trade list is a long list to read when what you know is one coordinate.
//! Both boxes are live at once, so a pairing is two choices in either order — which is what a step
//! sequence could not do, having already spent the first choice by the time the second is on
//! screen. Neither box loads anything on its own; the `↵` button spends the pair.
//!
//! [`NO_TRADE`] sits at the end of the trade box rather than at its head: a city on its own is the
//! cheap question, not the usual one, and the cursor opens on whatever a box's first row is.
//!
//! This is the only picker: `serve` does not run `fzf` over a directory, because choosing the first
//! study and choosing the fourth should not be two different motions.
//!
//! Chrome consumes `Ctrl+T`, `Ctrl+W` and `Ctrl+1..9` before a page sees them, so the defaults are
//! bare keys. The map affords it: its only text inputs are the picker and the place field, and the
//! keymap stands aside while either has the caret.
#[cfg(feature = "ssr")]
use std::path::PathBuf;

use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::map::State;

/// The last row of the trade field: a location built from the statistical archive alone, which is
/// every layer a trade did not add and not one billed call.
pub const NO_TRADE: &str = "(no trade · grid only)";

/// Which set the picker is filtering, and so how many boxes it puts up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pool {
	/// The product served, as one box per axis. Loading the pair opens the tab.
	Study,
	/// The open tabs; loading one switches to it.
	Open,
}
impl Pool {
	/// How many boxes it puts up: one per axis being crossed.
	fn cols(self) -> usize {
		match self {
			Self::Study => 2,
			Self::Open => 1,
		}
	}

	fn prompt(self, col: usize) -> &'static str {
		match (self, col) {
			(Self::Study, 0) => "what is being sold",
			(Self::Study, _) => "and where",
			(Self::Open, _) => "find an open tab",
		}
	}
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

/// One selection box per axis: a query narrows its list, `Enter` or a click picks the row, and the
/// pick stays put — it is a value, not a row number, so editing the query after does not move it.
/// `Ctrl-Enter` and the `↵` button load what the boxes hold, and nothing else does; `Escape` closes.
///
/// ↑/↓ (and `Ctrl-P`/`Ctrl-N`) clamp at both ends rather than wrap and `Tab` crosses boxes. The
/// cursor is only where the keyboard is looking: it opens on the first hit and returns there on
/// every edit, while the pick below it does not move.
#[component]
pub fn Picker(state: State, pool: Pool) -> impl IntoView {
	let cols = pool.cols();
	// one query, one cursor and one pick per axis, and which box has the caret
	let query = [RwSignal::new(String::new()), RwSignal::new(String::new())];
	let sel = [RwSignal::new(0usize), RwSignal::new(0usize)];
	let pick: [RwSignal<Option<(usize, String)>>; 2] = [RwSignal::new(None), RwSignal::new(None)];
	let side = RwSignal::new(0usize);
	let field = [NodeRef::<leptos::html::Input>::new(), NodeRef::<leptos::html::Input>::new()];

	let hits: [Memo<Vec<(usize, String)>>; 2] = [0, 1].map(|col| {
		Memo::new(move |_| {
			let q = query[col].get();
			let all: Vec<(usize, String)> = match (pool, col) {
				(Pool::Study, 0) => state.trades.get().into_iter().chain([NO_TRADE.to_owned()]).enumerate().collect(),
				(Pool::Study, _) => state.locations.get().into_iter().enumerate().collect(),
				(Pool::Open, _) => state.tabs.get().into_iter().enumerate().map(|(i, t)| (i, t.label)).collect(),
			};
			all.into_iter().filter(|(_, s)| subsequence(&q, s)).collect()
		})
	});
	// a box whose list is empty has no row under the cursor, and picking nothing is not a pick
	let choose = move |col: usize| {
		if let Some(hit) = hits[col].with_untracked(|h| h.get(sel[col].get_untracked()).cloned()) {
			pick[col].set(Some(hit));
		}
	};
	// half a pairing opens nothing — the button says so by being dead until both boxes hold a row
	let ready = move || (0..cols).all(|col| pick[col].with(Option::is_some));
	let take = move || match pool {
		Pool::Study =>
			if let (Some((i, trade)), Some((_, location))) = (pick[0].get_untracked(), pick[1].get_untracked()) {
				state.picker.set(None);
				// the row past the served trades is the one the server has no file for
				crate::map::open(state, (i < state.trades.with_untracked(Vec::len)).then_some(trade), location);
			},
		Pool::Open =>
			if let Some((i, _)) = pick[0].get_untracked() {
				state.picker.set(None);
				crate::map::activate(state, i);
			},
	};

	// the overlay is modal, so the keys below are only ever the picker's
	Effect::new(move |_| {
		if let Some(el) = field[side.get()].get() {
			el.focus().expect("the picker's field takes focus");
		}
	});
	// arrowing past the visible rows would otherwise leave the cursor off screen
	Effect::new(move |_| crate::map::scroll_pick(sel[side.get()].get()));

	view! {
		<div id="picker" class="panel" class:two=cols == 2>
			{(0..cols)
				.map(|col| {
					view! {
						<div class="col" class:on=move || side.get() == col>
							<input
								type="text"
								node_ref=field[col]
								placeholder=pool.prompt(col)
								prop:value=move || query[col].get()
								on:focus=move |_| side.set(col)
								on:input=move |ev| {
									query[col].set(event_target_value(&ev));
									sel[col].set(0);
								}
								on:keydown=move |ev| {
									let last = hits[col].with(|h| h.len()).saturating_sub(1);
									let down = ev.key() == "ArrowDown" || (ev.ctrl_key() && ev.key() == "n");
									let up = ev.key() == "ArrowUp" || (ev.ctrl_key() && ev.key() == "p");
									if down || up {
										ev.prevent_default();
										return sel[col]
											.set(
												if down {
													(sel[col].get_untracked() + 1).min(last)
												} else {
													sel[col].get_untracked().saturating_sub(1)
												},
											);
									}
									if ev.key() == "Enter" {
										ev.prevent_default();
										return match ev.ctrl_key() {
											true => take(),
											false => choose(col),
										};
									}
									if ev.ctrl_key() || ev.meta_key() || ev.alt_key() {
										return;
									}
									if ev.key() == "Tab" {
										ev.prevent_default();
										side.set((col + 1) % cols);
									}
								}
							/>
							<ul>
								{move || {
									hits[col]
										.get()
										.into_iter()
										.enumerate()
										.map(|(row, (id, name))| {
											view! {
												<li
													class:on=move || {
														pick[col].with(|p| p.as_ref().is_some_and(|(p, _)| *p == id))
													}
													class:cur=move || sel[col].get() == row
													on:mouseenter=move |_| sel[col].set(row)
													on:click=move |_| {
														side.set(col);
														sel[col].set(row);
														choose(col);
													}
												>
													{name}
												</li>
											}
										})
										.collect_view()
								}}
							</ul>
						</div>
					}
				})
				.collect_view()}
			<button class="go" title="Ctrl+Enter" disabled=move || !ready() on:click=move |_| take()>
				"↵"
			</button>
		</div>
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

/// Every character of `q`, in order, somewhere in `s`. Case-insensitive over ASCII, which is what a
/// file stem is.
fn subsequence(q: &str, s: &str) -> bool {
	let mut hay = s.chars().map(|c| c.to_ascii_lowercase());
	q.chars().all(|c| {
		let c = c.to_ascii_lowercase();
		hay.any(|h| h == c)
	})
}
