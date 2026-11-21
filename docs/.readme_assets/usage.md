```sh
# What the conductor bought is read from its queries cache, and mirrored in the work dir; `local`
# reads the mirror alone. Nothing is bought here.
#   queries_cache = "https://sa.evinvest.ltd/api/queries_cache";
# `view` draws on Google Maps.
#   maps_key = "...";

# The studies the work dir holds: what the conductor studied, and each location under each of its
# categories. Then one of them on a map, until Ctrl-C.
gmaps_optimal_placement view
gmaps_optimal_placement view <id> --overlay <wasm-bindgen output> --port 52850

# Refit the ranking model over every ordering the work dir holds.
gmaps_optimal_placement fit -l examples/locations

# Show the schema of a country's base, or of a location.
gmaps_optimal_placement schema base

# Find the towns in a country that are worth a study. Count what a cadastre draws, per commune, and
# divide by a column of the grid. Towns below the floor are not counted, and not downloaded.
gmaps_optimal_placement misc france pool --per ind --floor 2000
```

From a checkout, `nix run` opens a study, picked with fzf, with the overlay the flake builds.
Everything after `--` is the binary's own, and paths are read from the repository root.

```sh
nix run
nix run -- <id>
nix run .#help
```
