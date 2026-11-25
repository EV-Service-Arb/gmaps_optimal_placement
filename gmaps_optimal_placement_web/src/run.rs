//! A study run: for the first time, or again over answers that aged. Who buys it is the CLI's to say;
//! here is what it says before, the confirming, and the panel's pull of the study shown.
use std::time::Duration;

use ev_lib::uikit_leptos::{AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle};
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::map::State;

const HOUR: u64 = 3_600;
const DAY: u64 = 24 * HOUR;
/// The slider's, each about twice the one before it: as typed, and its seconds.
const STOPS: [(&str, u64); 15] = [
	("1h", HOUR),
	("2h", 2 * HOUR),
	("4h", 4 * HOUR),
	("8h", 8 * HOUR),
	("16h", 16 * HOUR),
	("1d", DAY),
	("2d", 2 * DAY),
	("4d", 4 * DAY),
	("1w", 7 * DAY),
	("2w", 14 * DAY),
	("1M", 30 * DAY),
	("2M", 60 * DAY),
	("4M", 120 * DAY),
	("8M", 240 * DAY),
	("1y", 365 * DAY),
];

/// Whoever buys the searches a study lacks.
#[cfg(feature = "ssr")]
pub trait Buyer: Send + Sync {
	fn quote(&self, def: &gmaps_optimal_placement_core::Definition) -> eyre::Result<Quote>;
	/// What `def` lacks, bought, and `again` of what is held; what came of it is held once this returns.
	fn run(&self, def: &gmaps_optimal_placement_core::Definition, again: Again) -> eyre::Result<()>;
}

/// Which of the answers held of a study a run asks again.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Again {
	Every,
	/// Every one at least this old.
	Older(Duration),
	Nothing,
}

/// What a run buys, in tokens.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct Quote {
	/// At least: whether another page is behind one is known only once it is answered.
	pub searches: usize,
	/// Of one search.
	pub price: f64,
	pub balance: f64,
	/// Of the answers held, which the run is in place of. `None` for a study nothing is held of.
	pub oldest_days: Option<f64>,
}

/// `text` as a `v_utils::Timeframe`, which the page does not link.
#[server]
pub async fn timeframe(text: String) -> Result<Duration, ServerFnError> {
	let t: v_utils::Timeframe = text.parse().map_err(|e| ServerFnError::new(format!("{e}")))?;
	match t.duration() {
		Duration::ZERO => Err(ServerFnError::new("no time at all")),
		d => Ok(d),
	}
}

#[component]
pub fn Confirm(state: State, at: (Option<String>, String), quote: Quote) -> impl IntoView {
	let Quote {
		searches,
		price,
		balance,
		oldest_days,
	} = quote;
	let title = StoredValue::new(match &at.0 {
		Some(c) => format!("Run {c} in {}?", at.1),
		None => format!("Run {}?", at.1),
	});
	let held = match oldest_days {
		Some(d) => format!("What is held of it, the oldest {d:.0} d, is asked again"),
		None => "Nothing is held of it".to_owned(),
	};
	let what = StoredValue::new(format!(
		"{held}: at least {searches} searches, {:.0} tokens of the {balance:.0} you hold. A page behind any of them is bought too.",
		searches as f64 * price
	));
	let at = StoredValue::new(at);
	view! {
		<div id="confirm">
			<AlertDialog
				open=true
				on_open_change=Callback::new(move |open: bool| {
					if !open {
						state.confirm.set(None);
					}
				})
			>
				<AlertDialogContent
					class="text-ink"
					aria_labelledby="confirm-title"
					aria_describedby="confirm-what"
				>
					<AlertDialogHeader>
						<AlertDialogTitle id="confirm-title">{title.get_value()}</AlertDialogTitle>
						<AlertDialogDescription id="confirm-what">{what.get_value()}</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>"Cancel"</AlertDialogCancel>
						<AlertDialogAction on_click=Callback::new(move |_| crate::map::run(
							state,
							at.get_value(),
							Again::Every,
						))>"Buy"</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</div>
	}
}

/// What the study shown lacks, pulled; what of it is old, refreshed.
#[component]
pub fn Pull(state: State) -> impl IntoView {
	let read = LocalResource::new(move || {
		let text = state.older.get();
		async move { (text.clone(), timeframe(text).await.map_err(|e| e.to_string())) }
	});
	// of the text as it stands, never of one it replaced
	let older = Memo::new(move |_| read.get().filter(|(of, _)| state.older.with(|t| t == of)).map(|(_, d)| d));
	let old = move || older.get().and_then(Result::ok); // the input says what is wrong
	let spendable = move || state.certain.get() && !state.running.get();
	let pull = move |again| {
		let at = crate::map::at(state).expect("shown under a study");
		crate::map::run(state, at, again)
	};
	move || {
		let held = state.summary.get()?.trade.is_some();
		crate::map::at(state)?.0?; // the grid alone is nobody's to buy
		let lacking = state.model.with_value(|m| m.as_ref()?.payload.notice.as_ref()?.missing);
		Some(view! {
			<div id="pull">
				{lacking
					.map(|n| {
						view! {
							<button
								class="pull"
								prop:disabled=move || !spendable()
								on:click=move |_| pull(Again::Nothing)
							>
								{format!("Pull data ({n}+ searches)")}
							</button>
						}
					})}
				{held
					.then(|| {
						view! {
							<div class="row older">
								<label for="older">"Refresh older than"</label>
								<input
									type="text"
									id="older"
									title="as 12h, 3d, 2w, 6M, 1y"
									aria-invalid=move || older.with(|o| matches!(o, Some(Err(_)))).to_string()
									prop:value=move || state.older.get()
									on:input=move |ev| state.older.set(event_target_value(&ev))
								/>
							</div>
							<input
								type="range"
								aria-label="how far back is old"
								aria-valuetext=move || state.older.get()
								min="0"
								max=STOPS.len() - 1
								prop:value=move || {
									old()
										.map(|d| {
											STOPS
												.partition_point(|(_, s)| *s < d.as_secs())
												.min(STOPS.len() - 1)
												.to_string()
										})
								}
								on:input=move |ev| {
									let stop: usize = event_target_value(&ev)
										.parse()
										.expect("a range's value is one of its steps");
									state.older.set(STOPS[stop].0.to_owned());
								}
							/>
							<button
								class="refresh"
								prop:disabled=move || !spendable() || old().is_none()
								on:click=move |_| pull(Again::Older(old().expect("disabled without one")))
							>
								{match lacking {
									Some(n) => format!("Refresh data, and pull the {n}+ lacking"),
									None => "Refresh data".to_owned(),
								}}
							</button>
							{move || {
								older.get().and_then(Result::err).map(|e| view! { <div class="note">{e}</div> })
							}}
						}
					})}
				{move || {
					(!state.certain.get())
						.then(|| {
							view! {
								<div class="note">
									"Spends tokens: served without --spend-real-quota-i-am-certain."
								</div>
							}
						})
				}}
			</div>
		})
	}
}
