# Order Minimum Value Fix

## Problem
```
[FAILED] Order Submission: Failed to place order: 
Object {"code": Number(40310000), "message": String("cost basis must be >= minimal amount of order 10")}
```

Alpaca Markets requires all orders to have a minimum value of **$10**. The system was generating orders with quantities that resulted in a total value less than this minimum.

## Root Cause
The execution engine only checked if orders exceeded the maximum limit but didn't enforce a minimum order value. When the LLM agent suggested small quantities (e.g., 0.1 units of a $5 crypto), the total order value would be $0.50, which is below Alpaca's $10 minimum.

## Solution Implemented

### 1. Added Minimum Order Value Validation
**File: `src/services/execution.rs`**

Added logic to:
- Calculate the estimated order value before submission
- Check if the value is below the minimum threshold
- Automatically adjust the quantity upward to meet the minimum
- Ensure price data is available before placing orders

```rust
// Check if estimated_price is available
if estimated_price == 0.0 {
    error!("Cannot estimate price. No market data available.");
    return;
}

// Ensure minimum order value
if estimated_value < config.min_order_amount {
    order.qty = config.min_order_amount / estimated_price;
    // Recalculate estimated_value
}

// Ensure maximum order value
if estimated_value > config.max_order_amount {
    order.qty = config.max_order_amount / estimated_price;
}
```

### 2. Made Minimum Order Amount Configurable
**File: `src/config.rs`**

Added `min_order_amount` field to `AppConfig`:
- Default value: **$10.00** (Alpaca's requirement)
- Can be overridden via `MIN_ORDER_AMOUNT` environment variable
- Ensures compliance with broker requirements

### 3. Updated Configuration Files

**`.env` and `.env.example`:**
```dotenv
# Minimum dollar amount per trade (Alpaca requires $10 minimum)
MIN_ORDER_AMOUNT=10.0
# Maximum dollar amount per trade (Risk Manager)
MAX_ORDER_AMOUNT=100.0
```

## Order Value Validation Flow

1. **LLM suggests quantity** → e.g., 0.5 units
2. **Get current market price** → e.g., $15.00
3. **Calculate order value** → 0.5 × $15.00 = $7.50
4. **Check minimum** → $7.50 < $10.00 ❌
5. **Adjust quantity** → $10.00 ÷ $15.00 = 0.6667 units
6. **Recalculate value** → 0.6667 × $15.00 = $10.00 ✅
7. **Check maximum** → $10.00 < $100.00 ✅
8. **Submit order** → 0.6667 units @ $15.00

## Benefits

✅ **Prevents order rejection** - All orders meet Alpaca's $10 minimum
✅ **Automatic adjustment** - No manual intervention needed
✅ **Configurable** - Can adjust limits via environment variables
✅ **Better logging** - Shows quantity adjustments and estimated values
✅ **Price validation** - Won't submit orders without market data

## Example Log Output

```
⚠️ [RISK] Order value $5.50 is below minimum $10.00. Adjusting quantity.
⚠️ [RISK] Quantity increased to 1.81818182 (value: $10.00)
🚀 [ORDER] Submitting: buy 1.81818182 DOGE/USD (Est. Value: $10.00)
✅ [SUCCESS] Order Placed: {...}
```

## Testing Recommendations

1. **Small value orders** - Test with low-priced assets (DOGE, SHIB)
2. **Edge cases** - Test orders very close to $10 threshold
3. **Price unavailable** - Verify behavior when no market data exists
4. **Both limits** - Test min and max enforcement together

## Configuration Options

| Variable | Default | Description |
|----------|---------|-------------|
| `MIN_ORDER_AMOUNT` | 10.0 | Minimum order value in USD (Alpaca requirement) |
| `MAX_ORDER_AMOUNT` | 100.0 | Maximum order value in USD (risk limit) |

## Related Files Modified

- ✅ `src/services/execution.rs` - Added min/max validation logic
- ✅ `src/config.rs` - Added min_order_amount field
- ✅ `.env` - Added MIN_ORDER_AMOUNT configuration
- ✅ `.env.example` - Added MIN_ORDER_AMOUNT documentation

## Alpaca Order Requirements

For reference, Alpaca's order requirements:
- **Minimum order value**: $10.00 USD
- **Crypto time_in_force**: "gtc" (Good Till Canceled)
- **Stock time_in_force**: "day" or "gtc"
- **Order types**: "market", "limit", "stop", "stop_limit"

Both requirements are now properly handled in the execution engine.

