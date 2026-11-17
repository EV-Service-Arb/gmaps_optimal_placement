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
