CLI and the study document.

Every command takes the same two paths, each a directory or a single `*.nix`. A directory is the
whole axis: the page picks out of it, `fzf` picks out of it, `fit` takes all of it.

```sh
gmaps_optimal_placement serve    <trades> <locations> [--port 8731] [--open]
gmaps_optimal_placement searches <trades> <locations> [-o searches.html]
gmaps_optimal_placement probe    <trades> <locations> [--dry-run]  # ask Google, from equal-demand nodes
gmaps_optimal_placement fit      <trades> <locations>              # refit rank::COEF over every ordering cached
gmaps_optimal_placement schema                    # JSON schema for the study document
```

The study is [`Study`]: a trade applied to a location, evaluating to an area, a grid source, what a
competitor is, and how demand follows from the columns that source publishes. [`load`] is where the
two meet, and where the pairing is named `<trade>_-_<location>`. `column` adds derived columns in
list order, so a study can name a quantity its source only implies. Every expression is evaluated
before any of it reaches the map — an expression naming a column that does not exist is an error,
not a zero.

`load` takes no trade as well, and then a location is already a study: `poi`, `model` and `rank` are
`None`, [`Study::build`] never reaches `poi::load`, and what comes back is the grid's own layers
under the city's own name. `serve` offers it as the last row of the trade field.

An optional `searches` block names groups of queries. [`fold`] is the half of that command that is
ours: the provider expands seeds semantically, a regex keeps or drops each member, and the survivors
sum into one line of [`SearchPayload`] — with their own series alongside, so the sum can be audited.

The `rank` block says which queries the study is about and what each is worth. `probe` asks them from
the demand strata, `fit` estimates `gmaps_optimal_placement_core::rank::COEF` over every ordering in the work dir
— the whole product at once, because how Google ranks is one mechanism and one pairing on its own is
about fifty orderings. A pairing nobody harvested contributes nothing and costs nothing: `fit` reads
the work dir with no key, so it cannot buy what it is missing. [`fit::Fit::check`] is what refuses a
fit whose implied catchment nothing observed.

[`Payload`] is what `serve` hands the browser. It carries evaluated per-cell values only: competitor
pressure, underserved demand, capture scores and the top-N sweep are `gmaps_optimal_placement_core::model`,
recomputed in the page because they move with the sliders.

The Google Maps keys are named by the config's `google_maps`, read from the environment and never written to disk.
Archives and API responses cache under `$XDG_CACHE_HOME/gmaps_optimal_placement`.
