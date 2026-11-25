The binary `gmaps_optimal_placement`: the GUI, refit and cross-validate the ranking model,
reconnoitre a country. Its [`Held`](gmaps_optimal_placement::Held) is a copy of prod's
`panel.db` (settings' `queries_cache`, its path; site_conductor's `nix run .#prod-restore --
panel-data/panel.db=<path>` takes it): it reads what the conductor bought, as of when each was
fetched. A study run in the GUI is bought by the conductor (settings' `conductor`, its origin; signed
in to in the browser, once), and the copy then takes those answers of it; nothing is bought here.

```sh
gmaps_optimal_placement view     [-c <category>] --port <p> --site <dir> [-o] # the GUI; --spend-real-quota-i-am-certain: a run asks nothing first, the panel pulls and refreshes
gmaps_optimal_placement fit      [-c <category>]                         # refit rank::COEF
gmaps_optimal_placement strength [-c <category>]                         # cross-validate the entrants
gmaps_optimal_placement misc <country> <tally>                           # which towns are worth a study
gmaps_optimal_placement schema base
```

The studies are two axes' product — each area the work dir's definitions are over, under each setting
of each category they carry in its country — and every one of those definitions, each once. `view`
offers the same axes, the categories extended by none, and calls an area by its slug: two different
areas of one slug are refused.

It is the app the library is named for: its settings, their environment and the work dir are named
`gmaps_optimal_placement` (`build.rs`).
