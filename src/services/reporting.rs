use std::{collections::HashMap, path::PathBuf, sync::{Arc, Mutex}};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::{
    bus::EventBus,
    events::{Event, ExecutionReport, OrderRequest},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TradeLogEntry {
    pub ts: String,
    pub symbol: String,

    /// "buy" | "sell"
    pub action: String,

    /// Exchange order id if known
    pub order_id: String,

    /// "new" | "filled" | "rejected" | ...
    pub status: String,

    pub qty: Option<f64>,
    pub price: Option<f64>,

    /// Estimated notional = qty * price when both are present
    pub notional: Option<f64>,

    /// Extra context (best-effort)
    pub notes: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClosedTrade {
    pub symbol: String,
    pub buy_time: String,
    pub sell_time: String,
    pub buy_price: f64,
    pub sell_price: f64,
    pub qty: f64,
    pub pnl: f64,
    pub pnl_percent: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenPosition {
    pub symbol: String,
    pub buy_time: String,
    pub buy_price: f64,
    pub qty: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PerformanceSummary {
    pub start_time: Option<String>,
    pub total_orders: u64,
    pub total_exec_reports: u64,

    pub buys: u64,
    pub sells: u64,

    pub filled: u64,
    pub rejected: u64,

    pub total_notional: f64,

    /// Per-symbol trade counts
    pub per_symbol: HashMap<String, u64>,

    /// Detailed trade history grouped by symbol
    pub history: HashMap<String, Vec<ClosedTrade>>,
    
    /// Currently open positions
    pub open_positions: HashMap<String, OpenPosition>,

    // === Micro-trading metrics ===
    /// Total realized P&L across all closed trades
    pub total_realized_pnl: f64,

    /// Number of winning trades
    pub winning_trades: u64,

    /// Number of losing trades
    pub losing_trades: u64,

    /// Sum of profits from winning trades
    pub total_profit: f64,

    /// Sum of losses from losing trades
    pub total_loss: f64,
}

/// Computed statistics for display
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComputedStats {
    pub runtime_minutes: f64,
    pub trades_per_hour: f64,
    pub win_rate_pct: f64,
    pub avg_profit_per_trade: f64,
    pub profit_factor: f64,  // total_profit / total_loss
    pub total_closed_trades: u64,
    pub open_position_count: usize,
}

impl PerformanceSummary {
    /// Compute derived statistics
    pub fn compute_stats(&self) -> ComputedStats {
        let runtime_minutes = if let Some(ref start) = self.start_time {
            if let Ok(start_dt) = chrono::DateTime::parse_from_rfc3339(start) {
                let now = Utc::now();
                (now.signed_duration_since(start_dt.with_timezone(&Utc))).num_seconds() as f64 / 60.0
            } else {
                0.0
            }
        } else {
            0.0
        };

        let total_closed = self.winning_trades + self.losing_trades;
        let trades_per_hour = if runtime_minutes > 0.0 {
            (total_closed as f64) / (runtime_minutes / 60.0)
        } else {
            0.0
        };

        let win_rate_pct = if total_closed > 0 {
            (self.winning_trades as f64 / total_closed as f64) * 100.0
        } else {
            0.0
        };

        let avg_profit_per_trade = if total_closed > 0 {
            self.total_realized_pnl / total_closed as f64
        } else {
            0.0
        };

        let profit_factor = if self.total_loss > 0.0 {
            self.total_profit / self.total_loss
        } else if self.total_profit > 0.0 {
            f64::INFINITY
        } else {
            0.0
        };

        ComputedStats {
            runtime_minutes,
            trades_per_hour,
            win_rate_pct,
            avg_profit_per_trade,
            profit_factor,
            total_closed_trades: total_closed,
            open_position_count: self.open_positions.len(),
        }
    }
}

#[derive(Clone)]
pub struct TradeReporter {
    summary: Arc<Mutex<PerformanceSummary>>,
    log_path: PathBuf,
}

impl TradeReporter {
    pub fn new(log_path: PathBuf) -> Self {
        Self {
            summary: Arc::new(Mutex::new(PerformanceSummary::default())),
            log_path,
        }
    }

    pub fn summary(&self) -> PerformanceSummary {
        self.summary.lock().unwrap().clone()
    }

    pub async fn start(&self, event_bus: EventBus) {
        let mut rx = event_bus.subscribe();
        let reporter = self.clone();

        tokio::spawn(async move {
            info!("📈 TradeReporter started (log: {})", reporter.log_path.display());

            while let Ok(event) = rx.recv().await {
                match event {
                    Event::Order(order) => {
                        reporter.on_order(&order);
                    }
                    Event::Execution(exec) => {
                        reporter.on_execution(&exec);
                    }
                    _ => {}
                }

                // Flush to disk best-effort on every relevant event. Cheap + safe.
                // Could be batched later.
                if let Err(e) = reporter.flush_summary() {
                    error!("TradeReporter failed to flush summary: {}", e);
                }
            }
        });
    }

    fn on_order(&self, order: &OrderRequest) {
        let mut s = self.summary.lock().unwrap();
        s.total_orders += 1;
        if order.action.eq_ignore_ascii_case("buy") {
            s.buys += 1;
        }
        if order.action.eq_ignore_ascii_case("sell") {
            s.sells += 1;
        }
        *s.per_symbol.entry(order.symbol.clone()).or_insert(0) += 1;

        drop(s);

        // Optional: write a log line for orders too (as "status=order_created")
        let entry = TradeLogEntry {
            ts: Utc::now().to_rfc3339(),
            symbol: order.symbol.clone(),
            action: order.action.clone(),
            order_id: "unknown".to_string(),
            status: "order_created".to_string(),
            qty: Some(order.qty).filter(|q| *q > 0.0),
            price: order.limit_price,
            notional: order.limit_price.and_then(|p| if order.qty > 0.0 { Some(p * order.qty) } else { None }),
            notes: Some(format!("type={} sl={:?} tp={:?}", order.order_type, order.stop_loss, order.take_profit)),
        };
        let _ = self.append_jsonl(&entry);
    }

    fn on_execution(&self, exec: &ExecutionReport) {
        let mut s = self.summary.lock().unwrap();

        // Initialize start_time on first execution
        if s.start_time.is_none() {
            s.start_time = Some(Utc::now().to_rfc3339());
        }

        s.total_exec_reports += 1;

        let st = exec.status.to_lowercase();
        if st.contains("fill") || st == "new" || st == "accepted" {
             // Assuming "new" or "accepted" means it will be filled for now, 
             // as we don't get async fill updates in this architecture yet.
             // Ideally we should wait for "filled".
             // But ExecutionEngine sends "new" immediately after submit.
             // We'll treat "new" as a fill for reporting purposes to track the lifecycle,
             // acknowledging this is an estimation.
             
             if let (Some(qty), Some(price)) = (exec.qty, exec.price) {
                 if exec.side.eq_ignore_ascii_case("buy") {
                     s.buys += 1;
                     s.open_positions.insert(exec.symbol.clone(), OpenPosition {
                         symbol: exec.symbol.clone(),
                         buy_time: Utc::now().to_rfc3339(),
                         buy_price: price,
                         qty,
                     });
                 } else if exec.side.eq_ignore_ascii_case("sell") {
                     s.sells += 1;
                     if let Some(open_pos) = s.open_positions.remove(&exec.symbol) {
                         let pnl = (price - open_pos.buy_price) * qty;
                         let pnl_percent = (price - open_pos.buy_price) / open_pos.buy_price * 100.0;
                         
                         // Track win/loss metrics
                         s.total_realized_pnl += pnl;
                         if pnl > 0.0 {
                             s.winning_trades += 1;
                             s.total_profit += pnl;
                         } else {
                             s.losing_trades += 1;
                             s.total_loss += pnl.abs();
                         }

                         let trade = ClosedTrade {
                             symbol: exec.symbol.clone(),
                             buy_time: open_pos.buy_time,
                             sell_time: Utc::now().to_rfc3339(),
                             buy_price: open_pos.buy_price,
                             sell_price: price,
                             qty,
                             pnl,
                             pnl_percent,
                         };
                         
                         s.history.entry(exec.symbol.clone()).or_default().push(trade);
                     }
                 }
                 s.total_notional += qty * price;
             }
             s.filled += 1;
        } else if st.contains("reject") {
            s.rejected += 1;
        }

        drop(s);

        let entry = TradeLogEntry {
            ts: Utc::now().to_rfc3339(),
            symbol: exec.symbol.clone(),
            action: exec.side.clone(),
            order_id: exec.order_id.clone(),
            status: exec.status.clone(),
            qty: exec.qty,
            price: exec.price,
            notional: match (exec.qty, exec.price) {
                (Some(q), Some(p)) => Some(q * p),
                _ => None,
            },
            notes: None,
        };

        let _ = self.append_jsonl(&entry);
    }

    fn append_jsonl(&self, entry: &TradeLogEntry) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        use std::io::Write;

        if let Some(parent) = self.log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)?;

        let line = serde_json::to_string(entry)?;
        writeln!(f, "{}", line)?;
        Ok(())
    }

    fn flush_summary(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let summary_path = self
            .log_path
            .with_file_name("trade_summary.json");

        let stats_path = self
            .log_path
            .with_file_name("trade_stats.json");

        if let Some(parent) = summary_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let s = self.summary.lock().unwrap().clone();
        let stats = s.compute_stats();
        
        // Write full summary
        std::fs::write(&summary_path, serde_json::to_vec_pretty(&s)?)?;
        
        // Write computed stats (smaller, easier to read)
        let stats_output = serde_json::json!({
            "runtime_minutes": format!("{:.1}", stats.runtime_minutes),
            "trades_per_hour": format!("{:.2}", stats.trades_per_hour),
            "win_rate_pct": format!("{:.1}%", stats.win_rate_pct),
            "avg_profit_per_trade": format!("${:.4}", stats.avg_profit_per_trade),
            "profit_factor": format!("{:.2}", stats.profit_factor),
            "total_closed_trades": stats.total_closed_trades,
            "open_positions": stats.open_position_count,
            "winning_trades": s.winning_trades,
            "losing_trades": s.losing_trades,
            "total_realized_pnl": format!("${:.4}", s.total_realized_pnl),
            "total_notional_traded": format!("${:.2}", s.total_notional),
        });
        std::fs::write(&stats_path, serde_json::to_vec_pretty(&stats_output)?)?;
        
        Ok(())
    }
}
