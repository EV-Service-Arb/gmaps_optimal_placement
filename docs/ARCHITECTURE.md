# gmaps_optimal_placement

Where should a service business open. A location describes an area and the statistical grid behind
it; a trade, applied to that location, describes what counts as a competitor and how demand follows
from whatever columns that grid publishes; the tool serves a map that answers it.

The question is the input. Retargeting a city, a country or a trade is an edit to one of the two
documents, never to the code.

```
        trade.nix (location.nix) ─── the pinned interface
                   │
   ┌───────────────┴───────────────────────────────────────────┐
   │ gmaps_optimal_placement_sources        network, cache     │
   │   GridSource ─ INSEE Filosofi 200 m                       │
   │               GEOSTAT 1 km                                │
   │   PoiSource  ─ Google Places                              │
   │   probe      ─ the same, asked from a node                │
   │   SearchVolume ─ Google Ads · DataForSEO                  │
   └──────┬────────────────────────────────────┬───────────────┘
          │                                    │  the grid rolled up by commune
          │              ┌─────────────────────┴─────────────────────┐
          │              │ gmaps_optimal_placement_misc              │
          │              │   Country · Tally — what a cadastre draws,│
          │              │   per commune, ÷ any column, one map      │
          │              └───────────────────────────────────────────┘
          │  cells + named columns, POIs + tiers, orderings, keyword series
   ┌──────┴────────────────────────────────────────────────────┐
   │ gmaps_optimal_placement_core     no I/O, wasm-safe        │
   │   Reproject · CellId · Grid · Expr                        │
   │   rank — features, Plackett–Luce, COEF                    │
   │   Payload — the whole thin waist                          │
   │   model — pressure, unmet, capture, top-N, clouds         │
   └───────┬───────────────────────────────────────┬───────────┘
           │  every expression                     │  every slider
   ┌───────┴──────────────────────────┐   ┌────────┴──────────────────────┐
   │ gmaps_optimal_placement          │   │ gmaps_optimal_placement_web   │
   │   CLI, study, settings, HTML     │──▶│   ssr: axum + server fns      │
   │   trades × locations             │   │   hydrate: the MapView island │
   └───────┬──────────────────────────┘   │   map_core.js: google.maps    │
           │  the orderings on disk       └────────┬──────────────────────┘
   ┌───────┴──────────────────────────┐            ▼
   │ gmaps_optimal_placement_rank     │   a served map · one <name>-searches.html
   │   Observed · Strategy · Strength │
   │   Adam, k-fold, the league table │
   └──────────────────────────────────┘
```

`serve` holds the two axes rather than a study, and the page picks a point on their product — one
field per axis, both live at once, because a coordinate is known in either order. The server builds
a pairing the first time a tab asks for it, and the page keeps several open over one map instance.
Naming both axes as files builds one before the listener binds, which is what keeps a scripted run
off the picker.

The trade axis carries one extra point: none. A trade file is a function of a location, so no trade
is that function's identity — the location's own attrset is already a study. `Payload::trade` is the
one `Option` that says so, and everything behind it is everything that is billed. A city can
therefore be looked at before any of the Places quota goes on it.

The axes are `--trades` and `--locations` rather than two positionals: they are the same shape, so
nothing about a bare pair of paths says which is which.

What a pairing is *called* — `<trade>_-_<location>`, from the two stems — is the CLI's to say. The
web crate keys on the stems and the pin file keys on the name, so neither has a second spelling of
it to keep in step.

The map answers where the people are. It does not answer how many are looking for the thing, which
is what `searches` is for: a cell can be dense, affluent and uncontested and still sit under a trade
nobody searches for.

## The line between baked and live

Rust evaluates everything that a slider cannot move — the demand model and every configured layer —
into a `Payload` the browser fetches. Competitor pressure, underserved demand, the capture score and
the top-N sweep are recomputed in the page, because they are functions of λ and the tier weights,
which are live controls. Moving either side across this line costs the map its interactivity or the
study its reproducibility.

Competitor weight is baked, and deliberately: `rank` scores every competitor once in `build`, before
the payload is serialised, so the browser only re-weights by tier and λ. What the probe observed of
each competitor, node by node, is baked the same way. The one thing scored live is the what-if — one
business, one arithmetic pass — so the scoring function reaches wasm only through that, and only a
model that can score one hypothetical business has to.

Where a fitted model *comes from* never reaches wasm at all. `gmaps_optimal_placement_rank` holds
the optimiser, the entrants and the cross-validation, and is linked by the CLI alone: an entrant may
carry whatever it needs without that landing in the browser or beside the HTTP server.

Both sides are Rust. `gmaps_optimal_placement_core` is wasm-safe and holds the model, so the same code that
`cargo t` pins against a fixture is the code the browser runs.

## The line between Rust and JavaScript

`map_core.js` rides in as a wasm-bindgen snippet and is the only file that names `google.maps`: the
map instance, the markers and their info windows, and the canvas fill loop — which stays there
because it needs `fromLatLngToDivPixel` every frame. Rust hands it `ringX`/`ringY`, `colors`,
`shown` and an opacity, and owns every one of those numbers. The pattern is `v_utils::lwc`'s.

Nothing throws across that boundary. Under `panic=abort` a rejected promise reaching wasm kills the
app, so every entry point in `map_core.js` returns a banner string instead.

## Invariants

- **Reprojection is exact or the build fails.** A silently shifted grid is the one bug that looks
  fine and is entirely wrong. Reference points are pinned in `gmaps_optimal_placement_core::proj`.
- **No fallbacks on missing or malformed data.** A cell that will not parse is an error, never a
  zero. Imputed-versus-observed provenance survives to the map.
- **Config defines the model; code defines the mechanism.** Anything a study would want to vary
  belongs in the study file. Anything two studies share belongs in Rust. How Google ranks is the
  same mechanism for a plumber and a detailer, so the reviews→prominence curve is a fitted constant
  in `core::rank`, not an expression a study writes; what the study says is which queries it cares
  about and what each is worth.
- **A trade may not name a city, and a location may not name a trade.** They are the two axes of a
  product, and a quantity that needs both is a quantity neither document can state honestly: it
  belongs in Rust, derived, or in the tool's own settings. The probe's bias radius and the POI
  sweep's tiling (`sweep.root`, `sweep.depth`) are there for exactly this reason. The alternative is the same knob written out per pairing, drifting apart silently.
- **Demand is a trade's word.** `model.demand` is what a trade says the grid means to it. Without
  one there is no demand surface, and nothing stands in for it: the layer is absent and the controls
  that read λ, a tier weight or a demand value are disabled. Population is not demand.
- **A fitted quantity is refitted, never hand-edited.** `rank::COEF` is the output of
  `gmaps_optimal_placement fit` over the orderings in the work dir. Nudging a coefficient because the map looks
  wrong turns a measurement back into the guess it replaced.
- **A model earns the map out of sample.** `gmaps_optimal_placement strength` cross-validates every
  entrant over the same orderings, held out by the region each was asked from, and refuses the table
  if counting every competitor the same predicts Google as well. An in-sample likelihood cannot tell
  a fitted weight from a memorised one, and a memorised one paints a map that looks decided.
- **The study file is a seed, never a sink.** `serve` only reads it. Pins promoted or hidden on the
  map are a diff beside it, under `XDG_DATA_HOME` — data, not cache, because a promoted candidate is
  a decision and cache is what cleaners delete.
- **`GOOGLE_MAPS_KEY` lives in the server's environment, never in an artifact.** Bulk archives and
  API responses cache under `GMAPS_OPTIMAL_PLACEMENT_WORK` (default `./tmp/geo`) so a rerun costs nothing — the
  INSEE archive is ~87 MB and Places calls are billed.
- **Nothing in that cache expires.** An answer older than its kind's configured age is served anyway,
  and its age rides onto the map beside the imputed flag. Google's daily search quota is the scarce
  side, and refetching spends a day of it to learn what is mostly the same thing; `--refresh` is the
  only thing that re-asks. Age is a caption, never a trigger.
- **A cache key covers everything its answer depends on**, so a changed input is a different file
  rather than a stale one and nothing needs invalidating. A derived answer is cached only where it is
  reproducible from what is kept: the grid extract keeps the archive's own rows and re-parses them,
  because caching the floats would need a round-trip no text format gives.
- **What a run is about to spend is stated, never guessed at.** No API key can read what is left of
  `SearchTextRequestPerDayPerProject`, so a local tally could only ever refuse work Google would have
  served. What the work dir is missing is counted and announced before anything is bought; what
  remains is Google's to say, in the 429.

## Sources are an enum

The set of statistical sources is closed and known at compile time, so `GridSource` and `PoiSource`
are enums matched on, not traits implemented. Adding a country adds a variant and a row to the table
it matches on.

What the sources disagree about is which columns exist, so a grid carries columns discovered from
its archive rather than a struct per country. Expressions name those columns, and an expression
naming one that does not exist fails against the source's own column list before a single POI call
is made.

## Search-volume providers are a trait

The set of providers is not closed: they differ on price, geography and honesty, and a new one is a
purchase, not a country. So `searches::SearchVolume` is a trait, one file per implementation, and
`searches::provider` is the one `match` that knows the set — the same containment the enums give,
without pretending the set is compile-time known.

They agree on the string that names a place, because DataForSEO's `location_name` format *is*
Google's `canonicalName`. Switching provider is a one-word study edit.

Grouping is by essence, not by string: a study names seeds, the provider expands them semantically,
a regex filters that expansion and the survivors sum into one line. The expansion is fuzzy, so every
member and its own series survives into the page — a sum you cannot audit is a sum you cannot argue
with. A keyword the provider has no data for is excluded from the sum, never zero-filled.

## What a map like this cannot know

These are not caveats about the implementation. They are the distance between the model and the
decision, and they are the difference between a map that informs one and a map that launders a
guess into authority. The worked examples are in `examples/`.

- **Motorisation is estimated, not measured.** No 200 m source publishes households-with-a-car;
  INSEE has it only at IRIS level. The detailing trade infers it from the house/flat split
  (`men_mais × 1.55 + men_coll × 0.85`). Directionally right, not a measurement.
- **Most fine-grained cells are modelled.** ~80 % of INSEE 200 m cells have fewer than 11 fiscal
  households, so their values are imputed. The flag is carried through to the map and the "hide
  imputed cells" control exists for it. Trust dense cells, distrust isolated rural ones.
- **Residential demand only.** No traffic counts, no workplace population, no rent or premises
  availability. For a walk-in bay, passing traffic matters as much as who lives nearby.
- **A competitor sits at its registered address**, which understates a mobile operator's reach.
- **Tiering is name-based.** A shop whose name says nothing about what it does is tiered on what its
  name does say.
- **A Places *type* is not the category the map shows.** `primaryTypeDisplayName` is the Business
  Profile category, of which there are thousands; `poi.included_type` takes a Places type, of which
  Table A has 478, and there is no cleaning, detailing or handyman type among them. So the filter is
  off by default, and what it costs when it is on was measured on the Clermont cache: `car_wash`
  keeps 35 % of the tier-1 detailers, because 14 of 40 of them carry no primary type at all;
  `service` keeps 92 % of the cleaners but filters only 23 % of the rows, so it saves almost nothing.
  `plumber` is the case where the two line up. Turning it on is a claim that Google types this trade,
  and the category list in the map panel is where that claim is checked.
- **Every constant in a study is a guess with a sane magnitude**, not a fitted value. They are in
  the study file so they can be argued with. Competitor weight is the exception: it comes out of
  `rank`, and the three things below are what that estimate cannot settle.
- **Reviews cause rank and rank causes reviews.** A shop ranks well and is therefore seen, clicked
  and reviewed. The coefficient is co-movement, not causation, so the what-if reads "reviews
  associated with that rank" and never "reviews needed".
- **The Places API ordering is not the local pack** a customer sees in Maps. It correlates with it;
  it is a different list, from a different endpoint, with no personalisation and no map viewport. The
  coverage layers draw that ordering — modelled as a share, or as the probe observed it — so a blue
  field is a claim about the API's list, not about what a customer is shown.
- **Name relevance is observed after Google's own filter.** These results came back *because* they
  matched the query, so the variation among them is compressed and the name coefficient rests on the
  businesses that sat at the censoring boundary.
- **City-level search volumes are bucketed and small.** Keyword Planner rounds hard (0, 10, 20, 30,
  50, 70, 90, 110…) and an account with no campaign spend gets the coarsest treatment. For a niche
  trade in a 150k city, expect a line in the tens that moves in steps. The shape of the year is
  trustworthy; the level is an order of magnitude, not a count.
- **Summing an expansion overstates, and counts events not people.** Google already merges close
  variants — plurals, accents, misspellings — into one keyword, so members are distinct queries and
  the sum is defensible. But two phrasings of one intent still add, and a person who searches twice
  is two searches.
- **Search volume is not local demand.** It excludes everyone who finds a detailer through Maps
  without a query, through a friend, or by driving past.
