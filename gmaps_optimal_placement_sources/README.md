Everything that reaches the network. [`GridSource`] is a closed enum; adding a country means adding a
variant and a row to the table it matches on, not a trait implementation.

- [`grid::load`] returns a [`gmaps_optimal_placement_core::Grid`] whose columns are whatever the chosen archive
  publishes. The bbox filter reads the cell corner out of the id, so only surviving cells are ever
  reprojected.
- [`poi::cached`] returns a category's competitors, each in the tier of the first search that found
  it ([`poi::queries`]), every [`Ranking`] those searches handed back, and the searches it lacks
  (`asks`), out of what the conductor's queries cache holds: nothing here buys a Places search.
- [`probe::plan`] is the request plan and is pure, so a cache lookup reads it without a key;
  [`probe::cached`] reads what of it was ever answered. A [`Ranking`] carries the [`Region`] it was asked from, which
  is what says who could have been returned — and, for a rectangle, that nobody was standing in it.
- [`Work`] is the untracked cache, and carries the policy the CLI resolved: how old an answer of each
  [`work::Kind`] may get before it is reported as old. Bulk archives download once, Places answers
  are keyed by the request, so editing a query asks anew and rerunning does not. Anything computed
  out of those is kept under a key its own inputs decide — the grid extract is one. Nothing expires
  and nothing here refuses: [`Work::age`] reports how old an answer is, and a pull is the only thing
  that re-asks.
