What gmaps_optimal_placement paints over a `google.maps.Map`: the heat, the competitor-pressure field
and the competitor dots. Two consumers, one paint path:

```text
  gop's Leptos map ──▶ Scene ──▶ paint.js ──▶ three canvases + markers on the host's map
  any host page ──import()──▶ /mfe/overlay.js ──▶ Overlay (feature `export`) ──▶ Scene
```

`Scene` decides every number — the ramp over the cells in view, the haze's alpha, the iso-lines —
and `paint.js` projects and fills, because that needs `fromLatLngToDivPixel` every frame. The heat is
warm and sequential; blue is the pressure field's alone.

`overlay(map, {api, csrf})` resolves to an `EventTarget`: `show`, `heat`, `pressure`, `competitors`,
`opacity`, `hideImputed`, `pull`, `dispose`; events `layers`, `legend`, `notice`. GETs never spend;
`pull` is the one POST. A data problem is a `notice`, never a throw.
