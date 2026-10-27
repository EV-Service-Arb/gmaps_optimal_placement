/* What every Angers study shares regardless of trade: the frame, the location Google resolves search
   volume against, and the premises worth asking about. The rest is [../_France.nix]. */
let france = import ../_France.nix; in
france
  // {
  area = {
    # Angers Loire Métropole and its commuter ring: Avrillé-ward (N) to Les Ponts-de-Cé and the Loire (S),
    # Beaucouzé and Saint-Jean-de-Linières (W) to Trélazé and Saint-Barthélemy-d'Anjou (E).
    bbox = { lat = [ 47.38 47.58 ]; lon = [ (-0.75) (-0.40) ]; };
    center = [ 47.4784 (-0.5632) ];
    zoom = 12;
  };

  searches = france.searches // { place = "Angers,Pays de la Loire,France"; };

  # A premises, not a business: whichever trade moves into it, it is the same unit on the same street.
  candidate = [ ];
}
