#:schema gmaps_optimal_placement.module.nix
{
  age = {
    communes = "1y";
    inventory = "1w";
    ordering = "1y";
  };
  sweep = { depth = 3; root = 3; };
  google_maps = [
    { key = { env = "GOOGLE_MAPS_KEY"; }; free = true; }
    { key = { env = "GOOGLE_MAPS_KEY_ICOS1"; }; free = true; }
  ];
}
