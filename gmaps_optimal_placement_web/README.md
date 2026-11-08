# gmaps_optimal_placement_web

The map. A Leptos island over a set of trades and a set of locations, and the axum server that feeds
their product.

```text
gmaps_optimal_placement serve trades/ locations/
        │
        ├─ GET /              shell — the Maps bootstrap, the CSS, the island marker
        ├─ GET /studies.json  the two axes, and the pairings the CLI prebuilt
        ├─ GET /payload/{trade}/{location}  the evaluated study, multi-MB, built once per pairing
        ├─ GET /place/{location}            the same city with no trade over it: the statistical
        │                                   archive alone, and not one billed call
        ├─ GET /find/{location}?q=          the one place a search names in that city's frame, as
        │                                   cached and as refused under `--block-spend` as any
        │                                   Places answer
        ├─ GET /pkg/*         the client wasm
        ├─ GET /mfe/*         the overlay module a host page imports
        └─ /api/*             server fns: the pin file under XDG_DATA_HOME,
                              the key bindings under XDG_CONFIG_HOME
```

`--behind-conductor` drops `/`, `/pkg` and `/api`; everything left but `/mfe/*` needs the
conductor's assertion.

The trade field ends in one row the server has no file for: taking it opens the location by itself.
λ, the tier weights and both sweeps are then disabled rather than hidden — the panel is what says
what a trade would buy.

A tab is one study's `Model` plus the controls as they stood when it was last left. Switching writes
the live signals into the tab being left and reads the next one into them; `press` and `unmet` are
recomputed rather than stored, because they are functions of λ and the tier weights. There is one
`google.maps` instance for the page's whole life — the scene and the pins each replace what is
there, so a switch needs no teardown.

Rust owns every number: recompute, colourise, the capture score, the top-N sweep, every report row
and the whole control panel. `map_core.js` owns the map instance and the pins; the heat, the
pressure field and the competitor dots go through `gmaps_optimal_placement_overlay::Scene`, the path
the conductor's page paints through too.

Nothing throws across that boundary: under `panic=abort` a rejected promise reaching wasm kills the
app, so `map_core.js`'s `mount` returns a banner string instead of the map.
