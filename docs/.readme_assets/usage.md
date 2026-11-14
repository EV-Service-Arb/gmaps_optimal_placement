```sh
# Name your Google Maps API keys in ~/.config/gmaps_optimal_placement.nix, each read from the
# environment so it never lands in a file. Searches are bought on the free (trial-credit) projects
# first, moving on as each one's day is spent; the map loads with the first:
#   google_maps = [ { key = { env = "GOOGLE_MAPS_KEY_TRIAL"; }; free = true; } { key = { env = "GOOGLE_MAPS_KEY"; }; free = false; } ];
export GOOGLE_MAPS_KEY_TRIAL=... GOOGLE_MAPS_KEY=...
# and where what is bought is cached for everyone, so no machine buys the same query twice: the
# Service-Arb conductor's queries cache, or `local` for this machine's work dir alone:
#   queries_cache = "https://sa.evinvest.ltd/api/queries_cache";

# Serve the map. A study is a category (a trade in a country, in the shape the Service-Arb
# conductor holds one: its Google categories, its keywords, the trade file that studies it) over a
# location, and the two directories are all this takes — the page picks the pairing, with a field
# per axis: type in either, Enter picks its top row, and Load opens the pair as a tab over one map.
# Press `t` for one more. Both flags default to what is shown here. Bulk archives cache in
# $XDG_CACHE_HOME/gmaps_optimal_placement, and so do the API responses the queries cache served;
# candidates you promote from the map land in $XDG_DATA_HOME/gmaps_optimal_placement.
gmaps_optimal_placement serve --categories examples/categories --locations examples/locations --open

# Name an axis as a file rather than a directory to narrow it; name both and the pairing is built
# before the server starts, which is how a scripted run never meets the picker.
gmaps_optimal_placement serve -c examples/categories/cleaning_FR.json -l examples/locations/Lyon.nix

# A category's keywords are weighed by their search volume over the country, so the provider's
# credentials (below) are needed before any competitor is scored.

# The last row of the category field is "no category". Select it to see the city from the
# statistical grid only. This map has no demand layer and no competitors, and it uses no Google Maps calls.

# Chart the monthly search volume of each query group. Set the credentials of the provider first:
# GOOGLE_ADS_{DEVELOPER_TOKEN,CLIENT_ID,CLIENT_SECRET,REFRESH_TOKEN,CUSTOMER_ID}, or
# DATAFORSEO_{LOGIN,PASSWORD}. Each axis is picked through fzf.
gmaps_optimal_placement searches -c examples/categories -l examples/locations

# Behind the Service-Arb conductor: no page, and no axis — the conductor's map hands in each
# category and area it wants studied, over the trades and the country bases.
PANEL_ASSERTION_KEYS=... gmaps_optimal_placement serve --behind-conductor --trades examples/trades --countries examples/countries

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
nix run .#searches -- -c examples/categories/cleaning_FR.json
nix run .#help
```
