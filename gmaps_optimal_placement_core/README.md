Grid geometry, reprojection and the expression language. No network, no filesystem.

- [`Reproject`] — EPSG:3035 <-> WGS84. Reference points are pinned in its tests; nothing downstream
  is trustworthy without them.
- [`CellId`] — `CRS3035RES{res}mN{north}E{east}`, one parser across resolutions, and the cell ring
  it implies.
- [`Grid`] — cells plus `IndexMap<String, Vec<f64>>` of whatever the source published. Sources fill
  it through [`Grid::push`], which fixes the column set from the first row and rejects any row that
  disagrees.
- [`Expr`] — the arithmetic a study writes in TOML, over those column names. The callable surface is
  `max`, `min`, `clamp`, `sqrt`, `pow` and nothing else.
- [`rank`] — five features off a business and a query, one Plackett–Luce likelihood over observed
  orderings, and [`rank::COEF`], which `gmaps_optimal_placement fit` regenerates. [`rank::Feats`] extracts,
  [`rank::Rank`] scores, and only the second one checks the coefficients: the fit has to be able to
  read features before it has any. [`rank::nodes`] is the probe's sampler, and takes demand and
  geometry only — the competitors are the regressor, and it may not see them. No optimiser lives
  here: `gmaps_optimal_placement_rank` holds that, and this crate is wasm-safe because it does not.
