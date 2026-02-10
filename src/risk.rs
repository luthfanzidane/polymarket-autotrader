use anyhow::Result;
use chrono::{Utc, NaiveDate};
use tracing::{info, warn};

use crate::config::Config;
use crate::scanner::MarketOpportunity;

/// Manages risk limits and position sizing
pub struct RiskManager {
    daily_spent: f64,
    daily_reset_date: NaiveDate,
    open_position_count: usize,
    total_exposure: f64,
    market_exposure: std::collections::HashMap<String, f64>,
}

impl RiskManager {
    pub fn new() -> Self {
        Self {
            daily_spent: 0.0,
            daily_reset_date: Utc::now().date_naive(),
            open_position_count: 0,
            total_exposure: 0.0,
            market_exposure: std::collections::HashMap::new(),
        }
    }

    /// Check if a trade is allowed under current risk limits
    pub fn check_trade(&mut self, opp: &MarketOpportunity, trade_amount: f64, config: &Config) -> Result<f64> {
        // Reset daily counter if new day
        let today = Utc::now().date_naive();
        if today != self.daily_reset_date {
            info!("📆 New day - resetting daily spend counter");
            self.daily_spent = 0.0;
            self.daily_reset_date = today;
        }

        // 1. Check daily spending limit
        if self.daily_spent + trade_amount > config.max_daily_spend_usd {
            let remaining = config.max_daily_spend_usd - self.daily_spent;
            if remaining <= 0.0 {
                warn!("⛔ Daily spend limit reached (${:.2}/${:.2})", self.daily_spent, config.max_daily_spend_usd);
                return Err(anyhow::anyhow!("Daily spend limit reached"));
            }
            info!("⚠️ Reducing trade to ${:.2} (daily limit)", remaining);
            return Ok(remaining);
        }

        // 2. Check max open positions
        if self.open_position_count >= config.max_open_positions {
            warn!("⛔ Max open positions reached ({}/{})", self.open_position_count, config.max_open_positions);
            return Err(anyhow::anyhow!("Max open positions reached"));
        }

        // 3. Check total exposure
        if self.total_exposure + trade_amount > config.max_total_exposure_usd {
            let remaining = config.max_total_exposure_usd - self.total_exposure;
            if remaining <= 0.0 {
                warn!("⛔ Max total exposure reached (${:.2}/${:.2})", self.total_exposure, config.max_total_exposure_usd);
                return Err(anyhow::anyhow!("Max total exposure reached"));
            }
            info!("⚠️ Reducing trade to ${:.2} (exposure limit)", remaining);
            return Ok(remaining);
        }

        // 4. Check per-market exposure
        let current_market_exposure = self.market_exposure
            .get(&opp.condition_id)
            .copied()
            .unwrap_or(0.0);
        if current_market_exposure + trade_amount > config.max_per_market_usd {
            let remaining = config.max_per_market_usd - current_market_exposure;
            if remaining <= 0.0 {
                warn!("⛔ Max per-market exposure reached for {}", opp.question);
                return Err(anyhow::anyhow!("Max per-market exposure reached"));
            }
            return Ok(remaining);
        }

        // 5. Cap at max per trade
        let final_amount = trade_amount.min(config.max_per_trade_usd);

        Ok(final_amount)
    }

    /// Record a trade was made
    pub fn record_trade(&mut self, condition_id: &str, amount: f64) {
        self.daily_spent += amount;
        self.total_exposure += amount;
        self.open_position_count += 1;
        *self.market_exposure.entry(condition_id.to_string()).or_insert(0.0) += amount;
    }

    /// Record a position was closed
    pub fn record_close(&mut self, condition_id: &str, amount: f64) {
        self.total_exposure = (self.total_exposure - amount).max(0.0);
        self.open_position_count = self.open_position_count.saturating_sub(1);
        self.market_exposure.remove(condition_id);
    }

    /// Update position count from actual data
    pub fn sync_positions(&mut self, count: usize, total_exposure: f64) {
        self.open_position_count = count;
        self.total_exposure = total_exposure;
    }

    /// Get risk summary
    pub fn summary(&self, config: &Config) -> String {
        format!(
            "📊 Risk: ${:.2}/${:.2} daily | {}/{} positions | ${:.2}/${:.2} exposure",
            self.daily_spent, config.max_daily_spend_usd,
            self.open_position_count, config.max_open_positions,
            self.total_exposure, config.max_total_exposure_usd,
        )
    }
}
