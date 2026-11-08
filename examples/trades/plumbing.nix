/* Plumbing and heating, over whichever city is handed in. */
loc:
{
  inherit (loc) area grid candidate country;

  poi = {
    source = "google_places";
    queries = [
      "plombier"
      "plomberie"
      "plombier chauffagiste"
      "chauffagiste"
      "dépannage plomberie"
      "débouchage canalisation"
      "installateur sanitaire"
      "installation chauffage"
    ];

    # A plumber is direct competition; a handyman who will take the job if asked is not found first.
    # First match wins, so this stays a list.
    tier = [
      {
        name = "plumber";
        weight = 1.0;
        match = "plombier|plomberie|sanitaire|chauffagiste|chauffage|chaudière|chaudiere|canalisation|débouchage|debouchage|assainissement";
        types = [ "plumber" ];
      }
      {
        name = "general";
        weight = 0.3;
        match = "multiservice|multi-service|multi service|dépannage|depannage|artisan|rénovation|renovation|entreprise générale|entreprise generale|tous corps d'état|bâtiment|batiment";
      }
    ];

    # The same queries surface the supply side: showrooms, wholesalers and pool shops sell the parts
    # rather than turn up at the house.
    drop = {
      match = "magasin|showroom|fourniture|grossiste|négoce|negoce|leroy merlin|castorama|brico|point.p|cedeo|piscine|spa\\b";
      types = [ "hardware_store" "home_goods_store" "store" ];
    };
  };

  # A flat's failing riser goes to whoever the syndic already contracts, so only its interior counts.
  # `log_*` partition `men` by construction period, and `men_prop` is the owner-occupier subset.
  column = loc.column ++ [
    { name = "homes"; expr = "men_mais + men_coll * 0.5"; }
    { name = "old_share"; expr = "(log_av45 + log_45_70) / max(men, 1)"; }
    { name = "owner_share"; expr = "men_prop / max(men, 1)"; }
  ];

  # A blocked drain is not a discretionary purchase, so income barely moves the count of callouts —
  # it moves the ticket, which this map does not model. Hence the far flatter exponent than detailing.
  model = {
    demand = "homes * (0.6 + old_share) * (0.5 + owner_share) * (max(nv, 4000) / 22000) ^ 0.4";
    # The plumber drives to the customer, not the other way round.
    lambda_m = 5000;
  };

  # Not `poi.queries`: those name the trade the way the trade names itself, which is exactly the
  # variation the name coefficient cannot be fitted on. "chauffage" is what the customer types.
  rank = {
    nodes = 32;
    term = [
      { text = "plombier"; weight = 1.0; }
      { text = "chauffage"; weight = 0.7; }
      { text = "dépannage plomberie"; weight = 0.4; }
    ];
  };

  layer = loc.layer ++ [
    {
      name = "Addressable dwellings";
      expr = "homes";
      note = "Houses in full, flats at half.";
    }
    {
      name = "Pre-1970 stock";
      expr = "log_av45 + log_45_70";
      note = "Lead, steel and first-generation copper. Where the failures are.";
    }
    {
      name = "Owner-occupiers";
      expr = "men_prop";
      note = "The household that pays the invoice itself, rather than passing it to a landlord.";
    }
  ];

  # Nobody searches "chauffagiste" and everybody searches "chauffage" (see niches/plumbing/Aquafix),
  # so the heating group is seeded on the trade word and matched on the layman's.
  searches = loc.searches // {
    group = [
      {
        name = "plomberie";
        seed = [ "plombier" "plombier chauffagiste" ];
        match = "plombier|plomberie|sanitaire|fuite|canalisation|débouchage|debouchage|chauffe-eau|chauffe eau|\\bwc\\b|robinet";
        drop = "emploi|salaire|formation|stage|jobs|apprentissage";
      }
      {
        name = "chauffage";
        seed = [ "chauffagiste" "installation chauffage" ];
        match = "chauffage|chaudière|chaudiere|chauffagiste|pompe à chaleur|pompe a chaleur|\\bpac\\b|radiateur";
        drop = "emploi|salaire|formation|stage|jobs|apprentissage";
      }
      {
        name = "urgence";
        seed = [ "dépannage plomberie" "plombier urgence" ];
        match = "urgence|urgent|dépannage|depannage|24h|24/24|nuit|dimanche";
      }
    ];
  };
}
