#![feature(default_field_values)]
#![doc = include_str!("../README.md")]

pub mod expr;
pub mod grid;
pub mod model;
pub mod payload;
pub mod proj;
pub mod rank;

pub use expr::Expr;
pub use grid::{Cell, CellId, Grid};
pub use model::Model;
pub use payload::{Candidate, LayerOut, Notice, Payload, Poi, PoiOut, Scale, TermOut, TierOut, Trade};
pub use proj::Reproject;
pub use rank::{Feats, Rank};

/// A pull refused for needing more Places searches than the host's budget for it. Here rather than
/// beside the purchase, because the server that answers the host with it never sees a purchase.
#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Unaffordable {
	/// At least this many: a tile at the cap opens four more.
	pub need: usize,
	pub budget: usize,
}
impl std::fmt::Display for Unaffordable {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "this pull needs at least {} Places searches, and may buy {}", self.need, self.budget)
	}
}
impl std::error::Error for Unaffordable {}
