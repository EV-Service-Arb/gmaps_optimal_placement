Everything that reaches the network. Two closed enums, [`GridSource`] and [`PoiSource`]; adding a
country means adding a variant and a row to the table it matches on, not a trait implementation.
Search-volume providers are the exception and are a trait — see `docs/ARCHITECTURE.md`.

- [`grid::load`] returns a [`gmaps_optimal_placement_core::Grid`] whose columns are whatever the chosen archive
  publishes. The bbox filter reads the cell corner out of the id, so only surviving cells are ever
  reprojected.
- [`poi::load`] returns competitors already sorted into the study's tiers, and every [`Ranking`] the
  searches that found them handed back. Tiering is the study's, not ours: [`PoiConfig`] is
  deserialized straight from the study document.
- [`probe::plan`] is the request plan and is pure, so a dry run and a cache lookup both read it
  without a key; [`probe::run`] spends. A [`Ranking`] carries the [`Region`] it was asked from, which
  is what says who could have been returned — and, for a rectangle, that nobody was standing in it.
- [`searches::provider`] resolves a name to a [`searches::SearchVolume`]. A [`Keyword`] the provider
  has no data for carries `monthly: None`, which is not zero and must not be summed as one.
- [`Work`] is the untracked cache, and carries the policy the CLI resolved: how old an answer of each
  [`work::Kind`] may get before it is reported as old. Bulk archives download once, POST responses are
  keyed by the request, so editing a query refetches and rerunning does not. Anything computed out of
  those is kept under a key its own inputs decide — the grid extract is one. Nothing expires and
  nothing here refuses: [`Work::age`] reports how old an answer is, a sweep states what it is about
  to buy before buying it, and `--refresh` is the only thing that re-asks.

[`poi::load`] buys with the keys given to [`Work::keys`], in that order.
