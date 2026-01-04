# Rust AutoHedge - Automated Cryptocurrency Trading System

[![Build Status](https://img.shields.io/badge/build-passing-brightgreen)]()
[![Tests](https://img.shields.io/badge/tests-287%20passing-brightgreen)]()
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange)]()

A high-performance, event-driven automated cryptocurrency trading system built in Rust with support for multiple exchanges (Alpaca, Binance, Coinbase, Kraken).

## 🚀 Features

### Core Trading
- **Multi-Exchange Support**: Alpaca (crypto/stocks), Binance, Coinbase, Kraken
- **High-Frequency Trading (HFT)**: 4 orders/second per symbol with intelligent rate limiting
- **Smart Position Management**: Automatic take-profit and stop-loss orders
- **Real-Time Market Data**: WebSocket streaming from all supported exchanges
- **Event-Driven Architecture**: Reactive system using event bus pattern

### Strategies
- **Micro-Trading Strategy**: Capitalizes on 1% volatility with small incremental trades
- **LLM-Powered Analysis**: OpenAI GPT integration for market analysis (optional)
- **Edge Detection**: Identifies profitable entry points using basis point calculations
- **Spread Analysis**: Monitors bid-ask spreads for optimal execution

### Risk Management
- **Per-Symbol Stop-Loss**: Configurable percentage-based stop losses
- **Take-Profit Limits**: Automatic profit-taking at target levels
- **Position Size Limits**: Maximum position size per symbol
- **Account Balance Protection**: 95% buying power safety margin
- **Rate Limiting**: Prevents API spam and exchange bans

### Advanced Features
- **Orphaned Position Detection**: Automatically fixes positions without exit orders
- **Failed Order Retry Logic**: Smart retry with exponential backoff
- **Position Synchronization**: Syncs with exchange on startup
- **Trade Reporting**: JSONL logs with comprehensive trade history
- **Keep-Alive Service**: Prevents free hosting services from sleeping

## 📋 Prerequisites

- **Rust**: 1.70 or higher
- **Exchange Account**: Paper or live trading account (Alpaca recommended for testing)
- **API Keys**: Exchange API key and secret
- **OpenAI API Key** (optional): For LLM-powered analysis

## 🔧 Installation

### 1. Clone Repository

```bash
git clone https://github.com/yourusername/rust-autohedge.git
cd rust-autohedge
```

### 2. Install Dependencies

```bash
cargo build --release
```

### 3. Configuration

Create a `.env` file in the project root:

```env
# Exchange Configuration
EXCHANGE=alpaca
TRADING_MODE=crypto

# Alpaca API (Paper Trading)
ALPACA_API_KEY=your_alpaca_key_here
ALPACA_SECRET_KEY=your_alpaca_secret_here
ALPACA_BASE_URL=https://paper-api.alpaca.markets

# OpenAI (Optional for LLM features)
OPENAI_API_KEY=your_openai_key_here

# Keep-Alive (Optional for Railway/Render deployment)
KEEP_ALIVE_URL=https://your-app.railway.app
```

Create `config.yaml` for trading parameters:

```yaml
# Exchange and Trading Mode
exchange: alpaca
trading_mode: crypto  # or 'stocks'

# Symbols to Trade
symbols:
  - BTC/USD
  - ETH/USD
  - SOL/USD
  - DOGE/USD
  - SHIB/USD
  - PEPE/USD

# Default Trading Parameters
defaults:
  take_profit_pct: 1.0      # 1% profit target
  stop_loss_pct: 0.5        # 0.5% stop loss
  max_position_size: 100.0  # Max USD per position
  order_amount: 100.0       # USD per order
  limit_order_expiration_days: 90

# Rate Limiting
rate_limit_ms: 250  # 250ms between orders (4/sec per symbol)

# Monitoring
history_limit: 50
chatter_level: normal  # 'verbose' for detailed logs

# HFT Strategy Parameters
hft:
  enabled: true
  min_edge_bps: 5           # Minimum 0.05% edge
  max_spread_bps: 50        # Maximum 0.5% spread
  lookback_periods: 10      # Price history to analyze

# LLM Integration (Optional)
llm:
  enabled: false
  model: gpt-4
  max_concurrent: 2
  queue_size: 10

# Symbol-Specific Overrides (Optional)
symbol_overrides:
  "BTC/USD":
    take_profit_pct: 2.0
    stop_loss_pct: 1.0
    max_position_size: 200.0
```

### 4. Environment Setup Example

Create `.env.example` for reference:

```env
# Exchange Configuration
EXCHANGE=alpaca
TRADING_MODE=crypto

# Alpaca Credentials (Paper Trading)
ALPACA_API_KEY=PK...
ALPACA_SECRET_KEY=...
ALPACA_BASE_URL=https://paper-api.alpaca.markets

# Binance (Optional)
BINANCE_API_KEY=
BINANCE_SECRET_KEY=

# Coinbase (Optional)
COINBASE_API_KEY=
COINBASE_SECRET_KEY=

# Kraken (Optional)
KRAKEN_API_KEY=
KRAKEN_SECRET_KEY=

# OpenAI (Optional for LLM)
OPENAI_API_KEY=sk-...

# Keep-Alive Service (Optional)
KEEP_ALIVE_URL=https://your-app.railway.app
```

## 🏃 Running the Application

### Development Mode

```bash
# Build and run
cargo run

# Run with logging
RUST_LOG=info cargo run

# Run tests
cargo test

# Run specific test
cargo test test_name
```

### Production Mode

```bash
# Build optimized binary
cargo build --release

# Run
./target/release/rust-autohedge

# Run in background
nohup ./target/release/rust-autohedge > autohedge.log 2>&1 &

# Check status
tail -f autohedge.log
```

### Docker Deployment

```bash
# Build image
docker build -t rust-autohedge .

# Run container
docker run -d \
  --name autohedge \
  --env-file .env \
  -v $(pwd)/config.yaml:/app/config.yaml \
  -v $(pwd)/data:/app/data \
  rust-autohedge

# View logs
docker logs -f autohedge
```

## 🌐 API Endpoints

The application exposes a REST API on `http://localhost:3000`:

### Trading Control

```bash
# Start trading
curl -X POST http://localhost:3000/start

# Stop trading
curl -X POST http://localhost:3000/stop

# Get status
curl http://localhost:3000/stats
```

### Health Check

```bash
# Ping endpoint
curl http://localhost:3000/ping
```

### Response Examples

**Start Trading**:
```json
{
  "status": "started"
}
```

**Status Check**:
```json
{
  "open_positions": {
    "BTC/USD": {
      "entry_price": 50000.0,
      "qty": 0.002,
      "take_profit": 50500.0,
      "stop_loss": 49750.0
    }
  },
  "pending_orders": 3,
  "trades_today": 24
}
```

## 📊 Monitoring

### Log Levels

Set `RUST_LOG` environment variable:

```bash
RUST_LOG=error     # Errors only
RUST_LOG=warn      # Warnings and errors
RUST_LOG=info      # Info, warnings, errors (default)
RUST_LOG=debug     # Detailed debugging
RUST_LOG=trace     # Maximum verbosity
```

### Trade Reports

Trade data is logged to `./data/trades.jsonl`:

```json
{"timestamp":"2026-01-03T12:00:00Z","symbol":"BTC/USD","side":"buy","price":50000.0,"qty":0.002,"pnl":null}
{"timestamp":"2026-01-03T12:05:00Z","symbol":"BTC/USD","side":"sell","price":50500.0,"qty":0.002,"pnl":1.0}
```

### Key Metrics

Watch logs for these indicators:

- **✅ Order filled** - Successful order execution
- **🔄 Recreating exit order** - Self-healing position management
- **⚠️ Rate limited** - Approaching exchange limits (normal)
- **❌ Order rejected** - Failed order (check logs for reason)

## 🏗️ Architecture

### System Components

```
┌─────────────────────────────────────────────────────┐
│                   REST API (Axum)                   │
│              /start  /stop  /stats  /ping           │
└────────────────────┬────────────────────────────────┘
                     │
┌────────────────────┴────────────────────────────────┐
│                   Event Bus                         │
│         (Market Data, Signals, Execution)           │
└──┬───────────┬────────────┬───────────┬────────────┘
   │           │            │           │
   ▼           ▼            ▼           ▼
┌──────┐  ┌─────────┐  ┌─────────┐  ┌──────────┐
│WebSocket│ │Strategy │  │Execution│  │Position  │
│Service│  │Engine   │  │Service  │  │Monitor   │
└───┬──┘  └────┬────┘  └────┬────┘  └─────┬────┘
    │          │            │             │
    │          │            │             │
    ▼          ▼            ▼             ▼
┌────────────────────────────────────────────────┐
│            Exchange APIs (Alpaca, etc)         │
└────────────────────────────────────────────────┘
```

### Data Flow

1. **Market Data** → WebSocket → Event Bus → MarketStore
2. **Strategy** → Analyzes quotes → Generates signals → Event Bus
3. **Execution** → Receives signals → Places orders → Exchange
4. **Position Monitor** → Tracks positions → Manages exits → Exchange

## 🔒 Security Best Practices

1. **API Keys**: Never commit `.env` file to version control
2. **Paper Trading**: Always test with paper trading accounts first
3. **Position Limits**: Set reasonable `max_position_size` limits
4. **Stop Losses**: Always configure stop-loss percentages
5. **Monitoring**: Watch logs for unexpected behavior
6. **Rate Limits**: Respect exchange rate limits to avoid bans

## 🐛 Troubleshooting

### Common Issues

**Issue**: "Insufficient balance" errors
- **Solution**: Check `max_position_size` and available buying power
- **Doc**: See `docs/fixes/QUANTITY_MISMATCH_FIX.md`

**Issue**: "Rate limit exceeded (429)"
- **Solution**: Increase `rate_limit_ms` in config.yaml
- **Current**: 250ms = 4 orders/sec per symbol

**Issue**: "Position not found on exchange"
- **Solution**: System auto-cleans orphaned positions (normal behavior)
- **Doc**: See `docs/fixes/POSITION_NOT_FOUND_FIX.md`

**Issue**: Positions without exit orders
- **Solution**: System auto-recreates exit orders (self-healing)
- **Doc**: See `docs/fixes/ORPHANED_POSITION_FIX.md`

### Debug Mode

```bash
# Enable detailed logging
RUST_LOG=debug cargo run

# Check specific module
RUST_LOG=rust_autohedge::services::execution=debug cargo run

# Output to file
cargo run 2>&1 | tee debug.log
```

## 📚 Documentation

- **[Claude Memory](./CLAUDE_MEMORY.md)** - Complete implementation reference for AI assistants
- **[User Guide](./docs/guides/USER_GUIDE.md)** - Detailed usage instructions
- **[Technical Design](./docs/TECHNICAL_DESIGN.md)** - Architecture documentation
- **[Refactoring Plan](./docs/REFACTORING_PLAN.md)** - Code improvement roadmap

### Fix Documentation

- [Quantity Mismatch Fix](./docs/fixes/QUANTITY_MISMATCH_FIX.md)
- [Orphaned Position Fix](./docs/fixes/ORPHANED_POSITION_FIX.md)
- [Position Not Found Fix](./docs/fixes/POSITION_NOT_FOUND_FIX.md)
- [Retry on Error Fix](./docs/fixes/RETRY_ON_ERROR_FIX.md)
- [Infinite Loop Fix](./docs/fixes/INFINITE_LOOP_COMPLETE_SUMMARY.md)

## 🧪 Testing

```bash
# Run all tests (287 tests)
cargo test

# Run specific test suite
cargo test position_monitor
cargo test execution
cargo test strategy

# Run with output
cargo test -- --nocapture

# Run integration tests
cargo test --test integration_tests
```

## 🚀 Deployment

### Railway

```bash
# Install Railway CLI
npm install -g @railway/cli

# Login
railway login

# Create project
railway init

# Add environment variables
railway variables set ALPACA_API_KEY=your_key

# Deploy
railway up
```

### Render

1. Connect GitHub repository
2. Set environment variables in dashboard
3. Build command: `cargo build --release`
4. Start command: `./target/release/rust-autohedge`

## 📈 Performance

- **Throughput**: 4 orders/second per symbol
- **Latency**: <100ms order placement
- **Concurrent Symbols**: 6+ simultaneously
- **Memory**: ~50MB typical usage
- **CPU**: <5% on modern hardware

## 🤝 Contributing

1. Fork the repository
2. Create feature branch (`git checkout -b feature/amazing-feature`)
3. Commit changes (`git commit -m 'Add amazing feature'`)
4. Push to branch (`git push origin feature/amazing-feature`)
5. Open Pull Request

## 📝 License

MIT License - see LICENSE file for details

## ⚠️ Disclaimer

This software is for educational purposes only. Cryptocurrency trading carries significant risk. Never trade with money you can't afford to lose. The authors are not responsible for any financial losses incurred using this software.

## 📞 Support

- **Issues**: [GitHub Issues](https://github.com/yourusername/rust-autohedge/issues)
- **Discussions**: [GitHub Discussions](https://github.com/yourusername/rust-autohedge/discussions)
- **Documentation**: See `CLAUDE_MEMORY.md` for comprehensive technical reference

## 🙏 Acknowledgments

- Built with [Tokio](https://tokio.rs/) async runtime
- WebSocket support via [tokio-tungstenite](https://github.com/snapview/tokio-tungstenite)
- REST API with [Axum](https://github.com/tokio-rs/axum)
- LLM integration via [async-openai](https://github.com/64bit/async-openai)

---

**Status**: ✅ Production Ready | 287 Tests Passing | Active Development

**Last Updated**: January 2026

