CLI and the study document.

Every command takes the same two axes: the conductor's categories, as the definitions it handed in
carry them (the latest of each, `-c <key>` to name one), and `-l` locations (`*.nix`, a directory or
a single file), over `-t` trades and `--countries` country bases looked up by name. A whole axis is
what the page picks out of, `fzf` picks out of, and `fit` takes all of — with every definition the
conductor ever handed in.

```sh
gmaps_optimal_placement serve    [-c <category>] -l <locations> [--port 8731] [--open]
gmaps_optimal_placement serve    --behind-conductor -t <trades> --countries <dir>
gmaps_optimal_placement searches [-c <category>] -l <locations> [-o searches.html]
gmaps_optimal_placement probe    [-c <category>] -l <locations> [--dry-run]  # ask Google, from equal-demand nodes
gmaps_optimal_placement fit      [-c <category>] -l <locations>              # refit rank::COEF over every ordering cached
gmaps_optimal_placement schema                    # JSON schema for the study document
```

The study is [`Study`]: an `sa_core::placement::Definition` — an area and a category — with the trade the
category names applied to the area, evaluating to a grid source, what a competitor is, and how demand
follows from the columns that source publishes. [`load`] is where they meet: the category's Google
categories become the sweep's queries and the first tier's Places type, and a trade that writes
either is refused. The study is named `<study>·<CC>·<slug>`. `column` adds derived columns in
list order, so a study can name a quantity its source only implies. Every expression is evaluated
before any of it reaches the map — an expression naming a column that does not exist is an error,
not a zero.

`load` takes no category as well, and then an area is already a study: `poi`, `model` and `rank` are
`None`, [`Study::build`] never reaches `poi::load`, and what comes back is the grid's own layers
under the area's own name. `serve` offers it as the last row of the category field.

An optional `searches` block names groups of queries. [`fold`] is the half of that command that is
ours: the provider expands the category's keywords semantically, each group's regex keeps or drops
each member of that one expansion, and the survivors
sum into one line of [`SearchPayload`] — with their own series alongside, so the sum can be audited.

The category's keywords are the terms the study is about, each worth its search volume over the
country, over the largest. `probe` asks them from the `rank.nodes` demand strata, `fit` estimates `gmaps_optimal_placement_core::rank::COEF` over every ordering in the work dir
— the whole product at once, because how Google ranks is one mechanism and one pairing on its own is
about fifty orderings. A pairing nobody harvested contributes nothing and costs nothing: `fit` reads
the work dir with no key, so it cannot buy what it is missing. [`fit::Fit::check`] is what refuses a
fit whose implied catchment nothing observed.

[`Payload`] is what `serve` hands the browser. It carries evaluated per-cell values only: competitor
pressure, underserved demand, capture scores and the top-N sweep are `gmaps_optimal_placement_core::model`,
recomputed in the page because they move with the sliders.

Google keys are slots of the config's `google` accounts (accounting's google.nix), read from the environment and never written to disk.
Archives and API responses cache under `$XDG_CACHE_HOME/gmaps_optimal_placement`.
