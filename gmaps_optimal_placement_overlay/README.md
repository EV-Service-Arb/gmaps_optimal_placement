Placement's drawing over the home map's `google.maps.Map`: the heat, the competitor-pressure field
and the competitor dots of one payload. A renderer only: the host fetches, caches and pulls.

```text
home map ──import()──▶ frontend/src/shared/placement_overlay/ (wasm-bindgen --target web) ──▶ Overlay ──▶ Scene ──▶ paint.js
```

`Scene` decides every number — the ramp over the cells in view, the haze's alpha, the iso-lines —
and `paint.js` projects and fills, because that needs `fromLatLngToDivPixel` every frame. The heat is
warm and sequential; blue is the pressure field's alone, and the dots are coloured by Google category.

The host awaits the module's default `init()`, then `new Overlay(map, emit)`. wasm-bindgen's `.d.ts`
types all of it, the events included (tsify over the Rust types, placement core's among them):

```text
show(payload | null, pulled)   the payload as placement wrote it; pulled: a pull's answer
heat(id | null)                a `Summary.layers[].id`
tune(Tune)                     {lambda_m, tiers: {name: {weight, show}}}; null = the payload's own
pressure(on) · competitors(on) · opacity(x) · hideImputed(on) · dispose()

emit  {kind: "data", detail: Summary | null}   what the shown payload is; on every show
      {kind: "legend", detail: Legend}         what is drawn in view
      {kind: "notice", detail: Notice}         why the map is less than asked; a null text clears it
```

A data problem is a notice, never a throw. `Model::try_new` checks a payload's weights against the
`rank::COEF` this was linked with: the crate and placement's server build from the same core.
