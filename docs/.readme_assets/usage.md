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
