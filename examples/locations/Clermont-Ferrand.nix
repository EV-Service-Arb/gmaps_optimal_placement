/* What every Clermont-Ferrand study shares regardless of trade: the frame and the premises worth asking
   about. The rest is [../countries/FR.nix]. */
import ../countries/FR.nix
  // {
  area = {
    # Clermont-Ferrand agglomeration, generous: Riom (N) to Issoire-ward (S), Volvic (W) to Lezoux-ward (E).
    kind = "city";
    viewport = { south = 45.55; north = 45.95; west = 2.90; east = 3.40; };
    margin_m = 0;
  };

  # A premises, not a business: whichever trade moves into it, it is the same unit on the same street.
  candidate = [
    { name = "VifNet"; at = [ 45.77616 3.06373 ]; }
  ];
}
