use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{info, debug};

use crate::config::Config;
use crate::executor::Trade;

/// Tracks open positions and monitors for exit signals
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub condition_id: String,
    pub token_id: String,
    pub question: String,
    pub side: String,
    pub entry_price: f64,
    pub current_price: f64,
    pub shares: f64,
    pub cost_usd: f64,
    pub current_value: f64,
    pub pnl: f64,
    pub pnl_pct: f64,
    pub url: String,
    pub entered_at: String,
    pub partial_sold: bool,
}

pub struct PositionTracker {
    positions: Vec<Position>,
}

impl PositionTracker {
    pub fn new() -> Self {
        Self {
            positions: Vec::new(),
        }
    }

    /// Add a new position from a filled trade
    pub fn add_from_trade(&mut self, trade: &Trade) {
        // Check if we already have this position
        if let Some(pos) = self.positions.iter_mut().find(|p| p.condition_id == trade.condition_id) {
            // Average in
            let total_shares = pos.shares + trade.size;
            let total_cost = pos.cost_usd + trade.cost_usd;
            pos.entry_price = total_cost / total_shares;
            pos.shares = total_shares;
            pos.cost_usd = total_cost;
            info!("📊 Averaged into position: {} (now {:.0} shares @ ${:.4})", pos.question, pos.shares, pos.entry_price);
        } else {
            let position = Position {
                condition_id: trade.condition_id.clone(),
                token_id: trade.token_id.clone(),
                question: trade.question.clone(),
                side: trade.side.clone(),
                entry_price: trade.price,
                current_price: trade.price,
                shares: trade.size,
                cost_usd: trade.cost_usd,
                current_value: trade.cost_usd,
                pnl: 0.0,
                pnl_pct: 0.0,
                url: trade.url.clone(),
                entered_at: trade.placed_at.clone(),
                partial_sold: false,
            };
            info!("📊 New position: {} {} {:.0} shares @ ${:.4}", trade.side, trade.question, trade.size, trade.price);
            self.positions.push(position);
        }
    }

    /// Update prices for all positions and check for exit signals
    pub fn update_prices(&mut self, price_updates: &[(String, f64)]) -> Vec<ExitSignal> {
        let mut signals = Vec::new();

        for (condition_id, new_price) in price_updates {
            if let Some(pos) = self.positions.iter_mut().find(|p| p.condition_id == *condition_id) {
                pos.current_price = *new_price;
                pos.current_value = pos.shares * new_price;
                pos.pnl = pos.current_value - pos.cost_usd;
                pos.pnl_pct = if pos.cost_usd > 0.0 { (pos.pnl / pos.cost_usd) * 100.0 } else { 0.0 };
            }
        }

        signals
    }

    /// Check all positions for exit signals based on config
    pub fn check_exits(&self, config: &Config) -> Vec<ExitSignal> {
        let mut signals = Vec::new();

        for pos in &self.positions {
            let price_multiple = pos.current_price / pos.entry_price;

            // Full exit: price hit auto_sell_multiplier
            if price_multiple >= config.auto_sell_multiplier {
                signals.push(ExitSignal {
                    condition_id: pos.condition_id.clone(),
                    token_id: pos.token_id.clone(),
                    question: pos.question.clone(),
                    side: pos.side.clone(),
                    signal_type: ExitType::FullExit,
                    shares_to_sell: pos.shares,
                    current_price: pos.current_price,
                    entry_price: pos.entry_price,
                    pnl_pct: pos.pnl_pct,
                });
            }
            // Partial exit: price hit partial_sell_multiplier (sell half)
            else if !pos.partial_sold && price_multiple >= config.partial_sell_multiplier {
                signals.push(ExitSignal {
                    condition_id: pos.condition_id.clone(),
                    token_id: pos.token_id.clone(),
                    question: pos.question.clone(),
                    side: pos.side.clone(),
                    signal_type: ExitType::PartialExit,
                    shares_to_sell: pos.shares / 2.0,
                    current_price: pos.current_price,
                    entry_price: pos.entry_price,
                    pnl_pct: pos.pnl_pct,
                });
            }
        }

        signals
    }

    /// Get all open positions
    pub fn positions(&self) -> &[Position] {
        &self.positions
    }

    /// Get position condition IDs
    pub fn position_ids(&self) -> Vec<String> {
        self.positions.iter().map(|p| p.condition_id.clone()).collect()
    }

    /// Total portfolio value
    pub fn total_value(&self) -> f64 {
        self.positions.iter().map(|p| p.current_value).sum()
    }

    /// Total cost basis
    pub fn total_cost(&self) -> f64 {
        self.positions.iter().map(|p| p.cost_usd).sum()
    }

    /// Total P/L
    pub fn total_pnl(&self) -> f64 {
        self.total_value() - self.total_cost()
    }

    /// Portfolio summary string
    pub fn summary(&self) -> String {
        let total_pnl = self.total_pnl();
        let pnl_pct = if self.total_cost() > 0.0 {
            (total_pnl / self.total_cost()) * 100.0
        } else {
            0.0
        };

        format!(
            "💼 Portfolio: {} positions | Cost: ${:.2} | Value: ${:.2} | P/L: ${:.2} ({:+.1}%)",
            self.positions.len(),
            self.total_cost(),
            self.total_value(),
            total_pnl,
            pnl_pct,
        )
    }
}

#[derive(Debug, Clone)]
pub struct ExitSignal {
    pub condition_id: String,
    pub token_id: String,
    pub question: String,
    pub side: String,
    pub signal_type: ExitType,
    pub shares_to_sell: f64,
    pub current_price: f64,
    pub entry_price: f64,
    pub pnl_pct: f64,
}

#[derive(Debug, Clone)]
pub enum ExitType {
    FullExit,
    PartialExit,
}

impl std::fmt::Display for ExitType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExitType::FullExit => write!(f, "🎯 Full Exit"),
            ExitType::PartialExit => write!(f, "🔄 Partial Exit (50%)"),
        }
    }
}
