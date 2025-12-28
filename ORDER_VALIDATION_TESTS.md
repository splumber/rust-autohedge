# Order Value Validation Tests

## Test Cases for Minimum Order Value

### Test 1: Order Below Minimum
**Input:**
- Quantity: 0.5
- Price: $5.00
- Min: $10.00
- Max: $100.00

**Expected:**
- Initial Value: $2.50 ❌
- Adjusted Quantity: 2.0 (10.00 / 5.00)
- Final Value: $10.00 ✅

### Test 2: Order Above Maximum
**Input:**
- Quantity: 20.0
- Price: $10.00
- Min: $10.00
- Max: $100.00

**Expected:**
- Initial Value: $200.00 ❌
- Adjusted Quantity: 10.0 (100.00 / 10.00)
- Final Value: $100.00 ✅

### Test 3: Order Within Range
**Input:**
- Quantity: 5.0
- Price: $10.00
- Min: $10.00
- Max: $100.00

**Expected:**
- Initial Value: $50.00 ✅
- No adjustment needed
- Final Value: $50.00 ✅

### Test 4: Crypto (Low Price, High Volume)
**Input:**
- Symbol: DOGE/USD
- Quantity: 100
- Price: $0.08
- Min: $10.00
- Max: $100.00

**Expected:**
- Initial Value: $8.00 ❌
- Adjusted Quantity: 125.0 (10.00 / 0.08)
- Final Value: $10.00 ✅

### Test 5: No Price Data
**Input:**
- Quantity: 10.0
- Price: $0.00 (no data)
- Min: $10.00
- Max: $100.00

**Expected:**
- Error: "Cannot estimate price for SYMBOL. No market data available."
- Order rejected ❌

### Test 6: Edge Case - Exactly Minimum
**Input:**
- Quantity: 1.0
- Price: $10.00
- Min: $10.00
- Max: $100.00

**Expected:**
- Initial Value: $10.00 ✅
- No adjustment needed
- Final Value: $10.00 ✅

### Test 7: Edge Case - Exactly Maximum
**Input:**
- Quantity: 10.0
- Price: $10.00
- Min: $10.00
- Max: $100.00

**Expected:**
- Initial Value: $100.00 ✅
- No adjustment needed
- Final Value: $100.00 ✅

### Test 8: Very Low Priced Crypto (SHIB)
**Input:**
- Symbol: SHIB/USD
- Quantity: 1000
- Price: $0.00001
- Min: $10.00
- Max: $100.00

**Expected:**
- Initial Value: $0.01 ❌
- Adjusted Quantity: 1,000,000 (10.00 / 0.00001)
- Final Value: $10.00 ✅

## Manual Testing Steps

1. **Start the system** with crypto mode enabled
2. **Monitor logs** for order submissions
3. **Check for warnings**:
   ```
   ⚠️ [RISK] Order value $X.XX is below minimum $10.00. Adjusting quantity.
   ⚠️ [RISK] Quantity increased to X.XXXXXXXX (value: $10.XX)
   ```
4. **Verify order success**:
   ```
   ✅ [SUCCESS] Order Placed: {...}
   ```

## Expected Behavior

- ✅ All orders have value ≥ $10.00
- ✅ All orders have value ≤ $100.00 (or configured max)
- ✅ Quantities are automatically adjusted
- ✅ Detailed logging shows adjustments
- ✅ Orders without price data are rejected
- ✅ No order rejection errors from Alpaca

## Verification Commands

Check recent orders via Alpaca API:
```bash
curl -H "APCA-API-KEY-ID: YOUR_KEY" \
     -H "APCA-API-SECRET-KEY: YOUR_SECRET" \
     https://paper-api.alpaca.markets/v2/orders?status=all&limit=10
```

Check account positions:
```bash
curl -H "APCA-API-KEY-ID: YOUR_KEY" \
     -H "APCA-API-SECRET-KEY: YOUR_SECRET" \
     https://paper-api.alpaca.markets/v2/positions
```

