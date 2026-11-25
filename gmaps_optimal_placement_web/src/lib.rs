#![doc = include_str!("../README.md")]
#![recursion_limit = "256"] // nested tachys view types blow the default 128 on the control panel

mod map;
mod pins;
mod run;
#[cfg(feature = "ssr")]
mod serve;
mod tabs;

#[cfg(feature = "ssr")]
pub use run::{Buyer, Quote};
#[cfg(feature = "ssr")]
pub use serve::{Works, serve};

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
	console_error_panic_hook::set_once();
	leptos::mount::hydrate_islands();
}
