```sh
# Every Google key is a slot of a billing account in accounting's google.nix, each read from the
# environment so it never lands in a file; ~/.config/gmaps_optimal_placement.nix imports the list whole.
# Places searches are bought on the accounts that provide `places`, in list order, moving on as each
# one's day is spent; the map loads with the first `browser` key; Keyword Planner uses the one account
# that provides `ads`. What is bought is counted against the account's `name`:
#   google = (import ~/s/ev/_sa/accounting/google.nix).accounts;
# and where what is bought is cached for everyone, so no machine buys the same query twice: the
# Service-Arb conductor's queries cache, or `local` for this machine's work dir alone:
#   queries_cache = "https://sa.evinvest.ltd/api/queries_cache";

# Serve the map. A study is a category (a trade in a country, as the Service-Arb conductor holds
# one: its Google categories, its keywords, the trade file that studies it) over a location. The
# categories are the ones the conductor handed in (the definitions under the work dir's
# `studies/`), so a work dir no conductor ever defined a study in offers locations alone. The page
# picks the pairing, with a field per axis: type in either, Enter picks its top row, and Load opens
# the pair as a tab over one map. Press `t` for one more. Bulk archives cache in
# $XDG_CACHE_HOME/gmaps_optimal_placement, and so do the API responses the queries cache served;
# candidates you promote from the map land in $XDG_DATA_HOME/gmaps_optimal_placement.
gmaps_optimal_placement serve --locations examples/locations --open

# Name a category by its key and a location as a file rather than a directory to narrow each; name
# both and the pairing is built before the server starts, which is how a scripted run never meets
# the picker.
gmaps_optimal_placement serve -c 'cleaning·FR (trade 2)' -l examples/locations/Lyon.nix

# A category's keywords are weighed by their search volume over the country, so the provider's
# credentials (below) are needed before any competitor is scored.

# The last row of the category field is "no category". Select it to see the city from the
# statistical grid only. This map has no demand layer and no competitors, and it uses no Google Maps calls.

# Chart the monthly search volume of each query group. Set the credentials of the provider first:
# an account providing `ads`, or
# DATAFORSEO_{LOGIN,PASSWORD}. Each axis is picked through fzf.
gmaps_optimal_placement searches -l examples/locations

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
nix run .#searches -- -c 'cleaning·FR (trade 2)'
nix run .#help
```
