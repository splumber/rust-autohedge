# autohedge Rust Algorithmic Trading Bot

[![CI](https://github.com/splumber/rust-autohedge/workflows/CI/badge.svg)](https://github.com/splumber/rust-autohedge/actions/workflows/ci.yml)
[![CodeQL](https://github.com/splumber/rust-autohedge/workflows/CodeQL%20Security%20Scan/badge.svg)](https://github.com/splumber/rust-autohedge/actions/workflows/codeql.yml)
[![Docker](https://github.com/splumber/rust-autohedge/workflows/Docker/badge.svg)](https://github.com/splumber/rust-autohedge/actions/workflows/docker.yml)

## Overview

**autohedge** is a high-frequency, event-driven trading bot written in Rust. It supports both deterministic HFT (high-frequency trading) and hybrid LLM-gated strategies for stocks and crypto, with robust risk management, position monitoring, and real-time logging. The system is highly configurable via environment variables and is designed for extensibility and operational safety.

---

## Architecture

- **Event-Driven Core:** All major components (market data, strategy, risk, execution, position monitor) communicate via an async event bus.
- **Strategy Modes:**
  - **HFT:** Fast, deterministic micro-trading on small price fluctuations.
  - **Hybrid:** HFT is gated by an LLM (Director) that periodically decides if a symbol is tradeable.
  - **LLM:** (Legacy) All trades require LLM analysis.
- **Risk Management:** All trades are checked by a risk engine, which can approve/reject and set stop-loss/take-profit.
- **Position Monitor:** Monitors open positions and triggers exits (sell) on TP/SL.
- **Execution Engine:** Handles order creation, sizing, and submission to Alpaca.
- **Chatter Level:** Controls log verbosity for debugging and monitoring.

---

## File/Module Structure

- `src/`
  - `main.rs` — Entrypoint, system wiring
  - `config.rs` — Loads and validates all configuration from `.env`
  - `bus.rs` — Event bus for async communication
  - `events.rs` — Event and signal types
  - `data/`
    - `alpaca.rs` — REST client for Alpaca API
    - `store.rs` — In-memory market data store
  - `services/`
    - `websocket_service.rs` — Market/news data streaming
    - `strategy.rs` — Strategy engine (HFT, hybrid, LLM)
    - `risk.rs` — Risk engine
    - `execution.rs` — Order execution
    - `position_monitor.rs` — Position tracking and exit logic
  - `agents/` — LLM and deterministic agents (Director, Quant, Execution, Risk)
  - `llm/` — LLM queue and async management

---

## Algorithm Details

### HFT Strategy
- **Buy Logic:**
  - On every quote, debounce for `HFT_EVALUATE_EVERY_QUOTES`.
  - Require spread ≤ `HFT_MAX_SPREAD_BPS`.
  - Compute momentum edge over last 10 quotes; require edge ≥ `HFT_MIN_EDGE_BPS`.
  - If all pass, emit a buy signal (logs: `[HFT] BUY trigger ...`).
- **Sell Logic:**
  - PositionMonitor checks every quote (if `EXIT_ON_QUOTES=true`).
  - Sell (exit) if price ≥ take-profit or ≤ stop-loss (logs: `[MONITOR] SELL trigger ...`).

### Hybrid Strategy
- Like HFT, but only runs HFT logic if LLM "gate" is open for the symbol.
- LLM gate is refreshed every `HYBRID_GATE_REFRESH_QUOTES` quotes.
- If LLM says "no_trade", HFT is paused for `HYBRID_NO_TRADE_COOLDOWN_QUOTES`.

### Chatter Level
- `CHATTER_LEVEL=low` — Minimal logs (errors, critical events)
- `CHATTER_LEVEL=normal` — Logs buy/sell triggers, gate open/close, exits
- `CHATTER_LEVEL=verbose` — Logs every skip reason, debounce, gate status, per-quote position checks

---

## Configuration (`.env`)

| Variable | Description | Example/Default |
|----------|-------------|-----------------|
| `APCA_API_KEY_ID` | Alpaca API key | ... |
| `APCA_API_SECRET_KEY` | Alpaca secret | ... |
| `APCA_API_BASE_URL` | Alpaca endpoint | https://paper-api.alpaca.markets |
| `TRADING_MODE` | `stocks` or `crypto` | crypto |
| `TRADING_SYMBOLS` | Comma list of symbols | DOGE/USD,XRP/USD,... |
| `LLM_QUEUE_SIZE` | Max queued LLM requests | 100 |
| `LLM_MAX_CONCURRENT` | Max parallel LLM calls | 3 |
| `MARKET_HISTORY_LIMIT` | Quotes to keep | 50 |
| `WARMUP_MIN_COUNT` | Quotes before trading | 50 |
| `MIN_ORDER_AMOUNT` | Min order USD | 10.0 |
| `MAX_ORDER_AMOUNT` | Max order USD | 100.0 |
| `NO_TRADE_COOLDOWN_QUOTES` | Quotes to wait after no_trade | 10 |
| `STRATEGY_MODE` | `llm`, `hft`, `hybrid` | hft |
| `HFT_EVALUATE_EVERY_QUOTES` | Debounce for HFT | 5 |
| `HFT_MIN_EDGE_BPS` | Min edge (bps) | 15.0 |
| `HFT_TAKE_PROFIT_BPS` | TP (bps) | 100.0 |
| `HFT_STOP_LOSS_BPS` | SL (bps) | 50.0 |
| `HFT_MAX_SPREAD_BPS` | Max spread (bps) | 25.0 |
| `EXIT_ON_QUOTES` | Monitor exits on quotes | true |
| `HYBRID_GATE_REFRESH_QUOTES` | LLM gate refresh cadence | 300 |
| `HYBRID_NO_TRADE_COOLDOWN_QUOTES` | Gate cooldown after no_trade | 100 |
| `CHATTER_LEVEL` | Log verbosity | normal |

---

## Running the Project

### Prerequisites
- Rust toolchain (stable)
- Valid Alpaca API keys (for live/paper trading)
- (Optional) OpenAI or compatible LLM endpoint for hybrid/LLM modes

### Setup
1. Copy `.env.example` to `.env` and fill in all required fields.
2. `cargo build --release`
3. `cargo run --release`

### Monitoring
- Logs are output to stdout; set `RUST_LOG` and `CHATTER_LEVEL` as needed.
- For real-time monitoring, use the planned Web UI or tail logs.

---

## Log Interpretation
- `[HFT] BUY trigger ...` — HFT is entering a trade (see log for edge, spread, TP/SL)
- `[MONITOR] SELL trigger (TAKE PROFIT/STOP LOSS) ...` — Position exited for profit or loss
- `[HYBRID] Gate OPEN/CLOSED ...` — LLM gate status for hybrid mode
- `[EXECUTION] ...` — Order submission, sizing, and tracking
- `[RISK] ...` — Risk checks and adjustments
- `[COOLDOWN] ...` — Symbol is cooling down after no_trade

---

## Example Trade Lifecycle
1. **Quote arrives** → Strategy engine evaluates (debounce, spread, edge)
2. **HFT triggers buy** → `[HFT] BUY trigger ...` log
3. **Order is submitted** → `[ORDER] Submitting ...` log
4. **Position is tracked**
5. **Price hits TP/SL** → `[MONITOR] SELL trigger ...` log
6. **Order is submitted to close**

---

## Operational Notes
- Always use real API keys and never commit them to source control.
- For production, run with `RUST_LOG=info` and `CHATTER_LEVEL=normal` or `low`.
- Monitor for API errors and ensure you do not exceed Alpaca rate limits.
- Use paper trading for all testing.
- Review logs for any `[FAILED]` or `[MONITOR]` errors.

---

## Extending/Modifying Strategies
- Add new strategy logic in `src/services/strategy.rs`.
- Add new risk checks in `src/services/risk.rs`.
- Add new agent logic in `src/agents/`.
- Update `.env` and `AppConfig` for new configuration knobs.
- Use the event bus to wire new components.

---

## Advanced Usage

### LLM Integration (Hybrid/LLM Modes)
- The bot can use an LLM (e.g., OpenAI, LM Studio, Ollama) for trade gating and analysis.
- Configure `OPENAI_API_KEY` and `OPENAI_BASE_URL` for your LLM provider.
- In `hybrid` mode, the LLM only gates HFT entries, minimizing API usage and cost.
- In `llm` mode, all trade decisions are LLM-driven (slower, more expensive).
- LLM responses are logged and can be used for audit or research.

### Web UI (Planned)
- A real-time monitoring Web UI is planned for visualizing:
  - Open positions, P&L, and trade history
  - Live quote/price charts
  - Strategy state and logs
- Until then, use log tailing or external dashboards for monitoring.

### Custom Strategies
- You can add new strategies by extending `src/services/strategy.rs`.
- Use the event bus to publish custom signals or events.
- Add new agent types in `src/agents/` for LLM or rule-based logic.

### Backtesting
- While not included by default, the architecture supports plugging in historical data sources for backtesting.
- Consider using a mock `MarketStore` and simulated event streams for testing strategies offline.

---

## Troubleshooting

- **No trades are happening:**
  - Check `WARMUP_MIN_COUNT`, `HFT_MIN_EDGE_BPS`, and `HFT_MAX_SPREAD_BPS` settings.
  - Ensure your `.env` is correct and API keys are valid.
  - Set `CHATTER_LEVEL=verbose` to see why trades are skipped.
- **Orders are rejected:**
  - Check `MIN_ORDER_AMOUNT` and `MAX_ORDER_AMOUNT`.
  - For crypto, ensure notional is above Alpaca's $10 minimum.
- **LLM errors:**
  - Check LLM API key and endpoint.
  - Review logs for `[FAILED]` or `[HYBRID] Director gate failed` messages.
- **Position not closing:**
  - Ensure `EXIT_ON_QUOTES=true` for fastest TP/SL monitoring.
  - Check logs for `[MONITOR]` messages.

---

## FAQ

**Q: Can I run this on real funds?**
A: Only after extensive paper trading and review. Use at your own risk.

**Q: How do I add a new symbol?**
A: Edit `TRADING_SYMBOLS` in `.env` (comma-separated list).

**Q: How do I tune for more/less trades?**
A: Lower `HFT_MIN_EDGE_BPS` and `HFT_MAX_SPREAD_BPS` for more trades; raise for fewer.

**Q: How do I see every decision?**
A: Set `CHATTER_LEVEL=verbose` and review logs.

**Q: Can I use a local LLM?**
A: Yes, set `OPENAI_BASE_URL` to your local endpoint (e.g., LM Studio, Ollama).

**Q: Is there a REST API?**
A: Not yet, but the event-driven core makes it easy to add one.

---

## Glossary

- **HFT:** High-Frequency Trading, fast micro-trades on small price moves.
- **LLM:** Large Language Model, used for trade gating/analysis.
- **TP/SL:** Take-Profit / Stop-Loss, automatic exit triggers.
- **Edge (bps):** Basis points of price movement used for trade entry.
- **Spread:** Difference between bid and ask price.
- **Debounce:** Waiting for N quotes before evaluating a trade.
- **Event Bus:** Internal async message system for decoupling components.
- **Director/Quant Agent:** LLM-based agents for trade decision and sizing.
- **Chatter Level:** Controls log verbosity for debugging/monitoring.

---

## Contact & Community

- For issues, open a GitHub issue or discussion.
- Contributions, bug reports, and feature requests are welcome!
- For security concerns, contact the maintainer directly.

---
