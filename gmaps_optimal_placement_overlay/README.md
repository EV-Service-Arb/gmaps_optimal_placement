What gmaps_optimal_placement paints over a `google.maps.Map`: the heat, the competitor-pressure field
and the competitor dots. Two consumers, one paint path:

```text
  gop's Leptos map ──▶ Scene ──▶ paint.js ──▶ three canvases + markers on the host's map
  any host page ──import()──▶ /mfe/gmaps_optimal_placement_overlay.js ──▶ Overlay (feature `export`) ──▶ Scene
```

`Scene` decides every number — the ramp over the cells in view, the haze's alpha, the iso-lines —
and `paint.js` projects and fills, because that needs `fromLatLngToDivPixel` every frame. The heat is
warm and sequential; blue is the pressure field's alone, and the dots are coloured by Google category.

A host awaits the module's default `init()`, then `new Overlay(map, api, csrf, onData, onLegend,
onNotice)`: one study at a time, the id of an `sa_core::placement::Definition` the host has PUT (the
conductor's server does), handed to `show`. wasm-bindgen's `.d.ts` beside the module types all of it,
the callbacks' payloads included (tsify over the Rust types):

```text
show(id | null)     GET {api}/studies/{id}; never spends
heat(id | null)     a `Summary.layers[].id`
tune(Tune)          {lambda_m, tiers: {name: {weight, show}}}; null = the payload's own
pressure(on) · competitors(on) · opacity(x) · hideImputed(on)
pull()              POST {api}/studies/{id}/pull, the one thing that buys
dispose()

onData    Summary | null — what the shown payload is; on landing and on show(null)
onLegend  Legend — what is drawn in view
onNotice  Notice — why the map is less than asked; a null text clears it
```

A data problem is a notice, never a throw. Everything told is about the study shown now.
