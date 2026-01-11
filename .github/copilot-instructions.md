# Copilot Instructions for Rust AutoHedge

## Project Overview

Rust AutoHedge is a high-performance, event-driven automated cryptocurrency trading system built in Rust. It supports multiple exchanges (Alpaca, Binance, Coinbase, Kraken) and implements HFT (High-Frequency Trading) and hybrid strategies.

## Documentation Structure

All documentation should be organized in the `docs/` folder:

```
docs/
├── INDEX.md                    # Main documentation index
├── TECHNICAL_DESIGN.md         # System architecture and design
├── HFT_PERFORMANCE.md          # Performance optimization docs
├── fixes/                      # Bug fixes and solutions
│   ├── INFINITE_LOOP_*.md
│   ├── ORDER_*.md
│   ├── POSITION_*.md
│   ├── QUANTITY_MISMATCH_FIX.md
│   ├── RETRY_ON_ERROR_FIX.md
│   └── SELL_LOGIC_ANALYSIS.md
└── guides/                     # User and developer guides
    ├── USER_GUIDE.md
    ├── POSITION_MANAGEMENT_GUIDE.md
    ├── REFACTORING_*.md
    └── ...
```

## Code Style Guidelines

### Rust Conventions
- Use `cargo fmt` before committing
- Follow Rust 2021 edition idioms
- Use `async/await` for async operations
- Prefer `tracing` crate for logging (info!, warn!, error!)
- Use `thiserror` for error types

### Naming Conventions
- Structs: PascalCase (e.g., `PositionInfo`, `MarketStore`)
- Functions: snake_case (e.g., `compute_order_sizing`)
- Constants: SCREAMING_SNAKE_CASE (e.g., `MAX_RETRIES`)
- Config fields: snake_case in YAML, snake_case in Rust

### Architecture Patterns
- **Event-Driven**: Use `EventBus` for inter-component communication
- **Services**: Each major component is a service (Strategy, Execution, Risk, etc.)
- **Traits**: Use `TradingApi` trait for exchange abstraction
- **Config-Driven**: All parameters in `config.yaml`, parsed into typed structs

## Key Components

### Core Services (`src/services/`)
- `strategy.rs` - Trading strategy (HFT, LLM, Hybrid modes)
- `execution.rs` / `execution_fast.rs` - Order execution
- `position_monitor.rs` - Position tracking, TP/SL management
- `risk.rs` - Risk assessment
- `reporting.rs` - Trade logging and statistics

### Data Layer (`src/data/`)
- `store.rs` - Market data storage (quotes, trades, bars)
- `alpaca.rs` - Alpaca API client

### Exchange Abstraction (`src/exchange/`)
- `traits.rs` - `TradingApi` trait definition
- `alpaca.rs`, `binance.rs`, etc. - Exchange implementations
- `types.rs` - Common order/position types

## Configuration

Key config sections in `config.yaml`:
- `defaults` - TP/SL percentages, order limits
- `hft` - HFT strategy parameters (edge_bps, spread limits)
- `micro_trade` - Position sizing, rate limiting, trailing stops
- `symbol_overrides` - Per-symbol parameter overrides

## Testing

- Unit tests: In `*_tests.rs` files alongside source
- Integration tests: In `tests/integration_tests.rs`
- Run all tests: `cargo test`
- Run specific: `cargo test test_name`

## Common Tasks

### Adding a New Exchange
1. Create `src/exchange/newexchange.rs`
2. Implement `TradingApi` trait
3. Add to `src/exchange/factory.rs`
4. Add config section in `config.rs`

### Modifying Trading Strategy
1. Edit `src/services/strategy.rs`
2. For HFT: modify `evaluate_hft()` function
3. Update config parameters in `config.yaml`

### Adding New Config Parameters
1. Add field to appropriate struct in `src/config.rs`
2. Add `#[serde(default)]` with default function if optional
3. Update tests in `src/config_tests.rs`

## Profitability Optimizations

Key parameters for profitability:
- `take_profit_bps` / `stop_loss_bps` - Risk/reward ratio (aim for 2:1)
- `min_edge_bps` - Minimum momentum before trading
- `max_spread_bps` - Only trade liquid markets
- `min_order_interval_ms` - Prevent overtrading
- `crypto_time_in_force: "ioc"` - Avoid stale limit orders

