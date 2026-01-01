# High-Frequency Trading Performance Optimizations

## Overview
This document describes the performance optimizations made to enable frequent small trades capturing 1% volatility.

## Key Changes

### 1. Account Balance Caching (`execution_utils.rs`)
**Problem**: Every order was calling `get_account()` API, causing rate limiting and latency.

**Solution**: `AccountCache` caches balance and refreshes every N seconds (configurable via `account_cache_secs`).

```yaml
micro_trade:
  account_cache_secs: 15  # Refresh balance every 15 seconds
```

### 2. Rate Limiting (`execution_utils.rs`)
**Problem**: Spamming orders too quickly can cause API errors and slippage.

**Solution**: `RateLimiter` enforces minimum interval between orders per symbol.

```yaml
micro_trade:
  min_order_interval_ms: 250  # 250ms minimum between orders
```

### 3. Aggressive Limit Pricing (`execution_utils.rs`)
**Problem**: Limit orders at mid-price often don't fill quickly enough.

**Solution**: `aggressive_limit_price()` moves limit price slightly toward market (configurable aggression in bps).

```yaml
micro_trade:
  aggression_bps: 10.0  # 10 basis points toward market for faster fills
```

### 4. Limit Order Expiration
**Problem**: GTC (Good Till Canceled) limit orders can stay open forever, leading to stale orders executing at unfavorable prices later.

**Solution**: Use Day time-in-force so orders expire at end of day.

```yaml
micro_trade:
  limit_orders_expire_daily: true  # Orders expire at end of day
```

### 5. Fast Execution Engine (`execution_fast.rs`)
**Problem**: LLM agent calls add 1-5 seconds latency per order.

**Solution**: When `strategy_mode: "hft"`, uses a fast execution path that:
- Skips LLM entirely (unless `use_llm_filter: true`)
- Uses cached account balance
- Pre-computes order sizes
- Submits limit orders directly

### 6. LLM Filter Option (Optional Enhancement)

**What it does**: When `use_llm_filter: true`, the system asks the LLM a quick yes/no question before executing HFT trades.

**Benefits of LLM Filter**:
- 🧠 **Smarter filtering**: LLM can reject trades during unusual market conditions
- 📊 **Context awareness**: Can factor in recent news or patterns it knows
- ⚡ **Faster than full LLM**: Only asks "yes/no" instead of full order generation
- 🛡️ **Safety net**: Adds a layer of validation for edge cases

**Tradeoffs**:
- ⏱️ **Added latency**: ~100-500ms per trade (vs 0ms without LLM)
- 💰 **Potential missed trades**: LLM might reject valid opportunities
- 🔄 **API dependency**: Requires working LLM connection

**When to use LLM filter**:
- Markets with high news sensitivity
- Lower-frequency trading (seconds, not milliseconds)
- When you want AI oversight on automated decisions

**When NOT to use LLM filter**:
- True high-frequency trading (need sub-100ms execution)
- During LLM service outages
- When every millisecond counts

```yaml
micro_trade:
  use_llm_filter: false  # Set to true to enable LLM validation
```

### 7. Smart Order Sizing (`execution_utils.rs`)
**Problem**: Fixed order sizes don't adapt to available balance.

**Solution**: `compute_order_sizing()` calculates optimal size based on:
- Target % of balance per trade
- Min/max order limits from config
- Available buying power (with 5% buffer for fees)

```yaml
micro_trade:
  target_balance_pct: 0.05  # 5% of balance per trade
```

### 8. Position Deduplication
**Problem**: Multiple buy signals for same symbol stack up positions.

**Solution**: Fast execution checks `tracker.has_position()` and pending orders before submitting.

### 9. Enhanced Reporting (`reporting.rs`)
New metrics for micro-trading analysis:
- **Win rate**: Percentage of profitable trades
- **Profit factor**: Total profit / Total loss
- **Trades per hour**: Trading frequency
- **Avg profit per trade**: Mean P&L per closed trade
- **Runtime tracking**: Minutes since start

## API Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /stats` | Quick performance stats (JSON) |
| `GET /report` | Full trade history and summary |

## Configuration

### Recommended HFT Settings for 1% Volatility

```yaml
strategy_mode: "hft"

hft:
  evaluate_every_quotes: 1      # Evaluate on every quote
  min_edge_bps: 1.0             # Low threshold for more trades
  take_profit_bps: 10.0         # 0.1% take profit
  stop_loss_bps: 5.0            # 0.05% stop loss
  max_spread_bps: 50.0          # Trade even with 0.5% spread

micro_trade:
  target_balance_pct: 0.05      # 5% of balance per trade
  aggression_bps: 10.0          # Aggressive limit pricing
  min_order_interval_ms: 250    # Max 4 orders/second
  account_cache_secs: 15        # Cache balance for 15s
  use_llm_filter: false         # Set true for LLM validation
  limit_orders_expire_daily: true  # Orders expire at end of day

defaults:
  take_profit_pct: 0.1          # 0.1% TP (matches HFT)
  stop_loss_pct: 0.05           # 0.05% SL (matches HFT)
  min_order_amount: 1.0         # $1 minimum order
  max_order_amount: 100.0       # $100 maximum order
```

## Output Files

| File | Description |
|------|-------------|
| `./data/trades.jsonl` | Full event log (JSONL format) |
| `./data/trade_summary.json` | Complete performance summary |
| `./data/trade_stats.json` | Quick stats for monitoring |

## Performance Comparison

| Metric | Before | After |
|--------|--------|-------|
| API calls per order | 2-3 | 0-1 |
| Order latency | 1-5s (LLM) | <100ms |
| Max orders/sec | ~1 | ~4 |
| Position dedup | None | Automatic |
| Balance check | Every order | Cached |
| Order expiration | GTC (never) | End of day |

