/* Car detailing and washing, over whichever city is handed in. */
loc:
{
  inherit (loc) area grid candidate country column layer;

  poi = {
    source = "google_places";
    queries = [
      "lavage auto"
      "nettoyage voiture"
      "car detailing"
      "car wash"
      "station de lavage"
      "lavage auto sans eau"
      "esthétique automobile"
      "covering carrosserie"
    ];

    # A dedicated detailer is direct competition; a rollover wash at a hypermarket is weak.
    # First match wins, so this stays a list.
    tier = [
      {
        name = "detail";
        weight = 1.0;
        match = "detail|esthétique|esthetique|nettoyage|clean|polissage|céramique|ceramique|\\bppf\\b|covering|renovation auto|rénovation auto|carrosserie";
      }
      {
        name = "wash";
        weight = 0.35;
        match = "lavage|lav'|lav’|\\blav\\b|wash|karcher|kärcher|rouleau";
        types = [ "car_wash" ];
      }
    ];

    # "lav'" and "wash" also match clothes laundromats and a bike-wash point
    drop = {
      match = "laverie|vélo|velo|\\bbike\\b|pressing|blanchisserie";
      types = [ "laundry" ];
    };
  };

  model = {
    demand = "cars * (max(nv, 4000) / 22000) ^ 1.6";
    lambda_m = 2000;
  };

  # Not `poi.queries`: those name the trade the way the trade names itself, which is exactly the
  # variation the name coefficient cannot be fitted on. Nobody outside the trade says "esthétique
  # automobile"; they say they want the car cleaned.
  rank = {
    nodes = 32;
    term = [
      { text = "lavage auto"; weight = 1.0; }
      { text = "nettoyage intérieur voiture"; weight = 0.6; }
      { text = "polissage carrosserie"; weight = 0.4; }
    ];
  };

  # `match` / `drop` are the same vocabulary as `poi.tier` / `poi.drop`: case-insensitive regex,
  # `drop` checked first. The expansion is fuzzy and drags in intent that is not demand.
  searches = loc.searches // {
    group = [
      {
        name = "detailing";
        seed = [ "car detailing" "esthétique automobile" ];
        match = "detail|esthetique|esthétique|polissage|céramique|ceramique|\\bppf\\b|covering";
        drop = "emploi|salaire|formation|stage|jobs";
      }
      {
        name = "lavage";
        seed = [ "lavage auto" "station de lavage" ];
        match = "lavage|wash|karcher|kärcher";
      }
    ];
  };
}
