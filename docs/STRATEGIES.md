# Strategy Reference

Every strategy in this document is backtestable via `cargo run --bin portfolio --release
-- --mode <name> [flags]` (the realistic, $500-capital-constrained simulator - see
`src/bin/portfolio.rs`'s module doc for why this is the trustworthy backtest tool, not
`backtest.rs`/`pairs.rs`), except **market making**, which has its own binary
(`cargo run --bin market_maker --release`) because its execution model (quoting both sides
instead of crossing the spread on a signal) doesn't fit the shared engine.

**Read this first:** as of the most recent full investigation, *no* strategy below -
including the 8 original signals - has demonstrated genuine, out-of-sample, cost-adjusted
edge over simply buying and holding the same assets. See the project's running memory /
prior findings for the full validation history (walk-forward windows, $500 capital
constraints, buy-and-hold benchmarking). Every `portfolio` run self-reports its result next
to a buy-and-hold baseline computed from the same data - always read that comparison, not
just the raw return.

## Quick reference: all modes

| `--mode` value | What it does | Long/Short | Key config section |
|---|---|---|---|
| `momentum` (default) | Buy on raw upward momentum, net of fees/spread | Long | `hft` |
| `reversion` | Buy on a short-term dip | Long | `hft` |
| `rsi` | Buy when RSI < oversold | Long | (consts in `portfolio.rs`) |
| `bollinger` | Buy below lower Bollinger band | Long | (consts) |
| `ma_cross` | Buy on a golden cross (fast MA > slow MA) | Long | (consts) |
| `donchian` | Buy on breakout above prior N-bar high | Long | (consts) |
| `vwap` | Buy when price is below its own VWAP | Long | (consts) |
| `trend_momentum` | Momentum entries filtered by a longer-term trend | Long | (consts) |
| `combined` | Enter only when `--min-agree` of the 8 above agree | Long | n/a |
| `short_momentum` | Standalone short on breakdown momentum + trend filter | **Short** | `strategies.short_momentum` |
| `vol_squeeze` | Bollinger-bandwidth squeeze then upward breakout | Long | `strategies.vol_squeeze` |
| `orb` | Opening-range breakout | Long | `strategies.orb` |
| `basket_arb` | Long/short a symbol vs. its sector peer group | **Both** | `strategies.basket_arb` |
| `news_sentiment` | Keyword-scored headline sentiment via Alpaca News | Long | `strategies.news_sentiment` |
| `ml_signal` | Logistic regression trained on a pre-window, predicts live | Long | `strategies.ml_signal` |
| (separate binary) `market_maker` | Quotes both sides, profits from spread + inventory skew | Both | `strategies.market_maker` |

Every mode also accepts the shared flags: `--starting-cash`, `--days`, `--offset-days`,
`--asset-class stocks|crypto`, `--tp-pct`/`--sl-pct` (fixed TP/SL override), `--leverage N`
(margin, see below), `--exclude-weekdays mon,fri` (calendar filter, see below).

---

## New strategies added this round

### Standalone short strategy: `short_momentum`

The first strategy in this codebase to trade the short side as its own idea, not just as a
pairs-trading hedge leg. Sells short when price has dropped at least `hft.min_edge_bps`
over `hft.momentum_lookback` bars **and** sits below its own
`strategies.short_momentum.trend_period`-bar moving average (confirms a genuine downtrend,
filters out dips inside an uptrend). Requires shorting enabled on the account (confirmed
available on this project's paper account - level 3 options, shorting, 4x margin).

```yaml
strategies:
  short_momentum:
    trend_period: 50   # longer-horizon trend filter
```

```
cargo run --bin portfolio --release -- --mode short_momentum --asset-class stocks --days 60
```

### Volatility squeeze breakout: `vol_squeeze`

Buys when Bollinger bandwidth (`(2 * num_sd * stddev) / mean` - a price-scale-normalized
measure of how tight the bands are) has dropped below `squeeze_threshold` at any point in
the last `lookback_period` bars, then price breaks above its own upper Bollinger band. This
is a *continuation* bet (volatility contraction -> expansion, trade the breakout direction)
as opposed to `bollinger`'s *reversion* bet (buy a dip below the lower band).

```yaml
strategies:
  vol_squeeze:
    period: 20
    num_sd: 2.0
    squeeze_threshold: 0.01   # bandwidth as a fraction of price - widen for a more
                              # volatile universe (crypto), tighten for quiet mega-caps
    lookback_period: 20
```

### Opening-range breakout: `orb`

Tracks each symbol's high over the first `range_bars` bars of every trading session, then
buys a breakout above that range once established. A classic day-trading pattern, untested
anywhere else in this project's signal search.

```yaml
strategies:
  orb:
    range_bars: 15   # ~15 minutes on 1Min bars
```

### Margin-leveraged sizing: `--leverage N` (any mode, long positions only)

Applies to **any** `portfolio` mode via the `--leverage` flag (default from
`strategies.margin.leverage_multiplier`, capped by whatever your account's margin approval
actually allows - this project's paper account has 4x). Only a portion of each position's
notional (`notional / leverage`) is drawn from cash; the rest is modeled as borrowed, with a
rough daily interest accrual (`strategies.margin.annual_margin_rate_pct`) charged over the
holding period and repaid on exit.

**Scoped to long positions only.** Shorts already have their own "borrowed shares" mechanic
in this simulator (selling borrowed shares credits cash at entry, buying them back debits
cash at exit); stacking margin leverage on top of that would need a materially different
accounting model than longs, so `--leverage` is a no-op for short entries in every mode,
including `short_momentum` and the short side of `basket_arb`.

**This is a simplified model, not a real Alpaca margin schedule** - it doesn't model
maintenance-margin calls, Reg T requirements, or day-trading buying-power rules. Confirm
your account's actual margin rate on the Alpaca dashboard before treating a leveraged
backtest result as a deployable estimate.

```
cargo run --bin portfolio --release -- --mode momentum --asset-class stocks --leverage 3.0
```

### Basket/sector relative-value arbitrage: `basket_arb`

Long/short a **single symbol** against the average return of its sector peer group (see
`sector_group()` in `src/bin/portfolio.rs` - six groups: mega-cap tech, finance, healthcare,
consumer, industrial/energy, index ETFs, matching `config.yaml`'s curated stock universe).
Computes a rolling z-score of `(symbol's own return - basket's average return)`; enters long
when the symbol has underperformed its peers by `entry_z` standard deviations (bet on
catch-up), short when it has overperformed by the same margin (bet on reversion). Only the
single symbol is ever traded - no actual basket order is placed, so this is realistically
executable via a normal retail account (unlike market making below).

```yaml
strategies:
  basket_arb:
    z_period: 30    # rolling window (bars) for the z-score
    entry_z: 2.0
```

### News sentiment: `news_sentiment`

Keyword-based headline sentiment (see `src/services/sentiment.rs` - counts positive vs.
negative finance-domain keywords, normalized to roughly `[-1, 1]`) fetched from **Alpaca's
own News API** (`AlpacaClient::get_news`, `data.alpaca.markets/v1beta1/news`) - no separate
signup or API key needed, unlike PEAD's earnings-surprise data, which remains blocked on
obtaining a third-party key (Finnhub free tier is the recommended path if that's ever
pursued). Enters long when the most recent headline for a symbol, published strictly before
the current bar (never leaks future news into the backtest), scores at or above
`sentiment_threshold` within `lookback_hours`.

Keyword scoring is deliberately crude - it exists to backtest cheaply over hundreds of
thousands of bars, which running every headline through the project's local LLM (already
wired up in `src/llm.rs` for other purposes) would be far too slow for. If this signal shows
real backtested edge, upgrading to LLM-scored sentiment for *live* trading (only a handful
of fresh headlines per symbol per day, not a full backtest's worth) would be the natural
next step.

```yaml
strategies:
  news_sentiment:
    lookback_hours: 24
    sentiment_threshold: 0.3
```

### ML logistic-regression signal: `ml_signal`

Trains a simple logistic regression (pure Rust, no ML crate dependency - see
`src/services/ml_signal.rs`) on 5 features (RSI, momentum, VWAP deviation, trend deviation,
Bollinger z-score, all indicator functions already used elsewhere in this codebase) to
predict whether price rises by `label_threshold_pct` within `label_horizon_bars`. **Training
happens on a window strictly BEFORE the backtest window** (`train_days` immediately
preceding `--days`/`--offset-days`) - the model never sees backtest-window data during
training, maintaining the same train/test discipline this project's walk-forward validation
established for hand-coded signals. At backtest time, the trained model just predicts live
off the same features every other bar computes, no different from any other signal.

```yaml
strategies:
  ml_signal:
    train_days: 60
    label_horizon_bars: 15
    label_threshold_pct: 0.3
    learning_rate: 0.1
    epochs: 150
    predict_threshold: 0.55   # tune per universe/date range - if a run reports 0 trades,
                              # the model's predicted probabilities never cleared this bar
    row_stride: 3             # subsamples training rows to bound training time
```

```
cargo run --bin portfolio --release -- --mode ml_signal --asset-class stocks --days 30
```

### Calendar filter: `--exclude-weekdays`

Layered on top of **any** mode - skips new entries on the listed weekdays (comma-separated,
e.g. `mon,fri`). Existing open positions still manage their TP/SL normally on excluded days;
only new entries are blocked.

```
cargo run --bin portfolio --release -- --mode momentum --exclude-weekdays mon,fri
```

### PDT (Pattern Day Trader) enforcement: `--enforce-pdt`

Layered on top of **any** mode, stocks only. FINRA restricts margin accounts with equity
below `--pdt-equity-threshold` (default $25,000, the real regulatory number) to 3 day
trades (same-symbol open+close within one session) per rolling 5-trading-day window; a 4th
trips "pattern day trader" status and broadly restricts the account. Every backtest before
this flag existed implicitly assumed unlimited day-trading frequency - momentum alone
generates ~38 trades/day, which is not legal to run in a real sub-$25k margin account.

Modeled conservatively: once 3 day trades have occurred in the trailing window, ALL new
entries are blocked (not just ones that would turn out to be day trades) until the window
rolls forward - the only way an automated system can guarantee it never trips the flag,
since you can't know in advance whether today's entry will need to exit before the close.
Existing open positions still exit normally; only new entries are affected. Doesn't apply
to crypto (not a security under this rule) or a cash account (which has its own T+1/T+2
settlement-delay constraint instead, not modeled here).

```
cargo run --bin portfolio --release -- --mode momentum --asset-class stocks --enforce-pdt
```

### Approximated market making: `market_maker` (separate binary)

```
cargo run --bin market_maker --release -- --asset-class stocks --days 60
```

Quotes both a bid and an ask around each bar's reference price (offset by
`quote_offset_bps` plus the asset's assumed half-spread), profiting from the spread when
both sides fill, and skewing quotes based on accumulated inventory
(`inventory_limit_usd`, `skew_factor`) to avoid running an unbounded directional position.

```yaml
strategies:
  market_maker:
    quote_offset_bps: 5.0
    inventory_limit_usd: 200.0
    skew_factor: 0.5
```

**Important caveat, read before drawing any conclusion from this backtest's numbers:** true
market making is **not realistically executable via a retail Alpaca account.** Institutional
market makers get exchange queue priority and often maker rebates through exchange
membership or direct market access; a retail broker just routes your limit order for
execution with none of that privilege. This backtest approximates a fill whenever a bar's
high/low crosses your quote level, which substantially overstates real-world fill
probability (a real market maker reacts to quote changes in microseconds, not once per
1-minute bar, and has priority at the same price level). **Treat this as an educational
illustration of the mechanics, not a deployable strategy candidate.**

---

## Deferred: not built, and why

- **Directional options** (long calls/puts, earnings straddles) - backtesting these
  correctly needs historical options-chain pricing and implied volatility, which isn't
  reliably available through Alpaca's data API for backtesting at the depth needed. Building
  a synthetic Black-Scholes approximation instead of real historical option prices would
  produce numbers that look precise but aren't trustworthy - deliberately not built rather
  than shipping something misleading.
- **PEAD (post-earnings drift)** - real, well-documented edge, but needs third-party
  earnings-surprise data Alpaca doesn't provide. Blocked on obtaining a free Finnhub API key
  (60 calls/min free tier, no card required) - see prior research notes. Not something this
  session can self-provision.

## How every mode is configured, at a glance

1. **`config.yaml`'s `strategies:` section** - the primary way to tune any new strategy's
   parameters. Every sub-section has built-in defaults in `src/config.rs`, so you only need
   to override what you actually want to change; omitting the whole `strategies:` block is
   valid too.
2. **CLI flags on `portfolio`/`market_maker`** - for the handful of parameters worth
   sweeping from the command line without editing config: `--mode`, `--starting-cash`,
   `--days`, `--offset-days`, `--asset-class`, `--tp-pct`/`--sl-pct`, `--leverage`,
   `--exclude-weekdays`, `--min-agree` (combined mode only).
3. **Existing `hft:` config block** - still governs momentum/reversion/short_momentum's
   lookback, edge thresholds, and spread/volume gates, same as before this round of work.

Every `portfolio` run writes full results (trades, equity curve, buy-and-hold comparison) to
`data/portfolio_<asset_class>_<mode>_<cash>_<days>d_offset<N>.json` for further analysis.
