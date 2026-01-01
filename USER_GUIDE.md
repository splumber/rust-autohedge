# AutoHedge Rust - User Guide

## Installation

1.  **Prerequisites**:
    *   Rust (latest stable)
    *   Alpaca Account (Paper or Live)
    *   Ollama (for local LLM) or OpenAI API Key

2.  **Build**:
    ```powershell
    cargo build --release
    ```

## Configuration

1.  **Config File**:
    Copy `config.example.yaml` to `config.yaml` (if not already present).
    
    ```yaml
    trading_mode: "crypto" # or "stocks"
    exchange: "alpaca"
    symbols:
      - "SOL/USD"
      - "DOT/USD"
    
    defaults:
      take_profit_pct: 0.1      # 0.1%
      stop_loss_pct: 0.05       # 0.05%
      min_order_amount: 1.0     # $1
      max_order_amount: 100.0   # $100
    
    strategy_mode: "hft"        # "hft", "llm", "hybrid"
    
    hft:
      evaluate_every_quotes: 1  # Check every quote
      min_edge_bps: 1.0         # Minimum momentum edge (basis points)
      take_profit_bps: 10.0     # HFT TP
      stop_loss_bps: 5.0        # HFT SL
    
    alpaca:
      api_key: "YOUR_KEY"
      secret_key: "YOUR_SECRET"
      base_url: "https://paper-api.alpaca.markets"
    ```

2.  **LLM Setup**:
    Ensure Ollama is running if using local models:
    ```powershell
    ollama run qwen3-coder:30b
    ```

## Running the Bot

1.  **Start the Server**:
    ```powershell
    ./target/release/rust_autohedge.exe
    ```

2.  **Start Trading**:
    The bot starts in "Server Mode". You must trigger trading via the API.
    
    ```powershell
    Invoke-RestMethod -Uri "http://localhost:3000/start" -Method Post
    ```

3.  **Stop Trading**:
    ```powershell
    Invoke-RestMethod -Uri "http://localhost:3000/stop" -Method Post
    ```

4.  **Cancel All Orders**:
    Useful for panic stopping or cleanup.
    ```powershell
    Invoke-RestMethod -Uri "http://localhost:3000/cancel_all" -Method Post
    ```

5.  **View Report**:
    ```powershell
    Invoke-RestMethod -Uri "http://localhost:3000/report" -Method Get
    ```

## Monitoring

*   **Logs**: Check the terminal output for real-time logs.
*   **Chatter Level**: Set `chatter_level: "verbose"` in `config.yaml` for detailed HFT decision logs.

## Troubleshooting

*   **Insufficient Funds**: The bot will cap orders to available balance. If balance is too low (< `min_order_amount`), it will skip trades.
*   **Rate Limits**: If you see 429 errors, increase `evaluate_every_quotes` or `no_trade_cooldown_quotes`.
*   **No Trades**:
    *   Check `min_edge_bps` (lower it).
    *   Check `spread` (if spread > `max_spread_bps`, it skips).
    *   Check `chatter_level: "verbose"` to see why it's skipping.

