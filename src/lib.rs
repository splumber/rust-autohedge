//! AutoHedge - Automated cryptocurrency trading system
//!
//! This library provides the core functionality for automated trading
//! including market data handling, strategy execution, and position management.

pub mod agents;
pub mod api;
pub mod bus;
pub mod config;
pub mod data;
pub mod events;
pub mod exchange;
pub mod llm;
pub mod services;

// Re-export commonly used types
pub use bus::EventBus;
pub use config::AppConfig;
pub use events::{AnalysisSignal, Event, ExecutionReport, MarketEvent, OrderRequest};

#[cfg(test)]
mod bus_tests;
#[cfg(test)]
mod config_tests;
#[cfg(test)]
mod events_tests;
