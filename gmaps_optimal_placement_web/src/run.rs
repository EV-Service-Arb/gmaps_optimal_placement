//! A study run: for the first time, or again over answers that aged. Who buys it is the CLI's to say;
//! here is what it says before, and the confirming.
use ev_lib::uikit_leptos::{AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle};
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

use crate::map::State;

/// Whoever buys the searches a study lacks.
#[cfg(feature = "ssr")]
pub trait Buyer: Send + Sync {
	fn quote(&self, def: &gmaps_optimal_placement_core::Definition) -> eyre::Result<Quote>;
	/// Every search `def` is built from, asked again; what came of it is held once this returns.
	fn run(&self, def: &gmaps_optimal_placement_core::Definition) -> eyre::Result<()>;
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
						))>"Buy"</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</div>
	}
}
