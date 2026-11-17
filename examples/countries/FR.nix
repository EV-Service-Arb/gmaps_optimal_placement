/* What every French study shares regardless of area or trade: the grid, how INSEE spells the
   numbers it publishes, and who prices its keywords. An area — a city file in [../locations/],
   or one the conductor hands in — takes this and adds its frame; a trade in [../trades/] is a
   function of that. */
{
  country = "FR";

  grid = {
    source = "insee_filosofi_200m";
    vintage = 2021;
  };

  # INSEE publishes the standard of living summed over individuals, not per head. It publishes no
  # motorisation at all at 200 m — that variable exists only at IRIS level — so `cars` is inferred;
  # see docs/ARCHITECTURE.md on what that does not know.
  column = [
    { name = "nv"; expr = "ind_snv / max(ind, 1)"; }
    { name = "cars"; expr = "men_mais * 1.55 + men_coll * 0.85"; }
  ];

  #Q: potentially harden this, so as to move out of the config, - I don't think this'll be changing
  layer = [
    {
      name = "Population";
      expr = "ind";
      note = "Raw head count per 200 m cell.";
    }
    {
      name = "Standard of living (€/yr)";
      expr = "nv";
      scale = "linear";
      note = "Mean disposable income per consumption unit. A per-person rate, not a density — thinly populated affluent suburbs light up.";
    }
    {
      name = "Estimated cars";
      expr = "cars";
      note = "Houses × 1.55 + flats × 0.85. Inferred, not measured.";
    }
    {
      name = "Households";
      expr = "men";
      note = "Fiscal households per cell.";
    }
  ];

  searches = {
    provider = "google_ads";
  };
}
