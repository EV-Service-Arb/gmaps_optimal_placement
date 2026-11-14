What gmaps_optimal_placement paints over a `google.maps.Map`: the heat, the competitor-pressure field
and the competitor dots. Two consumers, one paint path:

```text
  gop's Leptos map ──▶ Scene ──▶ paint.js ──▶ three canvases + markers on the host's map
  any host page ──import()──▶ /mfe/overlay.js ──▶ Overlay (feature `export`) ──▶ Scene
```

`Scene` decides every number — the ramp over the cells in view, the haze's alpha, the iso-lines —
and `paint.js` projects and fills, because that needs `fromLatLngToDivPixel` every frame. The heat is
warm and sequential; blue is the pressure field's alone, and the dots are coloured by Google category.

`overlay(map, {api, csrf})` resolves to an `EventTarget` over one study at a time, the
`core::define::Definition` a host hands to `show`:

```text
show(def | null)    PUT {api}/studies/{id} once a session, then GET; never spends; throws on a bad def
heat(id | null)     a `data.layers[].id`
tune({lambda_m, tiers: {name: {weight, show}}})   null = the payload's own
pressure(on) · competitors(on) · opacity(x) · hideImputed(on)
pull()              POST {api}/studies/{id}/pull, the one thing that buys
dispose()

events  data     core::model::Summary | null — what the shown payload is; on landing and on show(null)
        legend   {heat, pressure, competitors} — what is drawn in view; `cells` 0, `levels` null and
                 `in_view` 0 are each a layer that draws nothing
        notice   {text | null, missing | null} — why the map is less than asked; null text clears it
```

A data problem is a `notice`, never a throw. Every event is about the study shown now.
