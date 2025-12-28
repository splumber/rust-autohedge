# Position Management - Quick Reference

## How It Works

### Buy Order Placed ✅
```
1. Order executed
2. Position stored with: entry_price, stop_loss, take_profit, qty
3. Position Monitor starts watching (every 10s)
```

### Position Monitoring 👁️
```
Every 10 seconds:
- Get current price
- Check: current_price >= take_profit? → SELL
- Check: current_price <= stop_loss? → SELL
- Log status periodically
```

### Take Profit Hit 🎯
```
Current price >= take_profit
→ Generate SELL signal
→ Execute SELL order
→ Remove position from tracker
→ Profit realized!
```

### Stop Loss Hit 🛑
```
Current price <= stop_loss
→ Generate SELL signal
→ Execute SELL order
→ Remove position from tracker
→ Loss limited!
```

## Default Values

If Risk Agent doesn't provide values:
- **Stop Loss:** Entry price × 0.95 (-5%)
- **Take Profit:** Entry price × 1.10 (+10%)

## Risk Agent Guidelines

### Stop Loss
- **Stocks:** 3-7% below entry
- **Crypto:** 5-10% below entry (higher volatility)

### Take Profit
- **General:** 8-15% above entry
- **Risk/Reward:** Minimum 1.5:1 ratio

### Position Sizing
- **Maximum:** 5% of account cash
- **Minimum order:** $10
- **Maximum order:** $100 (configurable)

## Key Components

### PositionTracker
- Shared state between Execution Engine and Position Monitor
- Thread-safe (Arc<Mutex>)
- Stores: symbol, entry_price, qty, stop_loss, take_profit, entry_time

### Position Monitor
- Checks positions every 10 seconds
- Generates sell signals when targets hit
- Syncs with Alpaca on startup
- Automatic position cleanup

### Execution Engine
- **Buy orders:** Store position info with SL/TP
- **Sell orders:** Execute and remove from tracker
- Handles both automated and manual exits

## Log Messages to Watch

### Position Created
```
📊 [TRACKER] Added position: DOGE/USD @ $0.10000000 (SL: $0.09200000, TP: $0.11500000)
```

### Position Monitoring
```
📊 [MONITOR] DOGE/USD @ $0.10500000 (P/L: +5.00%, SL: $0.09200000, TP: $0.11500000)
```

### Take Profit
```
🎯 [MONITOR] TAKE PROFIT HIT for DOGE/USD!
   Entry: $0.10000000 → Current: $0.11500000 (P/L: +15.00%)
✅ [MONITOR] Exit signal published for DOGE/USD
🔻 [EXECUTION] Processing SELL order for DOGE/USD
✅ [SUCCESS] SELL Order Placed
📊 [TRACKER] Removed position: DOGE/USD
```

### Stop Loss
```
🛑 [MONITOR] STOP LOSS HIT for DOGE/USD!
   Entry: $0.10000000 → Current: $0.09000000 (P/L: -10.00%)
✅ [MONITOR] Exit signal published for DOGE/USD
🔻 [EXECUTION] Processing SELL order for DOGE/USD
✅ [SUCCESS] SELL Order Placed
📊 [TRACKER] Removed position: DOGE/USD
```

### Position Sync
```
🔄 [MONITOR] Syncing positions with Alpaca...
⚠️  [MONITOR] Added existing position DOGE/USD (defaults: SL -5%, TP +10%)
✅ [MONITOR] Position sync complete
```

## Configuration

### Check Interval
**File:** `src/services/position_monitor.rs`
```rust
check_interval_secs: 10  // Check every 10 seconds
```

**To adjust:**
- Faster (5s): More responsive, higher CPU
- Slower (30s): Less responsive, lower CPU

### Default Risk Parameters
**File:** `src/services/execution.rs`
```rust
let stop_loss = req.stop_loss.unwrap_or(estimated_price * 0.95);   // -5%
let take_profit = req.take_profit.unwrap_or(estimated_price * 1.10); // +10%
```

## Testing Checklist

- [ ] System opens position with correct SL/TP
- [ ] Position Monitor logs periodic status
- [ ] Take profit triggers sell at target price
- [ ] Stop loss triggers sell at stop price
- [ ] Position removed after sell
- [ ] Multiple positions tracked independently
- [ ] Existing Alpaca positions synced on startup

## Troubleshooting

### Position not closing at target
- Check logs for "TAKE PROFIT HIT" or "STOP LOSS HIT"
- Verify Position Monitor is running: "Position Monitor Started"
- Check current price vs targets in periodic logs

### Position not tracked
- Check for "Added position" log after buy order
- Verify Execution Engine initialized with tracker
- Check for errors in order execution

### Wrong SL/TP values
- Check Risk Agent output in logs
- Verify Risk Agent JSON parsing
- Check default values being applied

## Architecture

```
Buy Order
    ↓
Execution Engine → PositionTracker.add_position()
    ↓
Position Monitor (every 10s)
    ↓
Check price vs SL/TP
    ↓
Generate SELL signal if triggered
    ↓
Execution Engine → SELL order
    ↓
PositionTracker.remove_position()
    ↓
Position Closed ✅
```

## Best Practices

### For Day Trading
- Tighter stops: 3-5%
- Tighter targets: 5-8%
- Faster check interval: 5s

### For Swing Trading
- Standard stops: 5-7%
- Standard targets: 10-15%
- Normal check interval: 10s

### For Crypto
- Wider stops: 7-10%
- Wider targets: 15-20%
- Normal check interval: 10s

## Emergency Actions

### Manual Position Close
Via Alpaca dashboard or API:
```bash
curl -X DELETE \
  -H "APCA-API-KEY-ID: YOUR_KEY" \
  -H "APCA-API-SECRET-KEY: YOUR_SECRET" \
  https://paper-api.alpaca.markets/v2/positions/DOGE/USD
```

Position Monitor will detect closure on next check.

### Stop Position Monitor
Position Monitor runs automatically. To stop:
- Stop the trading system
- Positions remain in Alpaca but won't be monitored

---

## Status: ✅ FULLY OPERATIONAL

The system now has complete position lifecycle management with automated risk controls!

