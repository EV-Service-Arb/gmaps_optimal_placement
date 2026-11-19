Whole-country reconnaissance, one map. Not a study: a study asks where to open inside one
agglomeration, this asks which towns in a country are worth writing a study for at all.

[`Tally`] is what to count — something an official cadastre already draws, per administrative unit —
and [`Country`] is who publishes it. Both are closed enums, matched on, the same containment
`GridSource` has.

- [`compile`] rolls the country's statistical grid up by administrative unit
  ([`gmaps_optimal_placement_sources::grid::places`]), drops every unit under the floor, and only then
  counts the tally in the ones that are left. The floor is the cheap half of the design: with no
  storefront worth having under 2 000 inhabitants, four fifths of the communes in France are never
  fetched at all.
- The denominator is any column the grid publishes, named on the command line. `ind` reads the map per
  person, `men` per household, `men_mais` per household living in a house.
- A unit the cadastre has no vector layer for carries `tally: None`. That is not zero, it is not
  plotted, and it is counted separately in [`Compiled::stats`].
- [`Compiled::render`] writes one self-contained HTML map: colour is the ratio, area is the
  denominator, and the ramp tops out at the 98th percentile because one Riviera flattens a country.

```fish
nix run .#misc france pool --per ind --floor 2000
```

The pool count is the one that has been checked against something: `SYM = 65` in the PCI surface
layer has a median area of 31 m², and per commune it agrees with an independently published count of
French pools to within half a percent. `Building` and `Parcel` are the publisher's own layer names and
are counted, not verified — and their files are ~1 MB per commune against a pool layer's ~10 kB.

A rate over 1000 per 1000 is not an error. The numerator counts every pool and the denominator counts
resident households, and in a Riviera or Corsican commune most of the housing belongs to people
counted as households somewhere else. It ranks the stock, not the share of locals who own one.

What it cannot see: in-ground pools only, because an above-ground pool is not a built structure and
the cadastre has no fiscal reason to draw it. Observation dates run from 2007 to last year, so a
commune last flown over fifteen years ago is missing fifteen years of building.
