# Changelog

All notable changes to Rust AutoHedge will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Constants module (`src/constants.rs`) for magic number elimination
- Custom error types (`src/error.rs`) with `thiserror` integration
- Comprehensive documentation (README.md, CLAUDE_MEMORY.md)
- `.env.example` template for easy setup
- Documentation index (`docs/INDEX.md`)

### Changed
- Organized documentation into `docs/` folder structure
- All 287 tests passing

## [1.0.0] - 2026-01-03

### Added

#### Core Features
- Multi-exchange support (Alpaca, Binance, Coinbase, Kraken)
- High-frequency trading (HFT) strategy with edge detection
- LLM-powered market analysis (OpenAI GPT integration)
- Real-time WebSocket market data streaming
- Event-driven architecture with event bus
- REST API for trading control (`/start`, `/stop`, `/stats`)
- Trade reporting system (JSONL logs)
- Keep-alive service for free hosting platforms

#### Position Management
- Automatic take-profit and stop-loss orders
- Orphaned position detection and auto-fix
- Position synchronization on startup
- Exit order recreation with retry logic
- Smart quantity validation (handles partial fills)

#### Risk Management
- Per-symbol stop-loss and take-profit configuration
- Position size limits
- Account balance protection (95% safety margin)
- Rate limiting (4 orders/sec per symbol)
- Maximum retry attempts (3 per position)

#### Performance
- Lock-free rate limiting with DashMap
- Optimized order execution (<100ms latency)
- Concurrent position tracking
- Real-time market data processing

### Fixed

#### Critical Bugs
- **Infinite Retry Loop**: Added rate limiting (30s delay) and max attempts (3) to prevent positions from retrying indefinitely
- **Orphaned Positions**: System now auto-recreates exit orders when limit sells are cancelled or expired
- **Quantity Mismatches**: Extracts actual filled quantity from exchange API, handles partial fills correctly
- **Position Not Found**: Automatically cleans up positions that don't exist on exchange
- **Rate Limiter Bug**: Fixed `.elapsed()` bug that was permanently blocking orders after first order
- **Retry on Error**: Added fresh holdings verification on 403 insufficient balance errors

#### Position Management
- Exit orders now recreated when cancelled/expired
- Positions synced from exchange get exit orders immediately
- Restart handling for existing positions (all positions protected within 3-5 seconds)
- Stop-loss cancellation now properly triggers market sell

#### Order Management
- Limit sells set at correct prices (above limit buys)
- Order expiration handling (configurable in config.yaml)
- Proper filled quantity tracking in pending orders

### Technical Improvements

#### Architecture
- Event bus for decoupled component communication
- Trait-based exchange abstraction
- Async/await throughout with Tokio runtime
- DashMap for lock-free concurrent access (rate limiter)
- WebSocket streaming for real-time data

#### Testing
- 287 comprehensive tests
- Unit tests for all core components
- Integration tests for workflows
- Position tracker tests (20 tests)
- Rate limiter tests with exact timing verification

#### Code Quality
- Structured logging with tracing
- Constants module to eliminate magic numbers
- Custom error types for better error handling
- Helper functions in tests to reduce duplication
- Extensive documentation and comments

### Configuration

#### Environment Variables
- `EXCHANGE`: Primary exchange (alpaca, binance, coinbase, kraken)
- `TRADING_MODE`: crypto or stocks (Alpaca)
- `ALPACA_API_KEY`, `ALPACA_SECRET_KEY`, `ALPACA_BASE_URL`
- `OPENAI_API_KEY`: Optional for LLM features
- `KEEP_ALIVE_URL`: Optional for free hosting

#### Trading Parameters (config.yaml)
- `symbols`: List of symbols to trade
- `defaults`: TP/SL percentages, position sizes, order amounts
- `rate_limit_ms`: Milliseconds between orders (250ms = 4/sec)
- `hft`: HFT strategy parameters (edge, spread, lookback)
- `llm`: LLM integration settings
- `symbol_overrides`: Per-symbol parameter overrides

### Documentation

#### User Documentation
- Comprehensive README with features, setup, and usage
- User guide with detailed instructions
- Position management guide
- Troubleshooting section

#### Developer Documentation
- CLAUDE_MEMORY.md: Complete implementation reference for AI assistants
- Technical design document
- HFT performance documentation
- Refactoring plan (23 high-impact improvements identified)

#### Fix Documentation
- 9 detailed bug fix documents in `docs/fixes/`
- Each includes problem, root cause, solution, and testing
- Quick reference guides for common issues

### Known Issues

- WebSocket does not auto-reconnect on disconnect (manual restart required)
- PositionTracker still uses `Arc<Mutex<HashMap>>` (DashMap migration in progress)
- Some code still uses `Box<dyn Error>` instead of typed errors
- Test files have some code duplication (helper functions being added)

### Deployment

#### Supported Platforms
- Local development (macOS, Linux, Windows)
- Docker containers
- Railway
- Render
- Any platform supporting Rust binaries

#### System Requirements
- Rust 1.70+
- ~50MB memory
- <5% CPU on modern hardware
- Network connection for WebSocket and REST APIs

---

## Version History

### Version Numbering

We use [Semantic Versioning](https://semver.org/):
- **MAJOR**: Incompatible API changes
- **MINOR**: New functionality (backwards compatible)
- **PATCH**: Bug fixes (backwards compatible)

### Release Process

1. Update version in `Cargo.toml`
2. Update this CHANGELOG.md
3. Run all tests: `cargo test`
4. Build release: `cargo build --release`
5. Tag release: `git tag -a v1.0.0 -m "Release 1.0.0"`
6. Push tags: `git push origin v1.0.0`

---

## Future Roadmap

### Planned Features (v1.1.0)
- WebSocket auto-reconnection with exponential backoff
- Position cache to reduce API calls (80% reduction)
- Enhanced reporting dashboard
- More exchange integrations
- Backtesting framework

### Refactoring (v1.2.0)
- Migrate PositionTracker to DashMap
- Split position_monitor.rs into 4 modules (860 → 200 lines each)
- Split strategy.rs into HFT/LLM modules
- Extract WebSocket protocol handlers
- Apply typed errors throughout

### Performance (v1.3.0)
- Reduce API calls from ~10/sec to ~2/sec
- Optimize hot paths
- Add benchmarking suite
- Profile and optimize memory usage

---

## Upgrade Guide

### From Pre-1.0 to 1.0.0

**Breaking Changes**: None (initial release)

**New Required Fields in PositionInfo**:
If you have custom code creating `PositionInfo`, add:
```rust
last_recreate_attempt: None,
recreate_attempts: 0,
```

**Configuration**:
- Review and update `config.yaml` with new fields
- Check `.env.example` for new environment variables
- Verify `rate_limit_ms` setting (default: 250)

**Testing**:
1. Test with paper trading account first
2. Monitor logs for 24 hours
3. Verify position management working correctly
4. Check that orphaned positions are auto-fixed

---

## Support & Contributing

- **Issues**: Report bugs on GitHub Issues
- **Discussions**: Ask questions on GitHub Discussions
- **Contributing**: See CONTRIBUTING.md (coming soon)
- **Security**: Report vulnerabilities privately

---

**Status**: Production Ready ✅  
**Tests**: 287/287 Passing ✅  
**Documentation**: Complete ✅  
**Last Updated**: January 3, 2026

