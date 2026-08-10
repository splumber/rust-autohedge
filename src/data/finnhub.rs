//! Minimal Finnhub client for PEAD (post-earnings-announcement-drift) research
//! (`src/bin/pead.rs`) - the one signal this project's entire strategy search couldn't
//! test until now, since Alpaca has no earnings-estimate/surprise data. Free tier: 60
//! calls/min, no card required.

use crate::config::FinnhubConfig;
use reqwest::Client;
use serde::Deserialize;
use std::error::Error;

#[derive(Clone)]
pub struct FinnhubClient {
    client: Client,
    base_url: String,
    api_key: String,
}

/// One earnings report event from Finnhub's `/calendar/earnings` endpoint - unlike
/// `/stock/earnings`, this carries the actual report `date` (not just the fiscal period
/// end) and `hour` (before/after market), both needed to know exactly which trading day's
/// bar a PEAD entry should use.
#[derive(Deserialize, Debug, Clone)]
pub struct EarningsEvent {
    pub symbol: String,
    pub date: String,
    /// "bmo" (before market open), "amc" (after market close), or "dmh" (during market
    /// hours) / unspecified - determines whether the drift entry can happen same-day or
    /// must wait for the next trading day.
    #[serde(default)]
    pub hour: String,
    #[serde(rename = "epsEstimate")]
    pub eps_estimate: Option<f64>,
    #[serde(rename = "epsActual")]
    pub eps_actual: Option<f64>,
}

impl EarningsEvent {
    /// Surprise as a percentage of the estimate: positive = beat, negative = miss. `None`
    /// if either figure is missing (future/unreported event) or the estimate is ~0
    /// (percentage undefined).
    pub fn surprise_pct(&self) -> Option<f64> {
        let est = self.eps_estimate?;
        let act = self.eps_actual?;
        if est.abs() < 1e-6 {
            return None;
        }
        Some((act - est) / est.abs() * 100.0)
    }
}

#[derive(Deserialize)]
struct EarningsCalendarResponse {
    #[serde(rename = "earningsCalendar")]
    earnings_calendar: Vec<EarningsEvent>,
}

/// One entry from Finnhub's `/stock/earnings` endpoint - carries `surprisePercent`
/// directly (no need to compute it) and up to ~4 quarters of history on the free tier,
/// well beyond `/calendar/earnings`'s free-tier limit of the single most recent report.
/// The tradeoff: only the fiscal `period` end date is available, not the actual report
/// date, so callers needing a tradable entry timestamp must approximate one (see
/// `EarningsHistoryEntry::approximate_report_date`).
#[derive(Deserialize, Debug, Clone)]
pub struct EarningsHistoryEntry {
    pub symbol: String,
    /// Fiscal period end date (`YYYY-MM-DD`) - NOT the report/announcement date.
    pub period: String,
    #[serde(rename = "surprisePercent")]
    pub surprise_percent: Option<f64>,
}

impl EarningsHistoryEntry {
    /// Approximates the actual report date as the fiscal period end plus a fixed
    /// reporting lag (real companies vary, typically report 2-6 weeks after quarter end -
    /// this is a genuine approximation, not a measured date, and callers should treat any
    /// PEAD result built on it with correspondingly less confidence than one built on
    /// `EarningsEvent`'s real dates).
    pub fn approximate_report_date(&self, lag_days: i64) -> Option<String> {
        let date = chrono::NaiveDate::parse_from_str(&self.period, "%Y-%m-%d").ok()?;
        let approx = date + chrono::Duration::days(lag_days);
        Some(approx.format("%Y-%m-%d").to_string())
    }
}

impl FinnhubClient {
    pub fn new(config: FinnhubConfig) -> Self {
        Self {
            client: Client::new(),
            base_url: config.base_url,
            api_key: config.api_key,
        }
    }

    /// Fetches every earnings report event for `symbol` between `from` and `to`
    /// (`YYYY-MM-DD`), sorted oldest-to-newest, with events missing both eps figures
    /// filtered out (future-dated placeholders Finnhub includes in the range).
    pub async fn get_earnings_calendar(
        &self,
        symbol: &str,
        from: &str,
        to: &str,
    ) -> Result<Vec<EarningsEvent>, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/calendar/earnings", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&[("symbol", symbol), ("from", from), ("to", to), ("token", &self.api_key)])
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(format!("Finnhub get_earnings_calendar failed ({}): {}", status, body).into());
        }

        let mut parsed: EarningsCalendarResponse = serde_json::from_str(&body)
            .map_err(|e| format!("Finnhub get_earnings_calendar decode failed: {} (body: {})", e, body))?;

        parsed.earnings_calendar.retain(|e| e.eps_estimate.is_some() && e.eps_actual.is_some());
        parsed.earnings_calendar.sort_by(|a, b| a.date.cmp(&b.date));
        Ok(parsed.earnings_calendar)
    }

    /// Fetches up to ~4 quarters of surprise-percent history for `symbol` (free-tier
    /// limit) via `/stock/earnings`, sorted oldest-to-newest by fiscal period. See
    /// `EarningsHistoryEntry` for the report-date caveat.
    pub async fn get_earnings_history(
        &self,
        symbol: &str,
    ) -> Result<Vec<EarningsHistoryEntry>, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/stock/earnings", self.base_url);
        let resp = self
            .client
            .get(&url)
            .query(&[("symbol", symbol), ("token", &self.api_key)])
            .send()
            .await?;

        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(format!("Finnhub get_earnings_history failed ({}): {}", status, body).into());
        }

        let mut parsed: Vec<EarningsHistoryEntry> = serde_json::from_str(&body)
            .map_err(|e| format!("Finnhub get_earnings_history decode failed: {} (body: {})", e, body))?;
        parsed.retain(|e| e.surprise_percent.is_some());
        parsed.sort_by(|a, b| a.period.cmp(&b.period));
        Ok(parsed)
    }
}
