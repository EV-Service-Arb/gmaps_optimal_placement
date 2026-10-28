```sh
# Set your Google Maps API key. The server reads it; it never lands in a file.
export GOOGLE_MAPS_KEY=...

# Serve the map. A study is a trade applied to a location, and the two directories are all this
# takes — the page picks the pairing, with a field per axis: type in either, Enter crosses, and
# Ctrl+Enter opens it as a tab over one map. Press `t` for one more. Both flags default to what is
# shown here. Bulk archives and API responses cache in $XDG_CACHE_HOME/gmaps_optimal_placement;
# candidates you promote from the map land in $XDG_DATA_HOME/gmaps_optimal_placement.
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
