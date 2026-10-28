# Worked studies

There is no code here. A study is a **trade applied to a location**: `import trades/cleaning.nix
(import locations/Lyon.nix)`.

```
                      _France.nix                grid, nv, the raw INSEE layers, the volume provider
                   ^               ^
   locations/  Clermont-Ferrand.nix   Lyon.nix   bbox, the place volume resolves against, candidate
                   ^      ^      ^        ^
   trades/   car_detailing.nix  plumbing.nix  cleaning.nix
                                     `loc: { … }` — a function of the city it is handed

   poi / column / model / rank / searches.group are the trade, and none of them names a city
```

A location handed to no trade is a study too — `import locations/Lyon.nix` on its own, named after
the city, carrying the grid, the raw INSEE layers and the candidate addresses. It is the picker's
last trade row, `(no trade · grid only)`, and it makes no Places call.

Every trade × every location is a study, named `<trade>_-_<location>` by the tool. That name keys
the pin file under `$XDG_DATA_HOME` and the searches chart, so a candidate promoted on the Lyon
cleaning map cannot land on the Clermont one.

Nothing in a trade file may vary with the city. What would want to — the tiling of the POI sweep,
the probe's bias radius — is derived instead: the sweep quarters a tile whenever a query comes back
at Google's 60-result cap, and the radius is `area.bbox ÷ rank.nodes` as a circle. `lambda_m` and
the term weights stay flat across cities on purpose: λ is the trade's tolerance for unpaid commute
and the slider's *opening* position, and a term's weight is what a searcher typing it is worth.

```fish
nix run .#open                                   # the page opens on a picker: trade and city, then Load
nix run .#open examples/trades/plumbing.nix examples/locations/Clermont-Ferrand.nix  # or name one
nix run .#searches                               # fzf twice; monthly volume per query group
nix run .#study                                  # the detailing map, asserted in headless Chromium
```

## What the map gives you

| Layer | Meaning |
|---|---|
| **Underserved demand** ★ | demand ÷ (1 + competition already reaching the cell) — the one to shop on |
| Competitor pressure | Σ competitors of weight × exp(−distance/λ) |
| Demand | the study's `model.demand` |
| Population / Households / Households in houses / in flats | raw INSEE |
| Standard of living €/yr | per-person rate, **not** a density |

The first three need a trade. Without one the bottom two rows are the whole table, and λ, the tier
weights, the site sweep and the candidate comparison are disabled.

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
200 m grid, 32 columns        text search x 8 queries, each tiled as far as the 60-result cap needs
   |  bbox filter, 3035->4326    |  tier + drop
   v                             v
10 446 cells  ------------.  .---------- shops, tiered
455 k people               \/
                            served map
```

## Inputs

`GOOGLE_MAPS_KEY` must be set. The INSEE archive downloads once into `$XDG_CACHE_HOME/gmaps_optimal_placement/data/` (~87 MB) and is
shared by every study over the same country; Places responses are cached per request, so editing the
tiering rules costs nothing and a second trade only pays for its own queries.

## What these models do not know

See [docs/ARCHITECTURE.md](../docs/ARCHITECTURE.md#what-a-map-like-this-cannot-know). Read it before
quoting a number from one of these maps at anyone.
