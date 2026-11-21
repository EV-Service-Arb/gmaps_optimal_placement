# Locations

There is no code here, and no trade: a study is a **category over an area**, and the category is the
conductor's, as a definition it handed in carries it. What lives here is ground: a location is an
area and the premises worth asking about in it (`gmaps_optimal_placement schema location`). The
country's base under it is compiled into the library (`gmaps_optimal_placement/bases/<CC>.toml`).

```
          the library's bases/FR.toml          grid, nv, cars, the base layers
                   ^               ^
   locations/  Clermont-Ferrand.toml  Lyon.toml   area, candidate
                   ^      ^      ^        ^
   the conductor's category (its trade — a Google category —, country, additional Google
   categories, keywords, excluded, demand)
                trade's name + keywords        → the sweep's tier-1 searches
                each additional's name         → one search, one tier each
                weighted keywords              → the terms competitors are scored on
                demand                         → its columns and layers after the base's
```

An area handed to no category is a study too — named after the area, carrying the grid, the raw
INSEE layers and the candidate addresses. It makes no Places call. Behind the conductor the area is
the conductor's, over the same base, and carries no candidate.

Every category × every location in its country is a study, named `<trade>·<CC>·<slug>`.

Nothing in a base may name a trade, and nothing in a category may vary with the area. What would
want to — the tiling of the POI sweep, the probe's bias radius — is derived instead: the sweep asks
tiles of one quadtree and quarters a tile whenever a query comes back at Google's 60-result cap, and
the radius is the frame ÷ the probe's strata as a circle. The catchment λ the map opens on is what
`rank::COEF`'s distance term implies.

## What the map gives you

| Layer | Meaning |
|---|---|
| **Underserved demand** ★ | demand ÷ (1 + competition already reaching the cell) — the one to shop on |
| Demand | the category's demand |
| Population / Households | raw INSEE |
| Standard of living €/yr | per-person rate, **not** a density |
| Estimated cars | houses × 1.55 + flats × 0.85, inferred |

The first two need a category with a demand. Without a category the bottom rows are the whole table,
and λ, the tier weights, the pressure field, the site sweep and the candidate comparison are
disabled; with one but no demand, the competitors and their pressure are there and nothing that
reads a demand is.

Competitor pressure — Σ competitors of weight × exp(−distance/λ) — is not a row: it is the blue
field drawn over whichever layer is on.

## What these models do not know

See [docs/ARCHITECTURE.md](../docs/ARCHITECTURE.md#what-a-map-like-this-cannot-know). Read it before
quoting a number from one of these maps at anyone.
