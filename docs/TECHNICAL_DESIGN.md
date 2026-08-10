# AutoHedge Rust - Technical Design Document

## Overview
AutoHedge is a high-performance, hybrid algorithmic trading system written in Rust. It combines Large Language Model (LLM) based decision making with High-Frequency Trading (HFT) logic to execute trades in crypto and traditional markets.

## Architecture

The system is built on an Event-Driven Architecture (EDA) using a central `EventBus`.

### Core Components

1.  **Event Bus (`bus.rs`)**:
    *   Asynchronous broadcast channel (MPMC).
    *   Carries `MarketEvent`, `AnalysisSignal`, `OrderRequest`, `ExecutionReport`.

2.  **Market Data (`data/`, `exchange/`)**:
    *   **MarketStore**: In-memory circular buffer of recent quotes and trades.
    *   **Exchange Adapters**: Traits (`TradingApi`, `MarketDataStream`) implemented for Alpaca, Binance, Coinbase, Kraken.
    *   **WebSocket Service**: Consumes real-time data and publishes `MarketEvent`s.

3.  **Strategy Engine (`services/strategy.rs`)**:
    *   **HFT Mode**: Evaluates every quote for momentum and spread edges. Generates signals based on strict mathematical rules.
    *   **LLM Mode**: Uses "Director" agent to assess market context and "Quant" agent to generate thesis.
    *   **Hybrid Mode**: Uses LLM as a "Gate" (Director) to approve/reject symbols for HFT execution.

4.  **Risk Engine (`services/risk.rs`)**:
    *   Validates all signals against account balance and risk parameters.
    *   **HFT Fast Path**: Bypasses LLM risk check for HFT signals to minimize latency.
    *   **LLM Risk Agent**: Evaluates complex trade theses against portfolio state.

5.  **Execution Engine (`services/execution.rs`)**:
    *   Executes `OrderRequest`s.
    *   **Smart Sizing**: Adjusts order quantity based on account balance and config limits (`min_order_amount`, `max_order_amount`).
    *   **HFT Fast Path**: Uses Limit orders (at Ask for Buy) for immediate execution with protection.
    *   **Agent Execution**: Uses "Execution" agent to craft complex orders for LLM signals.

6.  **Position Monitor (`services/position_monitor.rs`)**:
    *   Tracks open positions and pending orders.
    *   Manages **Take Profit** and **Stop Loss**.
    *   **Limit Order Management**: Tracks pending Limit Buys and Sells.
    *   **Auto-Exit**: Triggers sell signals when TP/SL targets are hit.

## Agents (LLM)

*   **Director**: High-level strategy. Decides "Trade" or "No Trade" based on market history and news.
*   **Quant**: Technical analysis. Generates a thesis and confidence score.
*   **Risk**: Risk management. Approves/Rejects trades and sets TP/SL.
*   **Execution**: Order crafting. Determines precise order parameters.

## Configuration (`config.yaml`)

The system is fully configurable via YAML:
*   **Trading Mode**: `crypto` or `stocks`.
*   **Strategy Mode**: `hft`, `llm`, or `hybrid`.
*   **HFT Parameters**: `min_edge_bps`, `evaluate_every_quotes`, etc.
*   **Risk Parameters**: `take_profit_pct`, `stop_loss_pct`, `max_order_amount`.
*   **Symbol Overrides**: Per-symbol configuration.

## Data Flow

1.  **Market Data** -> `EventBus` (MarketEvent)
2.  **Strategy Engine** consumes MarketEvent.
    *   *HFT*: Calculates edge -> `EventBus` (Signal)
    *   *LLM*: Director -> Quant -> `EventBus` (Signal)
3.  **Risk Engine** consumes Signal.
    *   Checks Balance/Risk -> `EventBus` (OrderRequest)
4.  **Execution Engine** consumes OrderRequest.
    *   Submits to Exchange -> `EventBus` (ExecutionReport)
5.  **Position Monitor** tracks execution.
    *   Monitors Price vs TP/SL -> `EventBus` (Signal: Sell)

## Performance Optimizations

*   **HFT Fast Path**: Skips LLM inference for high-frequency signals.
*   **Limit Orders**: Uses Limit orders for entry/exit to control slippage.
*   **Asynchronous**: Fully async Rust (Tokio) for non-blocking I/O.
*   **In-Memory Store**: `MarketStore` avoids DB latency for hot path data.

