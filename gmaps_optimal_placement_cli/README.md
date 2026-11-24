The binary `gmaps_optimal_placement`: the GUI, refit and cross-validate the ranking model,
reconnoitre a country. Its [`Held`](gmaps_optimal_placement::Held) is a copy of prod's
`panel.db` (settings' `queries_cache`, its path; site_conductor's `nix run .#prod-restore --
panel-data/panel.db=<path>` takes it), opened read-only: it reads what the conductor bought, as of
when each was fetched, and buys nothing.

```sh
gmaps_optimal_placement view     [-c <category>] [-l <locations>] --port <p> --site <dir> # the GUI
gmaps_optimal_placement fit      [-c <category>] [-l <locations>] # refit rank::COEF
gmaps_optimal_placement strength [-c <category>] [-l <locations>] # cross-validate the entrants
gmaps_optimal_placement misc <country> <tally>                    # which towns are worth a study
gmaps_optimal_placement schema base|location
```

The studies are two axes' product — each location (`*.toml`: an area and its candidates) under each of
the conductor's categories in its country, each setting the work dir's definitions carry — and
every definition a host studied in the work dir, each once. `view` offers the same axes, the
locations extended by every area a host studied that no file has, and the categories by none.

It is the app the library is named for: its settings, their environment and the work dir are named
`gmaps_optimal_placement` (`build.rs`).
