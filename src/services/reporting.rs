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

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PerformanceSummary {
    pub total_orders: u64,
    pub total_exec_reports: u64,

    pub buys: u64,
    pub sells: u64,

    pub filled: u64,
    pub rejected: u64,

    pub total_notional: f64,

    /// Per-symbol trade counts
    pub per_symbol: HashMap<String, u64>,
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
        s.total_exec_reports += 1;

        let st = exec.status.to_lowercase();
        if st.contains("fill") {
            s.filled += 1;
        }
        if st.contains("reject") {
            s.rejected += 1;
        }

        if let (Some(q), Some(p)) = (exec.qty, exec.price) {
            s.total_notional += q * p;
        }

        drop(s);

        let entry = TradeLogEntry {
            ts: Utc::now().to_rfc3339(),
            symbol: exec.symbol.clone(),
            action: "unknown".to_string(),
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

        if let Some(parent) = summary_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let s = self.summary.lock().unwrap().clone();
        std::fs::write(summary_path, serde_json::to_vec_pretty(&s)?)?;
        Ok(())
    }
}

