```sh
# What the conductor bought is read off a copy of prod's panel.db, taken by site_conductor's
#   GITOPS_DIR=~/s/ev/gitops DEVOPS_DIR=~/s/ev/devops nix run ~/s/ev/_sa/site_conductor#prod-restore -- panel-data/panel.db=<path>
# and taken again for what prod bought since. Nothing is bought here.
#   queries_cache = "<path>";
# The GUI draws on Google Maps.
#   maps_key = "...";

# Refit the ranking model over every ordering the work dir holds.
gmaps_optimal_placement fit -l examples/locations

# Show the schema of a country's base, or of a location.
gmaps_optimal_placement schema base

# Find the towns in a country that are worth a study. Count what a cadastre draws, per commune, and
# divide by a column of the grid. Towns below the floor are not counted, and not downloaded.
gmaps_optimal_placement misc france pool --per ind --floor 2000
```

From a checkout:

```sh
nix run .#help
```
