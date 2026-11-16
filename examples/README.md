# Worked studies

There is no code here. A study is a **category over an area**: the category (`categories/*.json`,
in the shape the conductor hands one in) names the trade file that studies it, and that trade is
applied to the area — `import trades/cleaning.nix (import locations/Lyon.nix)`.

```
                     countries/FR.nix            grid, nv, cars, the base layers, the volume provider
                   ^               ^
   locations/  Clermont-Ferrand.nix   Lyon.nix   frame, the place volume is charted over, candidate
                   ^      ^      ^        ^
   trades/   car_detailing.nix  plumbing.nix  cleaning.nix
                                     `loc: { … }` — a function of the area it is handed
                   ^      ^      ^
   categories/  car_detailing_FR.json  plumbing_FR.json  cleaning_FR.json
                Google categories → the sweep's queries and the first tier's type
                keywords          → the terms competitors are scored on, weighed by volume

   column / model / poi tiers and drop / searches.group are the trade, and none of them names an area
```

An area handed to no category is a study too — `import locations/Lyon.nix` on its own, named after
the city, carrying the grid, the raw INSEE layers and the candidate addresses. It is the picker's
last category row, `(no category · grid only)`, and it makes no Places call. Behind the conductor
the area is the conductor's, over `countries/<CC>.nix`, and carries no candidate.

Every category × every location in its country is a study, named `<study>·<CC>·<slug>`. That name
keys the pin file under `$XDG_DATA_HOME` and the searches chart, so a candidate promoted on the Lyon
cleaning map cannot land on the Clermont one.

Nothing in a trade file may vary with the area. What would want to — the tiling of the POI sweep,
the probe's bias radius — is derived instead: the sweep asks tiles of one quadtree and quarters a
tile whenever a query comes back at Google's 60-result cap, and the radius is the frame ÷
`rank.nodes` as a circle. `lambda_m` stays flat across areas on purpose: λ is the trade's tolerance
for unpaid commute and the slider's *opening* position.

```fish
nix run .#open                                   # the page opens on a picker: category and city, then Load
nix run .#open -- -c examples/categories/plumbing_FR.json -l examples/locations/Clermont-Ferrand.nix  # or name one
nix run .#searches                               # fzf twice; monthly volume per query group
nix run .#study                                  # the detailing map, asserted in headless Chromium
```

## What the map gives you

| Layer | Meaning |
|---|---|
| **Underserved demand** ★ | demand ÷ (1 + competition already reaching the cell) — the one to shop on |
| Demand | the study's `model.demand` |
| Population / Households | raw INSEE |
| Standard of living €/yr | per-person rate, **not** a density |
| Estimated cars | houses × 1.55 + flats × 0.85, inferred |

The first two need a category. Without one the bottom rows are the whole table, and λ, the tier
weights, the pressure field, the site sweep and the candidate comparison are disabled.

Competitor pressure — Σ competitors of weight × exp(−distance/λ) — is not a row: it is the blue
field drawn over whichever layer is on.

Live controls: catchment radius λ, the weight of one tier against the other, opacity, marker filters
per tier, hide imputed cells.

- **Click anywhere** → capture score, demand within 1 / 3 km, nearest competitor of each tier,
  counts within 2 / 5 km.
- **Compare candidates** → the location's `candidate` addresses side by side.
- **Rank top 10 sites** → Huff-style sweep over ~2 600 candidate cells, greedy pick kept ≥1.5 km
  apart.

For car detailing at the defaults the ranking lands on **Chamalières, Châtel-Guyon, Ceyrat,
Saint-Bonnet-près-Riom, Riom, Royat** — affluent and house-heavy to the west, thin competition to
the north. Downtown Clermont scores high on people and low on opportunity: 6 detailers within 2 km
already.

```
INSEE Filosofi 2021           Google Places API (New)
200 m grid, 32 columns        text search per Google category, over quadtree tiles quartered as far as the 60-result cap needs
   |  bbox filter, 3035->4326    |  tier + drop
   v                             v
10 446 cells  ------------.  .---------- shops, tiered
455 k people               \/
                            served map
```

## Inputs

An account in the config's `google` must provide `places`. The INSEE archive downloads once into `$XDG_CACHE_HOME/gmaps_optimal_placement/data/` (~87 MB) and is
shared by every study over the same country; Places responses are cached per request on one quadtree,
so editing the tiering rules costs nothing, a second category only pays for its own queries, and an
area overlapping one already swept pays only for the new ground. Ranking terms are weighed by the
keywords' search volume over the country, which needs the volume provider's credentials
(a `google` account providing `ads`, or `DATAFORSEO_*`); without them a map opens with no competitors and says so.

## What these models do not know

See [docs/ARCHITECTURE.md](../docs/ARCHITECTURE.md#what-a-map-like-this-cannot-know). Read it before
quoting a number from one of these maps at anyone.
