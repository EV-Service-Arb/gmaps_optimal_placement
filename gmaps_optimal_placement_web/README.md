# gmaps_optimal_placement_web

The server behind the conductor, its one client: a definition in, its payload out.

```text
GET  /trades.json        Read   sa_core::placement::Catalog: what a category may be studied by, and where
POST /studies            Read   a Definition → its Payload, built once and kept; never spends
POST /studies/pull       Spend  ?budget=<n>, a Definition → its Payload, built again buying at most n
                                Places searches; 402 unaffordable when the sweep needs more. Every
                                answer carries x-billed: what it bought, `{item: n}`
```

Building a study is the CLI's job, so it arrives as a closure: this crate never learns what a `.nix`
file is, nor that a trade is a function of an area.
