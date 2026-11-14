/* What every Angers study shares regardless of trade: the frame, the location Google resolves search
   volume against, and the premises worth asking about. The rest is [../countries/FR.nix]. */
let france = import ../countries/FR.nix; in
france
  // {
  area = {
    # Angers Loire Métropole and its commuter ring: Avrillé-ward (N) to Les Ponts-de-Cé and the Loire (S),
    # Beaucouzé and Saint-Jean-de-Linières (W) to Trélazé and Saint-Barthélemy-d'Anjou (E).
    kind = "city";
    viewport = { south = 47.38; north = 47.58; west = (-0.75); east = (-0.40); };
    margin_m = 0;
  };

  searches = france.searches // { place = "Angers,Pays de la Loire,France"; };

  # A premises, not a business: whichever trade moves into it, it is the same unit on the same street.
  candidate = [ ];
}
