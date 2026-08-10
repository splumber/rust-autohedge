# Test Suite Documentation

## Overview

This document describes the comprehensive test suite for the AutoHedge trading system.

## Test Summary

| Test Suite | Tests | Status |
|------------|-------|--------|
| Library Unit Tests | 160 | ✅ Pass |
| Binary Unit Tests | 105 | ✅ Pass |
| Integration Tests | 11 | ✅ Pass |
| **Total** | **276** | ✅ **All Pass** |

## Test Files

### Unit Tests (src/)

| File | Module | Description | Tests |
|------|--------|-------------|-------|
| `bus_tests.rs` | `bus` | EventBus pub/sub messaging | 7 |
| `events_tests.rs` | `events` | Event types (Quote, Trade, Signal, Order, Execution) | 25 |
| `config_tests.rs` | `config` | Configuration parsing and validation | 30 |
| `data/store_tests.rs` | `data::store` | MarketStore (quotes, trades, bars, news) | 15 |
| `exchange/types_tests.rs` | `exchange::types` | Exchange types and symbol conversion | 35 |
| `services/execution_utils_tests.rs` | `services::execution_utils` | Order sizing, aggressive pricing, rate limiting | 25 |
| `services/reporting_tests.rs` | `services::reporting` | Trade reporting and performance metrics | 22 |
| `services/position_monitor_tests.rs` | `services::position_monitor` | Position and pending order tracking | 25 |

### Integration Tests (tests/)

| File | Description | Tests |
|------|-------------|-------|
| `integration_tests.rs` | End-to-end flow testing | 11 |

## Test Categories

### 1. EventBus Tests (`bus_tests.rs`)
- Basic creation and subscription
- Event publishing and receiving
- Multiple subscribers
- All event types (Market, Signal, Order, Execution)
- Channel capacity handling

### 2. Events Tests (`events_tests.rs`)
- MarketEvent::Quote construction and fields
- MarketEvent::Trade construction and fields
- AnalysisSignal for buy/sell/no_trade
- OrderRequest for market/limit orders
- ExecutionReport for filled/new/rejected
- Event enum pattern matching
- Clone and Debug traits

### 3. Configuration Tests (`config_tests.rs`)
- MicroTradeConfig defaults and deserialization
- Defaults struct parsing
- SymbolConfig with partial overrides
- HftConfig, HybridConfig, LlmConfig
- Exchange configs (Alpaca, Binance, Coinbase, Kraken)
- `get_symbol_params()` with overrides
- BPS to percent conversion
- Sensible default validation

### 4. MarketStore Tests (`store_tests.rs`)
- Quote storage and retrieval
- Trade storage and retrieval
- Bar storage and retrieval
- History limit enforcement
- Multiple symbol handling
- News storage
- Concurrent access safety

### 5. Exchange Types Tests (`types_tests.rs`)
- AccountSummary fields and serialization
- Position creation
- Side enum (Buy/Sell) serialization
- OrderType enum (Market/Limit)
- TimeInForce enum (Day/GTC/IOC)
- PlaceOrderRequest construction
- OrderAck parsing
- ExchangeCapabilities
- Symbol conversion (Coinbase, Kraken, Binance)

### 6. Execution Utils Tests (`execution_utils_tests.rs`)
- `compute_order_sizing()` basic cases
- Min/max order clamping
- 95% buying power cap
- Can't afford minimum order
- Zero/negative price handling
- `aggressive_limit_price()` for buy/sell
- Zero aggression (mid price)
- High aggression capping
- `RateLimiter` first call, second call, after interval
- Concurrent rate limiting

### 7. Reporting Tests (`reporting_tests.rs`)
- PerformanceSummary defaults
- Computed stats (win rate, profit factor, trades/hour)
- All wins/all losses edge cases
- Runtime tracking
- ClosedTrade profit/loss
- OpenPosition tracking
- TradeLogEntry for buy/sell/rejected
- Serialization/deserialization
- Per-symbol tracking
- History and open positions

### 8. Position Monitor Tests (`position_monitor_tests.rs`)
- PositionTracker creation
- Add/get/remove positions
- Has position check
- Mark closing
- Position overwrite
- Pending order add/remove
- Multiple pending orders
- Update check time
- PositionInfo fields and clone
- PendingOrder fields and clone
- Concurrent position access
- Concurrent pending order access

### 9. Integration Tests (`integration_tests.rs`)
- Market data to signal flow
- Signal to order flow
- Order to execution flow
- Position tracking lifecycle
- Order sizing with position tracker
- Multi-symbol handling
- TP/SL calculation from entry price
- HFT edge calculation
- Spread calculation
- Concurrent event publishing
- Complete position lifecycle (pending → fill → TP → close)

## Running Tests

```bash
# Run all tests
cargo test

# Run with output
cargo test -- --nocapture

# Run specific test
cargo test test_name

# Run specific module
cargo test bus_tests

# Run only integration tests
cargo test --test integration_tests

# Run with verbose output
cargo test -- --show-output
```

## Test Coverage Areas

### ✅ Covered
- Event bus messaging
- All event types
- Configuration parsing
- Market data storage
- Exchange type serialization
- Order sizing calculations
- Aggressive limit pricing
- Rate limiting
- Trade reporting
- Performance metrics
- Position tracking
- Pending order management
- Symbol normalization
- TP/SL calculations
- HFT edge/spread calculations

### 🔄 Future Improvements
- Mock exchange API tests
- WebSocket message parsing tests
- LLM agent response parsing tests
- Strategy engine signal generation tests
- Risk engine approval tests
- End-to-end trading simulation tests

