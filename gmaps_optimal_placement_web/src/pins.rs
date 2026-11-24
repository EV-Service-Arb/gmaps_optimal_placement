//! Pins — a click on the map, or a pick of the top-ten sweep — and the card that is their control
//! surface. A pin lasts as long as its tab.
use gmaps_optimal_placement_core::model::Report;
pub use imp::{copy, forget};
use leptos::prelude::*;

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
							let at = p.at;
							view! {
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
									on:click=move |_| forget(state, p.id.clone())
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

#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
	/// Stable for the pin's life, which is what the card and the marker click agree on.
	pub id: String,
	/// [lat, lon]
	pub at: [f64; 2],
	/// The sweep's rank, or nothing for a click.
	pub label: Option<String>,
}

#[cfg(not(feature = "hydrate"))]
mod imp {
	use super::State;

	pub fn copy(_: State, _: [f64; 2]) {}
	pub fn forget(_: State, _: String) {}
}

#[cfg(feature = "hydrate")]
mod imp {
	use leptos::prelude::*;
	use wasm_bindgen::prelude::*;

	use super::State;

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

	pub fn forget(state: State, id: String) {
		state.pins.update(|v| v.retain(|p| p.id != id));
		state.selected.set(None);
		state.report.set(None);
	}
}
