CLI and the study document.

Every command but `serve` takes the same two axes: the conductor's categories, as the definitions it handed in
carry them (the latest of each, `-c <key>` to name one), and `-l` locations (`*.nix`, a directory or
a single file), over `--countries` country bases looked up by name. `fit` and `strength` take
the whole product, with every definition the conductor ever handed in.

```sh
gmaps_optimal_placement serve    --countries <dir> [--port 8731]           # the conductor's studies
gmaps_optimal_placement fit      [-c <category>] -l <locations>              # refit rank::COEF over every ordering cached
gmaps_optimal_placement strength [-c <category>] -l <locations>              # cross-validate the entrants
gmaps_optimal_placement schema                    # JSON schema for the study document
```

The study is [`Study`]: an `sa_core::placement::Definition` — an area and a category — laid over the
area's base, evaluating to a grid source, its derived columns and layers, and the category's demand
columns and layers after them. [`load`] is where they meet. The study is named
`<trade>·<CC>·<slug>`. `column` adds derived columns in list order, so a study can name a quantity
its source only implies. Every expression is evaluated before any of it reaches the map — an
expression naming a column that does not exist is an error, not a zero.

`load` takes no category as well, and then an area is already a study: [`Study::build`] never
reaches the sweep, and what comes back is the grid's own layers under the area's own name.

The category's weighted keywords are the terms competitors are scored on. The probe's plan asks them
from equal-demand strata; `fit` estimates `gmaps_optimal_placement_core::rank::COEF` over every
ordering in the work dir — the whole product at once, because how Google ranks is one mechanism and
one pairing on its own is about fifty orderings. A pairing nobody harvested contributes nothing and
costs nothing: `fit` reads what the conductor's queries cache holds, so it cannot buy what it is
missing. [`fit::Fit::check`] is what refuses a fit whose implied catchment nothing observed.

[`Payload`] is what `serve` hands the conductor, whose map draws it. It carries evaluated per-cell
values only: competitor pressure, underserved demand, capture scores and the top-N sweep are
`gmaps_optimal_placement_core::model`, recomputed in the browser because they move with the sliders.

No key is held here: the conductor buys every search. Archives and API responses cache under
`$XDG_CACHE_HOME/gmaps_optimal_placement`.
