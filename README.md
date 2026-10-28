# gmaps_optimal_placement
![Minimum Supported Rust Version](https://img.shields.io/badge/nightly-1.92+-ab6000.svg)
![Lines Of Code](https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/valeratrades/b48e6f02c61942200e7d1e3eeabf9bcb/raw/gmaps_optimal_placement-loc.json)
<br>
[<img alt="ci errors" src="https://img.shields.io/github/actions/workflow/status/EV-Service-Arb/gmaps_optimal_placement/errors.yml?branch=main&style=for-the-badge&style=flat-square&label=errors&labelColor=420d09" height="20">](https://github.com/EV-Service-Arb/gmaps_optimal_placement/actions?query=branch%3Amain) <!--NB: Won't find it if repo is private-->
[<img alt="ci warnings" src="https://img.shields.io/github/actions/workflow/status/EV-Service-Arb/gmaps_optimal_placement/warnings.yml?branch=main&style=for-the-badge&style=flat-square&label=warnings&labelColor=d16002" height="20">](https://github.com/EV-Service-Arb/gmaps_optimal_placement/actions?query=branch%3Amain) <!--NB: Won't find it if repo is private-->

Where should a service business open?

Two Nix files describe it. A location gives an area and the statistical grid behind it; a trade —
applied to that location — gives what counts as a competitor and how demand follows from the columns
the grid publishes. `gmaps_optimal_placement` paints that demand under the competitors already on the
ground and serves the map. How much any one competitor counts is fitted, not guessed:
`gmaps_optimal_placement probe` asks Google the study's own queries from equal-demand points across the area,
and `gmaps_optimal_placement fit` estimates what its ordering rewards. `gmaps_optimal_placement
strength` is what says that estimate is worth painting: every candidate model cross-validated over
the same orderings, and a refusal if counting every competitor the same does as well.

A trade also names groups of queries, and `gmaps_optimal_placement searches` charts how often each group is
asked for over the trailing year — the map says where the people are, this says how many of them
are looking for the thing.

The question is the input: another city is a location file, another trade is a trade file, and every
pairing of the two is a study. Neither is an edit to the code. A location on its own is a study too
— the grid, and nothing a trade decides — and it is the one the Google Maps quota is not spent on.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the invariants, and
[examples/](examples) for worked studies.
<!-- markdownlint-disable -->
<details>
<summary>
<h2>Installation</h2>
</summary>

```sh
cargo install --path gmaps_optimal_placement
```

</details>
<!-- markdownlint-restore -->

## Usage
```sh
# Set your Google Maps API key. The server reads it; it never lands in a file.
export GOOGLE_MAPS_KEY=...

# Serve the map. A study is a trade applied to a location, and the two directories are all this
# takes — the page picks the pairing, with a field per axis: type in either, Enter picks its top
# row, and Load opens the pair as a tab over one map. Press `t` for one more. Both flags default to what is
# shown here. Bulk archives and API responses cache in $GMAPS_OPTIMAL_PLACEMENT_WORK (default
# ./docs/.readme_assets/tmp/geo); candidates you promote from the map land in $XDG_DATA_HOME/gmaps_optimal_placement.
gmaps_optimal_placement serve --trades examples/trades --locations examples/locations --open

# Name an axis as a file rather than a directory to narrow it; name both and the pairing is built
# before the server starts, which is how a scripted run never meets the picker.
gmaps_optimal_placement serve -t examples/trades/cleaning.nix -l examples/locations/Lyon.nix

# The last row of the trade field is "no trade". Select it to see the city from the statistical grid
# only. This map has no demand layer and no competitors, and it uses no Google Maps calls.

# Chart the monthly search volume of each query group. Set the credentials of the provider first:
# GOOGLE_ADS_{DEVELOPER_TOKEN,CLIENT_ID,CLIENT_SECRET,REFRESH_TOKEN,CUSTOMER_ID}, or
# DATAFORSEO_{LOGIN,PASSWORD}. Each axis is picked through fzf.
gmaps_optimal_placement searches -t examples/trades -l examples/locations

# Show the schema of the study document.
gmaps_optimal_placement schema

# Find the towns in a country that are worth a study. Count what a cadastre draws, per commune, and
# divide by a column of the grid. Towns below the floor are not counted, and not downloaded.
gmaps_optimal_placement misc france pool --per ind --floor 2000
```

From a checkout, `nix run` does the same and builds the wasm client first. Everything after `--` is
the binary's own, so the two axes are the same flags, and paths are read from the repository root.

```sh
nix run .#open
nix run .#open -- -l examples/locations/Lyon.nix
nix run .#searches -- -t examples/trades/cleaning.nix
nix run .#help
```

## Sources

| `grid.source` | Area | Cell | Publishes |
|---|---|---|---|
| `insee_filosofi_200m` | France | 200 m | households, housing type, standard of living |
| `geostat_1km` | EU | 1 km | census counts by age, sex, employment, origin |

| `searches.provider` | Credentials | Notes |
|---|---|---|
| `google_ads` | `GOOGLE_ADS_*` | Keyword Planner at the source; needs Basic API access |
| `dataforseo` | `DATAFORSEO_LOGIN`, `DATAFORSEO_PASSWORD` | the same numbers, resold per call |

## Expressions

`column`, `model` and `layer` are arithmetic over the column names the chosen source publishes,
plus `max`, `min`, `clamp`, `sqrt` and `pow`. A name that does not exist is an error at load,
against the source's own column list.


<br>

<sup>
	This repository follows <a href="https://github.com/valeratrades/.github/tree/master/best_practices">my best practices</a> and <a href="https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md">Tiger Style</a> (except "proper capitalization for acronyms": (VsrState, not VSRState) and formatting). For project's architecture, see <a href="./docs/ARCHITECTURE.md">ARCHITECTURE.md</a>.
</sup>

#### License

<sup>
	Licensed under <a href="LICENSE">Blue Oak 1.0.0</a>
</sup>

<br>

<sub>
	Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this crate by you, as defined in the Apache-2.0 license, shall
be licensed as above, without any additional terms or conditions.
</sub>

