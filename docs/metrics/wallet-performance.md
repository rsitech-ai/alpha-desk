# Wallet performance

Cash flows are never trading profit. Time-weighted return links subperiod
returns at external deposit and withdrawal boundaries using exact decimal
arithmetic. These calculators are synthetic-fixture proven only.

Spec 15.3 sibling calculators consume only caller-supplied observed fields:

- Holding-time uses observed open and close protocol times. The median is an
  observed order statistic (no interpolated period). An empty sample fails
  closed. Close time is never inferred.
- Slippage uses observed fills versus an observed reference price, typically an
  order limit. Missing fills or missing reference prices withhold (`None`).
  Mid/mark prices are not invented.
- Concentration may include collateral and regime HHI when those series are
  supplied. Empty series are omitted, not invented.
- Entry and exit markouts evaluate only observed prices at labeled horizons.
  An empty point set withholds.
- Long/short beta is computed per market only when that market's return is
  observed. Missing returns withhold; a zero market return is unsupported.

Regime-split ledger performance and live fill capture are still withheld.
These calculators are not production-ready and do not close Stage gates.
