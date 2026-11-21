Everything that reaches the network, and the work dir. [`GridSource`] is a closed enum; adding a country means adding a
variant and a row to the table it matches on, not a trait implementation.

- [`grid::load`] returns a [`gmaps_optimal_placement_core::Grid`] whose columns are whatever the chosen archive
  publishes. The bbox filter reads the cell corner out of the id, so only surviving cells are ever
  reprojected.
- [`poi::cached`] returns a category's competitors, each in the tier of the first search that found
  it ([`poi::queries`]), every [`Ranking`] those searches handed back, and the searches it lacks
  (`asks`), out of what the work dir's [`Held`] holds: nothing here fetches a Places search.
- [`probe::plan`] is the request plan and is pure, so a cache lookup reads it without a key;
  [`probe::cached`] reads what of it was ever answered. A [`Ranking`] carries the [`Region`] it was asked from, which
  is what says who could have been returned — and, for a rectangle, that nobody was standing in it.
- [`Work`] is the work dir, the host's [`Held`], `since`, and the policy: how old an answer of
  each [`work::Kind`] may get before it is reported as old, and how the sweep tiles. Bulk archives
  download once; anything computed out of them is kept under a key its own inputs decide — the grid
  extract is one. Nothing expires and nothing here refuses: [`Work::age`] reports how old an answer
  is, and a pull is the only thing that asks again.
