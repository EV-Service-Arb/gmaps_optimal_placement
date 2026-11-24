#:schema gmaps_optimal_placement.module.nix
{
  age = {
    communes = "1y";
    inventory = "1w";
    ordering = "1y";
  };
  maps_key = { env = "GOOGLE_MAPS_KEY"; };
  # a copy of prod's panel.db: site_conductor's `nix run .#prod-restore -- panel-data/panel.db=<this>`
  queries_cache = "/home/v/.local/share/gmaps_optimal_placement/panel.db";
}
