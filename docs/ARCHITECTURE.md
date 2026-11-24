# gmaps_optimal_placement

Where should a service business open. An area and the country base under it describe the ground
and the statistical grid behind it; the conductor's category — one setting of a trade, which is one
of Google's categories, in a country — describes everything else: the Google categories and
keywords its competitors are found by, what
each keyword is worth, what is never a competitor, and how demand follows from whatever columns the
grid publishes. The library builds the payload that answers it; the conductor links it and draws it
on its map, and the GUI here draws the same studies through the same library and overlay.

The question is the input. Retargeting an area, a country or a category is an edit to one of those
documents, never to the code.

```
  the conductor's category · an area · bases/<CC>.toml ─── a Definition, addressed by its SHA-256
                   │
   ┌───────────────┴───────────────────────────────────────────┐
   │ gmaps_optimal_placement_sources        network, work dir  │
   │   GridSource ─ INSEE Filosofi 200 m · GEOSTAT 1 km        │
   │   poi, probe ─ the sweep and the probe, out of Held       │
   │   Work { dir, since, Held }                               │
   └──────┬────────────────────────────────────┬───────────────┘
          │                                    │  the grid rolled up by commune
          │                    ┌───────────────┴───────────────────────────┐
          │                    │ gmaps_optimal_placement_misc              │
          │                    │   what a cadastre draws, per commune      │
          │                    └───────────────────────────────────────────┘
   ┌──────┴────────────────────────────────────────────────────┐
   │ gmaps_optimal_placement_core     no I/O, wasm-safe        │
   │   Definition · Category · Ask · Country — the vocabulary  │
   │   Reproject · CellId · Grid · Expr                        │
   │   rank — features, Plackett–Luce, COEF, divisors          │
   │   Payload — the whole thin waist                          │
   │   model — Selection, unmet, capture, share, top-N         │
   └───────┬─────────────────────────────────┬─────────────────┘
           │                                 │
   ┌───────┴────────────────────────┐  ┌─────┴──────────────────────────┐
   │ gmaps_optimal_placement        │  │ gmaps_optimal_placement_overlay│
   │   load · asks · build          │  │   Overlay · Scene · paint.js   │
   │   pressure, at build           │  │   (wasm)                       │
   │   bases compiled in            │  │                                │
   └───────┬──────────────┬─────────┘  └─────┬───────────────┬──────────┘
           │              └─────────┬────────┘               │
           │                        ▼                        │
           │             site_conductor links both, one pin  │
           │   ┌──────────────────────────────────────────┐  │
           ├──▶│ gmaps_optimal_placement_web   the GUI    │◀─┘
           │   │   ssr: serve — the axes, one build a pair│
           │   │   hydrate: tabs · picker · pins · panel  │
           │   └──────────────────────┬───────────────────┘
   ┌───────┴──────────────────────────┴──────┐
   │ gmaps_optimal_placement_cli             │
   │   view · fit · strength · misc · schema │
   │   Held: a copy of prod's panel.db,      │
   │   its queries cache, read-only          │
   └───────┬─────────────────────────────────┘
   ┌───────┴──────────────────────────┐
   │ gmaps_optimal_placement_rank     │
   │   Observed · Strategy · Strength │
   │   Adam, k-fold, the league table │
   └──────────────────────────────────┘
```

A study is a `Definition` (`_core`): an area (a city's viewport or a circle, and a margin, as
the conductor keeps it) and a category, each named as the conductor names it. The category drives
the billed half. The sweep searches its trade's name in the country's language and each of its
keywords, then each additional Google category's name. A competitor's tier is the first of those
whose search found it — the trade's, then each additional in order — unless Excluded drops it
first. Its weighted keywords are what every competitor is scored on, each relative to the trade's
name. Its demand is
what the grid means to it. A definition is addressed by its SHA-256, because an area or a category
can be redefined under the same name.

The category axis carries one extra point: none. The area's own base is already a study, the
identity of the pairing. `Payload::trade` is the one `Option` that says so, and everything behind it
is everything that is billed. A city can therefore be looked at before any of the Places quota goes
on it.

The CLI's commands hold two axes, both read off the definitions under the work dir's `studies/`: a
category is each setting of a trade a host studied, told apart by `Category::id`, and a location is
each area a host studied, called by its slug, and by its id too where a slug was redefined. The CLI never reads the conductor's catalogue, and a
copy of either kept here would drift from it. What a study is *called* — `<trade>·<CC>·<slug>` — is the definition's.

## The line between baked and live

`build` evaluates a study into the `Payload` a page fetches: the demand model, every configured
layer, each competitor's weight, what the probe observed of it node by node, and every cell's
competitor pressure. The page derives the rest by arithmetic over it — underserved demand once per
model; one competitor's share of the cells, a site's capture score and the top-N sweep when asked —
at the payload's own λ and tier weights. What a control moves is the view: which layer is on, and
which marks are shown (one `Selection` over tiers, searches and kinds: the dots, the counts, the
ages).

Pressure is one private function of the library, over the grid and the weighted competitors. The
page reads `Trade::pressure`, and everything that needs pressure reads that one array, so a share of
a cell is of the pressure painted on it.

`rank` scores every competitor once in `build`. The scoring function reaches wasm for two passes of
arithmetic: `Model::try_new` rescores the competitors, which checks the payload against the linked
`rank::COEF` and splits each score by search for its mark, and the what-if scores one business that
does not exist yet.

Where a fitted model *comes from* never reaches wasm at all. `gmaps_optimal_placement_rank` holds
the optimiser, the entrants and the cross-validation, and is linked by the CLI alone: an entrant may
carry whatever it needs without that landing in the browser or in a host.

Both sides are Rust. `gmaps_optimal_placement_core` is wasm-safe and holds the model, so the same code that
`cargo t` pins against a fixture is the code the browser runs.

## The line between this repo and the conductor

The conductor depends on placement at build time only: it links the library (`load`, `asks`,
`build`), `_core` for the vocabulary its stored categories are written in, and the overlay, from one
pin. `Model::try_new` checks a payload's weights against the `rank::COEF` it was linked with, so
that pin is what makes builder and renderer one model. Nothing placement links depends on the
conductor; the CLI alone reads a copy of its queries cache (`sa_core::queries_cache`'s keys), and the
conductor never links the CLI. This repo keeps what training needs — the grids, the sources,
`probe`'s plan, `fit`, `strength` — and the GUI a fit is judged by; what it trains on is what
the conductor bought, never a search of its own.

## Two visual axes

The heat is one layer at a time on a warm sequential ramp. Competitor pressure is never one of
those layers: it is a field of its own — a blue haze rising with the pressure's percentile, and
iso-lines at its 50th, 75th and 90th — drawn over whatever heat is on, so where people are and where
competitors already reach read at once. Blue is the field's alone; no heat ramp and no category
colour comes near it.

Both scales are relative to the cells in view. A city zoomed into is read against itself, and the
legend's ticks move with the view.

## Held, and a pull

Placement fetches no billed answer. A Places search is an `Ask`, and what was answered for it is
whatever the host's `Held` returns, with when it was fetched: the conductor's reads its own queries
cache, from the blocking thread a build runs on; the CLI's reads a copy of prod's `panel.db`.
Public archives are placement's own, under the work dir.

A pull is its host's. It calls `asks` — the first unanswered page down each branch of the sweep —
buys those, and asks again, since what lies behind a page (the next one, a tile's quarters) is known
only once it is answered; once none is lacking it calls `build`. Placement never counts a purchase.

`Work::since` is the pull's: an answer fetched before it is lacking. The host keeps when the first
pull since the study's last whole one began, and forgets it once one ends whole, so what a stopped
pull paid for is served to its retry and a refunded failure is never bought twice.

Every `asks` and `build` keeps the definition under `studies/<id>.json`, whichever host built it:
that is what `fit` harvests. Its name is its bytes' SHA-256, which tells a definition of another
format, whole and named as passed over, from a corrupt one, an error.

## Invariants

- **Reprojection is exact or the build fails.** A silently shifted grid is the one bug that looks
  fine and is entirely wrong. Reference points are pinned in `gmaps_optimal_placement_core::proj`.
- **No fallbacks on missing or malformed data.** A cell that will not parse is an error, never a
  zero. Imputed-versus-observed provenance survives to the map.
- **Config defines the model; code defines the mechanism.** Anything a study would want to vary
  belongs in its category or its base. Anything two studies share belongs in Rust. How Google ranks
  is the same mechanism for a plumber and a detailer, so the reviews→prominence curve and the
  catchment its distance term implies are fitted constants in `core::rank`, and the probe's sample
  is the tool's; what the category says is which queries it cares about and what each is worth.
- **A category may not name an area, and an area may not name a trade.** They are the two axes of a
  product, and a quantity that needs both is a quantity neither document can state honestly: it
  belongs in Rust, derived, or in the tool's own settings. The probe's bias radius and the POI
  sweep's tiling (`sweep.zoom`, `sweep.depth`) are there for exactly this reason. The alternative
  is the same knob written out per pairing, drifting apart silently.
- **Demand is the category's word.** Its demand is what it says the grid means to it. Without one
  there is no demand surface, and nothing stands in for it: the demand layers are absent and what
  reads a demand value — underserved demand, capture, the site sweep, the probe's strata — is not
  offered; the competitors and their pressure still are. Population is not demand.
- **Pressure is computed here, in the background, never in a page.** One private function of the
  library computes it, in `build`, on its host's blocking thread; nothing wasm links can. What
  computes it may be a model no page can afford to run, which is why placement is a library of its
  own and not a part of the conductor.
- **A fitted quantity is refitted, never hand-edited.** `rank::COEF` is the output of
  `gmaps_optimal_placement fit` over the orderings in the work dir, and `rank::LAMBDA_M` what it
  implies. Nudging a coefficient because the map looks wrong turns a measurement back into the
  guess it replaced.
- **A model earns the map out of sample.** `gmaps_optimal_placement strength` cross-validates every
  entrant over the same orderings, held out by the region each was asked from, and refuses the table
  if counting every competitor the same predicts Google as well. An in-sample likelihood cannot tell
  a fitted weight from a memorised one, and a memorised one paints a map that looks decided.
- **The conductor is the one center of data ingestion.** A Places search is never bought here, for
  a map or for training: the conductor buys what a pull lacks, and `fit` and `strength` read only
  what it already holds. A study placement cannot build out of that says so and opens on the rest.
- **A billed answer is bought once, wherever it is read.** Places answers are kept in the conductor's
  queries cache (site_conductor's docs/ARCHITECTURE.md, Queries cache), prod's the one authority,
  which its `Held` reads in process; the CLI's reads a copy of prod's `panel.db` (site_conductor's
  `prod-restore`), read-only: what the copy lacks is lacking, and the study names it. Bulk archives
  are free and cache in the work dir alone — the INSEE
  archive is ~87 MB. A 200 that is a refusal is never kept: it would answer for its request everywhere.
- **Nothing in that cache expires.** An answer older than its kind's configured age is served anyway,
  and its age rides onto the map beside the imputed flag. Google's daily search quota is the scarce
  side, and refetching spends a day of it to learn what is mostly the same thing; a pull is the only
  thing that re-asks. Age is a caption and a changed tiling is a setting;
  neither is a trigger.
- **A cache key covers everything its answer depends on**, so a changed input is a different file
  rather than a stale one and nothing needs invalidating. An answer is keyed on the ground it is
  about — a tile of one Web-Mercator quadtree — never on the area that asked, so overlapping areas
  share it. A derived answer is cached only where it is
  reproducible from what is kept: the grid extract keeps the archive's own rows and re-parses them,
  because caching the floats would need a round-trip no text format gives.
- **What a pull lacks is named, never guessed at.** No API key can read what is left of its
  project's `SearchTextRequestPerDayPerProject`, so placement does not count one: it names the
  searches a study lacks, and the conductor's keys and meters decide what buying them costs.

## Sources are an enum

The set of statistical sources is closed and known at compile time, so `GridSource` is an enum
matched on, not a trait implemented. Adding a country adds a variant and a row to the table
it matches on.

What the sources disagree about is which columns exist, so a grid carries columns discovered from
its archive rather than a struct per country. Expressions name those columns, and an expression
naming one that does not exist fails against the source's own column list before a single POI call
is made.

## What a map like this cannot know

These are not caveats about the implementation. They are the distance between the model and the
decision, and they are the difference between a map that informs one and a map that launders a
guess into authority.
- **Motorisation is estimated, not measured.** No 200 m source publishes households-with-a-car;
  INSEE has it only at IRIS level. `bases/FR.toml` infers it from the house/flat split
  (`men_mais × 1.55 + men_coll × 0.85`) as a base layer, and a detailing category's demand reads
  it. Directionally right, not a measurement.
- **Most fine-grained cells are modelled.** ~80 % of INSEE 200 m cells have fewer than 11 fiscal
  households, so their values are imputed. The flag is carried through to the map and the "hide
  imputed cells" control exists for it. Trust dense cells, distrust isolated rural ones.
- **Residential demand only.** No traffic counts, no workplace population, no rent or premises
  availability. For a walk-in bay, passing traffic matters as much as who lives nearby.
- **A competitor sits at its registered address**, which understates a mobile operator's reach.
- **A tier is which search found a competitor.** Places shows no Business Profile category, so a
  shop is filed under the first of the category's searches that returned it: one that only an
  additional category's name search surfaced counts at that category's weight, whatever it is.
- **A Places type is not a Business Profile category.** `primaryTypeDisplayName` names the Places
  type, of which Table A has a few hundred, with no cleaning, detailing or handyman type among them,
  and many competitors carry none. The category list in the map panel shows it beside the tiers as
  evidence; nothing tiers by it.
- **Every constant in a category is a guess with a sane magnitude**, not a fitted value. They are in
  the category so they can be argued with. Competitor weight is the exception: it comes out of
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
