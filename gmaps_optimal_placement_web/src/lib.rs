#![feature(default_field_values)]
#![recursion_limit = "256"] // nested tachys view types blow the default 128 on the control panel
#![doc = include_str!("../README.md")]

pub mod map;
pub mod pins;
#[cfg(feature = "ssr")]
pub mod serve;
pub mod tabs;

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
	console_error_panic_hook::set_once();
	leptos::mount::hydrate_islands();
}
