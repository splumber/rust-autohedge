//! AutoHedge - Automated cryptocurrency trading system
//!
//! This library provides the core functionality for automated trading
//! including market data handling, strategy execution, and position management.

pub mod agents;
pub mod data;
pub mod llm;
pub mod api;
pub mod config;
pub mod events;
pub mod bus;
pub mod services;
pub mod exchange;

// Re-export commonly used types
pub use bus::EventBus;
pub use config::AppConfig;
pub use events::{Event, MarketEvent, AnalysisSignal, OrderRequest, ExecutionReport};

#[cfg(test)]
mod bus_tests;
#[cfg(test)]
mod events_tests;
#[cfg(test)]
mod config_tests;

