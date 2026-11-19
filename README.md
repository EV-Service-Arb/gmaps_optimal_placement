# gmaps_optimal_placement
![Minimum Supported Rust Version](https://img.shields.io/badge/nightly-1.92+-ab6000.svg)
![Lines Of Code](https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/valeratrades/b48e6f02c61942200e7d1e3eeabf9bcb/raw/gmaps_optimal_placement-loc.json)
![Build time](https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/valeratrades/b48e6f02c61942200e7d1e3eeabf9bcb/raw/gmaps_optimal_placement-build.json)
![Test time](https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/valeratrades/b48e6f02c61942200e7d1e3eeabf9bcb/raw/gmaps_optimal_placement-test.json)
<br>
[![errors](https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/valeratrades/b48e6f02c61942200e7d1e3eeabf9bcb/raw/gmaps_optimal_placement-errors.json)](https://github.com/EV-Service-Arb/gmaps_optimal_placement/actions/workflows/errors.yml?query=branch%3Amain)
[![warnings](https://img.shields.io/endpoint?url=https://gist.githubusercontent.com/valeratrades/b48e6f02c61942200e7d1e3eeabf9bcb/raw/gmaps_optimal_placement-warnings.json)](https://github.com/EV-Service-Arb/gmaps_optimal_placement/actions/workflows/warnings.yml?query=branch%3Amain)

Where should a service business open?

A study is a category over an area. The area — a city's outline or a circle, over a country's base —
gives the statistical grid behind it; the category, the conductor's trade set up in a country, gives
the Google categories and keywords its competitors are found and scored by, and how demand follows
from the columns the grid publishes. `gmaps_optimal_placement` paints that demand under the
competitors already on the ground, and the conductor's map draws it. How much any one competitor
counts is fitted, not guessed: `gmaps_optimal_placement fit` estimates what Google's ordering
rewards, over the orderings the conductor's queries cache already holds. `gmaps_optimal_placement
strength` is what says that estimate is worth painting: every candidate model cross-validated over
the same orderings, and a refusal if counting every competitor the same does as well.

The question is the input: another city is another area, another trade another category, and
neither is an edit to the code. An area on its own is a study too — the grid, and nothing a category
decides — and it is the one the Google Maps quota is not spent on.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the invariants, and
[examples/](examples) for the country base and the locations.
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
# Where what is bought is cached for everyone, so no machine buys the same query twice: the
# Service-Arb conductor's queries cache, or `local` for this machine's work dir alone. No Places
# search is bought here: the conductor buys what a pull lacks.
#   queries_cache = "https://sa.evinvest.ltd/api/queries_cache";

# Serve the Service-Arb conductor, the map's one host: it hands in each category and area it wants
# studied, over the country bases, and draws the payload itself.
PANEL_ASSERTION_KEYS=... gmaps_optimal_placement serve --countries examples/countries

# Refit the ranking model over every ordering the work dir holds.
gmaps_optimal_placement fit -l examples/locations

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
nix run .#help
```

## Sources

| `grid.source` | Area | Cell | Publishes |
|---|---|---|---|
| `insee_filosofi_200m` | France | 200 m | households, housing type, standard of living |
| `geostat_1km` | EU | 1 km | census counts by age, sex, employment, origin |

## Expressions

`column`, `layer` and the category's demand are arithmetic over the column names the chosen
source publishes, plus `max`, `min`, `clamp`, `sqrt` and `pow`. A name that does not exist is an
error at load, against the source's own column list.


<br>

#### Dev links
- <sup><a href="./docs/ARCHITECTURE.md">architecture</a></sup>
- <sup><a href="https://github.com/EV-Main/.github/blob/main/code/STYLE.md">style</a></sup>
- <sup><a href="https://github.com/EV-Main/.github/tree/main/code/conventions">conventions</a></sup>
- <sup><a href="https://github.com/EV-Main/.github/blob/main/code/conventions/git">contributing</a></sup>

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

