## Sources

| `grid.source` | Area | Cell | Publishes |
|---|---|---|---|
| `insee_filosofi_200m` | France | 200 m | households, housing type, standard of living |
| `geostat_1km` | EU | 1 km | census counts by age, sex, employment, origin |

## Expressions

`column`, `layer` and the category's demand are arithmetic over the column names the chosen
source publishes, plus `max`, `min`, `clamp`, `sqrt` and `pow`. A name that does not exist is an
error at load, against the source's own column list.
