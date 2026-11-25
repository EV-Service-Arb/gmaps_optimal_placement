The local GUI: a Leptos island over the two axes the CLI hands it, and the axum server that feeds
their product. The CLI's `view` runs it; nothing here reads settings or knows what `Held` is.

```text
gmaps_optimal_placement view [-c ..] --port <p> --site <dir> [--open]
        │   categories, locations, a Work per build, and who buys a run — the CLI's
        ▼
   serve ─┬─ GET /                            shell — the Maps bootstrap, the CSS, the island
          ├─ GET /studies.json                the two axes, by the labels the picker shows
          ├─ GET /payload/{category}/{location}  load(&def)?.build(&work), once per pairing
          ├─ GET /place/{location}            the same with no category: the grid alone
          ├─ POST /quote/{category}/{location}   what a run buys at least, to be confirmed; from this page alone
          ├─ POST /run/{category}/{location}     bought (`Buyer`), and built again; from this page alone
          ├─ GET /page                        --open: held by every page while it is open; an event closes it
          ├─ GET /pkg/*                       the client wasm (`hydrate`), built apart from the server (`ssr`)
          └─ /api/*                           server fns: the keys, under XDG_CONFIG_HOME
```

A build is the library's `load` and `build` on a blocking thread, over a fresh `Work`, exactly as
the conductor builds its own, and what is not held is the payload's notice. A run is the `Buyer`'s and
goes on without the page that asked for it; every payload built before it is built again when asked for.

A tab is one study's `Model` plus the controls as they stood when it was last left. Switching writes
the live signals into the tab being left and reads the next one into them. There is one
`google.maps` instance and one `gmaps_optimal_placement_overlay::Overlay` on it for the page's
whole life: a switch is the overlay shown the next payload.

The overlay paints the heat, the pressure field, the dots and the hint, and tells the legend and each
mark left; the panel is drawn from that and from `Summary`. The island keeps the `Model` for what the
overlay does not do: the pin cards and the top-N sweep. A pin lasts as long as its tab. `map_core.js` owns the map instance and the pins.

Nothing throws across that boundary: under `panic=abort` a rejected promise reaching wasm kills the
app, so `map_core.js`'s `mount` returns a banner string instead of the map.
