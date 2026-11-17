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
`gmaps_optimal_placement fit` estimates what Google's ordering rewards, over the orderings the
conductor's queries cache already holds. `gmaps_optimal_placement
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
# Every Google key is a slot of a billing account in accounting's google.nix, each read from the
# environment so it never lands in a file; ~/.config/gmaps_optimal_placement.nix imports the list whole.
# Keyword Planner uses the one account that provides `ads`; no Places search is bought here, the
# conductor buys what a pull lacks:
#   google = (import ~/s/ev/_sa/accounting/google.nix).accounts;
# and where what is bought is cached for everyone, so no machine buys the same query twice: the
# Service-Arb conductor's queries cache, or `local` for this machine's work dir alone:
#   queries_cache = "https://sa.evinvest.ltd/api/queries_cache";

# Chart the monthly search volume of each query group. Set the credentials of the provider first:
# an account providing `ads`, or
# DATAFORSEO_{LOGIN,PASSWORD}. Each axis is picked through fzf.
gmaps_optimal_placement searches -l examples/locations

# Serve the Service-Arb conductor, the map's one host: it hands in each category and area it wants
# studied, over the trades and the country bases, and draws the payload itself.
PANEL_ASSERTION_KEYS=... gmaps_optimal_placement serve --trades examples/trades --countries examples/countries

# Show the schema of the study document.
gmaps_optimal_placement schema

# Find the towns in a country that are worth a study. Count what a cadastre draws, per commune, and
# divide by a column of the grid. Towns below the floor are not counted, and not downloaded.
gmaps_optimal_placement misc france pool --per ind --floor 2000
```

From a checkout, `nix run` does the same. Everything after `--` is the binary's own, and paths are
read from the repository root.

```sh
nix run .#conductor -- --port 52850
nix run .#searches -- -c 'cleaning·FR (trade 2)'
nix run .#help
```

## Sources

| `grid.source` | Area | Cell | Publishes |
|---|---|---|---|
| `insee_filosofi_200m` | France | 200 m | households, housing type, standard of living |
| `geostat_1km` | EU | 1 km | census counts by age, sex, employment, origin |

| `searches.provider` | Credentials | Notes |
|---|---|---|
| `google_ads` | the one `google` account providing `ads` | Keyword Planner at the source; needs Basic API access |
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

