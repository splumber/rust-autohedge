# AutoHedge Rust

AutoHedge Rust is an autonomous, AI-powered algorithmic trading bot written in Rust. It uses a multi-agent LLM system to analyze market data, manage risk, and execute trades on the Alpaca platform.

## Features
*   **Multi-Agent AI:** Specialized agents for Strategy (Director), Technical Analysis (Quant), Risk Management, and Execution.
*   **Event-Driven:** High-performance, asynchronous architecture using Tokio and channels.
*   **Real-time Data:** Streams live market data from Alpaca (Stocks & Crypto).
*   **Configurable:** Easy setup via environment variables.
*   **Safety:** Built-in risk checks and "Human-in-the-loop" simulation via strict prompts.

## Quick Start

1.  **Prerequisites:**
    *   Rust Toolchain (`rustup`)
    *   Alpaca Paper Trading Keys
    *   OpenAI API Key

2.  **Configuration:**
    Copy `.env.example` to `.env` (or create one) and fill in your keys:
    ```env
    OPENAI_API_KEY=your_openai_key
    APCA_API_KEY_ID=your_alpaca_key
    APCA_API_SECRET_KEY=your_alpaca_secret
    TRADING_MODE=crypto
    TRADING_SYMBOLS=BTC/USD
    ```

3.  **Run:**
    ```bash
    cargo run
    ```

4.  **Start the Bot:**
    In a separate terminal:
    ```bash
    curl -X POST http://localhost:3000/start
    ```

## Documentation
For detailed architecture and design decisions, see [TECHNICAL_DESIGN.md](TECHNICAL_DESIGN.md).
# AutoHedge Rust - Technical Design Document

## 1. Overview
AutoHedge Rust is an autonomous, AI-driven algorithmic trading system built in Rust. It leverages a multi-agent LLM (Large Language Model) architecture to analyze market data, formulate trading strategies, manage risk, and execute orders via the Alpaca API. The system is designed for high concurrency, safety, and extensibility.

## 2. Architecture
The system follows an Event-Driven Architecture (EDA) using a central Event Bus to decouple components.

### 2.1 Core Components
1.  **API Server (Axum):** Exposes REST endpoints to control the bot (start/stop) and query status.
2.  **WebSocket Service:** Connects to Alpaca's Market Data stream (Stocks or Crypto) and publishes `MarketEvent`s to the Event Bus.
3.  **Event Bus:** A broadcast channel that distributes events (Market Data, Signals, Orders) to all subscribers.
4.  **Market Store:** An in-memory, thread-safe store (using `DashMap`) that maintains a sliding window of recent price history for technical analysis.
5.  **LLM Queue:** A centralized, priority-based queue for managing concurrent requests to the OpenAI API (or compatible LLMs). It handles rate limiting and prioritization of critical signals.

### 2.2 Multi-Agent Pipeline
The trading logic is distributed across specialized AI Agents:
1.  **Director Agent:**
    *   **Role:** High-level strategy.
    *   **Input:** Recent price history, news (future).
    *   **Output:** Trade decision (Buy/Sell/Hold), thesis, and confidence.
2.  **Quant Agent:**
    *   **Role:** Technical validation.
    *   **Input:** Director's thesis + tabular market data.
    *   **Output:** Technical indicators (Support/Resistance), volatility check, and technical score.
3.  **Risk Agent:**
    *   **Role:** Capital preservation.
    *   **Input:** Proposed trade + Account Buying Power.
    *   **Output:** Approval decision, position size, stop-loss, and take-profit levels.
4.  **Execution Agent:**
    *   **Role:** Order formatting.
    *   **Input:** Approved trade parameters.
    *   **Output:** JSON payload for the Alpaca API order.

### 2.3 Data Flow
1.  **Ingestion:** `WebSocketService` receives a quote/trade -> Updates `MarketStore` -> Publishes `MarketEvent`.
2.  **Strategy:** `StrategyEngine` listens for `MarketEvent`.
    *   Checks if enough history exists (Warm-up).
    *   Triggers `DirectorAgent` (LLM) to analyze the trend.
    *   If Director sees an opportunity -> Triggers `QuantAgent`.
    *   If Quant confirms -> Publishes `AnalysisSignal`.
3.  **Risk:** `RiskEngine` listens for `AnalysisSignal`.
    *   Fetches account balance.
    *   Triggers `RiskAgent` to validate size and limits.
    *   If Approved -> Publishes `OrderSignal`.
4.  **Execution:** `ExecutionService` listens for `OrderSignal`.
    *   Triggers `ExecutionAgent` to format the JSON.
    *   Submits order to Alpaca API.

## 3. Configuration
The system is configured via environment variables (`.env`).

| Variable | Description | Default |
| :--- | :--- | :--- |
| `OPENAI_API_KEY` | API Key for OpenAI (or compatible provider). | **Required** |
| `APCA_API_KEY_ID` | Alpaca API Key ID. | **Required** |
| `APCA_API_SECRET_KEY` | Alpaca API Secret Key. | **Required** |
| `TRADING_MODE` | Asset class to trade (`stocks` or `crypto`). | `stocks` |
| `TRADING_SYMBOLS` | Comma-separated list of symbols (e.g., `AAPL,TSLA` or `BTC/USD`). | `AAPL` or `BTC/USD` |
| `LLM_MODEL` | Model name (e.g., `gpt-4-turbo`, `gpt-3.5-turbo`). | `gpt-4-turbo-preview` |
| `LLM_MAX_CONCURRENT` | Max parallel LLM requests. | `3` |
| `LLM_QUEUE_SIZE` | Max pending LLM requests. | `100` |
| `MARKET_HISTORY_LIMIT` | Number of candles to keep in memory. | `50` |
| `WARMUP_MIN_COUNT` | Min data points before trading starts. | `50` |
| `MAX_ORDER_AMOUNT` | Max $ amount per trade (used by Risk Agent). | `100.0` |

## 4. Running the System

### Prerequisites
*   Rust (latest stable)
*   Alpaca Paper Trading Account
*   OpenAI API Key

### Steps
1.  **Clone & Setup:**
    ```bash
    git clone <repo_url>
    cd rust_autohedge
    ```
2.  **Environment:**
    Create a `.env` file in the root directory:
    ```env
    OPENAI_API_KEY=sk-...
    APCA_API_KEY_ID=...
    APCA_API_SECRET_KEY=...
    TRADING_MODE=crypto
    TRADING_SYMBOLS=BTC/USD,ETH/USD
    ```
3.  **Build & Run:**
    ```bash
    cargo run
    ```
4.  **Control:**
    The server starts on `http://localhost:3000`.
    *   **Start Trading:**
        ```bash
        curl -X POST http://localhost:3000/start
        ```
    *   **Stop Trading:**
        ```bash
        curl -X POST http://localhost:3000/stop
        ```
    *   **Check Assets:**
        ```bash
        curl http://localhost:3000/assets
        ```

## 5. Future Improvements
*   **Backtesting Engine:** Simulate strategy against historical data.
*   **Database Integration:** Persist trade history and signals (Postgres/SQLite).
*   **Dashboard:** Web UI for real-time monitoring.
*   **News Integration:** Feed news headlines to the Director Agent.

