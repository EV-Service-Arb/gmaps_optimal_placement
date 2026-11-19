# gmaps_optimal_placement_web

The server behind the conductor, its one client: a definition in, its payload out.

```text
GET  /health             open
GET  /catalog.json       Read   sa_core::placement::Catalog: the countries a category may be studied in
POST /studies            Read   a Definition → its Payload, built once and kept, of what is held
POST /studies/asks       Spend  a Definition → sa_core::placement::Asks: the Places searches a pull of
                                it still lacks, for the conductor to buy; asked again until none
POST /studies/pull       Spend  a Definition → its Payload, built again of what the pull bought
```

Nothing here buys.

Building a study is the CLI's job, so it arrives as a closure: this crate never learns what a `.nix`
file is.
