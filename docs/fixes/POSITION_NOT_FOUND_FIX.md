# Position Not Found Error - Fixed

## The Errors You Saw

```
❌ [MONITOR] Position DOGE/USD not found in exchange holdings - cannot retry
❌ [MONITOR] Position PEPE/USD not found in exchange holdings - cannot retry
```

## Root Cause

The system was tracking positions that **no longer existed on the exchange**. This happens when:

1. **Position was closed externally** (via Alpaca dashboard, mobile app, etc.)
2. **Stop-loss triggered on exchange side** (not through our system)
3. **Market sell executed elsewhere** (another bot, API call)
4. **Account liquidation** (rare, but possible)

When trying to recreate an exit order, the retry logic would:
1. Check if position exists on exchange ✅
2. If not found → ERROR and give up ❌
3. Position stays in tracker forever (orphaned) ❌

## Solution Implemented

Now when a position is not found on exchange, we **clean it up automatically**:

### 1. Early Detection (Before Order Placement)

```rust
// When recreating limit sell order
let (actual_qty, position_exists) = match exchange.get_positions().await {
    Ok(positions) => {
        if let Some(pos) = positions.iter().find(|p| p.symbol == position.symbol) {
            (pos.qty, true)  // Found it
        } else {
            warn!("Position {} not found on exchange - likely already closed", symbol);
            (0.0, false)  // Not found
        }
    }
};

// If position doesn't exist, clean up and return
if !position_exists {
    tracker.remove_position(&position.symbol);
    info!("🧹 Cleaned up tracked position {} (not on exchange)", symbol);
    return;  // Don't try to place order
}
```

### 2. Retry Logic Cleanup (After Order Failure)

```rust
// If order fails with insufficient balance, verify holdings
match exchange.get_positions().await {
    Ok(positions) => {
        if let Some(pos) = positions.iter().find(|p| p.symbol == position.symbol) {
            // Found it - retry with correct quantity
        } else {
            // Not found - clean up
            warn!("Position {} not found - likely already closed", symbol);
            tracker.remove_position(&symbol);
            info!("🧹 Cleaned up tracked position {}", symbol);
        }
    }
}
```

## What Happens Now

### Scenario: Position Closed Externally

```
Step 1: System tracks position DOGE/USD (100 coins)
        Limit sell order cancelled or expired

Step 2: User manually closes position on Alpaca dashboard
        Exchange: DOGE/USD position = 0
        System tracker: DOGE/USD position = 100 (stale!)

Step 3: System tries to recreate limit sell order
        Verification call: get_positions() from exchange
        Result: DOGE/USD not in list

Step 4: ⚠️ Detect position missing
        "Position DOGE/USD not found on exchange - likely already closed"

Step 5: 🧹 Clean up
        Remove DOGE/USD from tracker
        "Cleaned up tracked position DOGE/USD (not on exchange)"
        No error, no retry attempt

Step 6: ✅ Clean state
        System tracker now matches exchange reality
```

### Scenario: Stop-Loss Triggered Externally

```
Step 1: System has position PEPE/USD with exit order

Step 2: Price drops suddenly, exchange triggers stop-loss
        Position closed on exchange side
        Our exit order cancelled

Step 3: System tries to recreate exit order
        Verification: PEPE/USD not found on exchange

Step 4: 🧹 Auto-cleanup
        "Position PEPE/USD not found - likely already closed"
        Remove from tracker

Step 5: ✅ No error, clean state
```

## Logging Changes

### Before Fix (Errors)
```
❌ [MONITOR] Position DOGE/USD not found in exchange holdings - cannot retry
❌ [MONITOR] Position PEPE/USD not found in exchange holdings - cannot retry
[Positions remain orphaned in tracker ❌]
```

### After Fix (Clean)
```
⚠️ [MONITOR] Position DOGE/USD not found on exchange during verification - likely already closed
🧹 [MONITOR] Cleaned up tracked position DOGE/USD (not on exchange)

⚠️ [MONITOR] Position PEPE/USD not found on exchange - likely already closed
🧹 [MONITOR] Cleaned up tracked position PEPE/USD (not on exchange)
[Tracker is now in sync with exchange ✅]
```

## Benefits

| Issue | Before | After |
|-------|--------|-------|
| Orphaned positions | Stay forever | Auto-cleaned |
| Error messages | ❌ Scary errors | ⚠️ Informational warnings |
| Tracker accuracy | Drifts from reality | Stays in sync |
| Manual cleanup | Required | Automatic |
| System stability | Can accumulate issues | Self-healing |

## Edge Cases Handled

### 1. Position Exists But Qty Changed

```rust
if (actual_qty - position.qty).abs() > 0.000001 {
    // Update to match exchange
    corrected_pos.qty = actual_qty;
    tracker.add_position(corrected_pos);
}
```

**Result**: Updates quantity, continues to place order ✅

### 2. Position Has Zero Quantity

```rust
if final_qty <= 0.0 {
    warn!("Position {} has zero/negative quantity - removing", symbol);
    tracker.remove_position(&symbol);
    return;
}
```

**Result**: Cleaned up, no order placed ✅

### 3. API Error During Verification

```rust
Err(e) => {
    warn!("Could not verify holdings: {} (using tracked qty)", e);
    (position.qty, true)  // Assume exists on API error
}
```

**Result**: Falls back to tracked quantity, attempts order ✅

### 4. Position Found in Retry But Not Initial Check

```rust
// Initial check: Position not found → cleaned up → return
// Retry never happens (already returned)
```

**Result**: Consistent behavior, no confusion ✅

## Files Modified

**`src/services/position_monitor.rs`**:

1. **`recreate_limit_sell_order()`** (lines 660-720)
   - Check if position exists on exchange
   - Return early if not found (with cleanup)
   - Change zero-qty error to warning + cleanup

2. **Retry logic** (lines 750-850)
   - If position not found in retry → cleanup instead of error
   - Log informational message, not error

## Testing

All 287 tests pass:
```bash
cargo test
# test result: ok. 287 passed
```

## Real-World Scenarios

### Scenario 1: Manual Trading
You manually close a position on Alpaca while bot is running:
- ✅ Bot detects position is gone
- ✅ Removes from tracker
- ✅ No errors, continues normally

### Scenario 2: Stop-Loss on Exchange
Exchange closes position due to stop-loss:
- ✅ Bot detects closure
- ✅ Cleans up tracker
- ✅ Ready for new trades

### Scenario 3: Multiple Bots
Another bot closes position:
- ✅ This bot detects it's gone
- ✅ Syncs state
- ✅ No conflicts

### Scenario 4: Account Changes
You close account or transfer assets:
- ✅ Bot detects positions missing
- ✅ Cleans up all tracked positions
- ✅ Fresh state

## Verification

### Check Tracker State

Before fix, tracker could have "ghost positions":
```bash
curl http://localhost:3000/stats | jq .open_positions
# Might show positions that don't exist on exchange
```

After fix, tracker stays accurate:
```bash
curl http://localhost:3000/stats | jq .open_positions
# Always matches Alpaca dashboard
```

### Monitor Logs

Watch for cleanup messages:
```bash
tail -f rust-autohedge.log | grep "Cleaned up"
# 🧹 [MONITOR] Cleaned up tracked position DOGE/USD (not on exchange)
```

This is **normal and healthy** - it means the system is self-healing!

## Summary

✅ **Detects positions missing from exchange**  
✅ **Automatically removes orphaned positions from tracker**  
✅ **Changes errors to informational warnings**  
✅ **Two-stage detection (initial + retry)**  
✅ **Self-healing system**  
✅ **No manual cleanup needed**  
✅ **All tests passing**  

**Before**: Errors when positions missing, tracker drifts from reality  
**After**: Auto-cleanup, tracker stays in sync, informational logging  

The "Position not found" error is now **completely eliminated** and replaced with automatic cleanup! 🎉

