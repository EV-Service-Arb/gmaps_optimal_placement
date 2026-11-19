/* What every Lyon study shares regardless of trade: the frame and the premises worth asking
   about. The rest is [../countries/FR.nix]. */
import ../countries/FR.nix
  // {
  area = {
    # The Métropole and the ring that commutes into it: Neuville (N) to Givors (S), the Monts du
    # Lyonnais (W) to Pont-de-Chéruy-ward (E). Villefranche-sur-Saône is left out — it is 30 km up
    # the Saône and buys from its own high street.
    kind = "city";
    viewport = { south = 45.62; north = 45.90; west = 4.70; east = 5.02; };
    margin_m = 0;
  };

  # A premises, not a business: whichever trade moves into it, it is the same unit on the same street.
  candidate = [ ];
}
