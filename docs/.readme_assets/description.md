Where should a service business open?

Two Nix files describe it. A location gives an area and the statistical grid behind it; a trade —
applied to that location — gives what counts as a competitor and how demand follows from the columns
the grid publishes. `gmaps_optimal_placement` paints that demand under the competitors already on the
ground and serves the map. How much any one competitor counts is fitted, not guessed:
`gmaps_optimal_placement fit` estimates what Google's ordering rewards, over the orderings the
conductor's queries cache already holds. `gmaps_optimal_placement
strength` is what says that estimate is worth painting: every candidate model cross-validated over
the same orderings, and a refusal if counting every competitor the same does as well.

A trade also names groups of queries, and `gmaps_optimal_placement searches` charts how often each group is
asked for over the trailing year — the map says where the people are, this says how many of them
are looking for the thing.

The question is the input: another city is a location file, another trade is a trade file, and every
pairing of the two is a study. Neither is an edit to the code. A location on its own is a study too
— the grid, and nothing a trade decides — and it is the one the Google Maps quota is not spent on.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the invariants, and
[examples/](examples) for worked studies.
