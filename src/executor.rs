use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::config::Config;
use crate::scanner::MarketOpportunity;
use crate::clob::{ClobClient, OrderSide};

/// A trade record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub id: String,
    pub condition_id: String,
    pub token_id: String,
    pub question: String,
    pub side: String,
    pub price: f64,
    pub size: f64,           // number of shares
    pub cost_usd: f64,       // total USDC spent
    pub status: TradeStatus,
    pub url: String,
    pub placed_at: String,
    pub filled_at: Option<String>,
    pub order_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TradeStatus {
    Pending,
    Filled,
    PartialFill,
    Cancelled,
    Failed,
    PaperTrade,
}

impl std::fmt::Display for TradeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TradeStatus::Pending => write!(f, "⏳ Pending"),
            TradeStatus::Filled => write!(f, "✅ Filled"),
            TradeStatus::PartialFill => write!(f, "🔄 Partial"),
            TradeStatus::Cancelled => write!(f, "❌ Cancelled"),
            TradeStatus::Failed => write!(f, "💀 Failed"),
            TradeStatus::PaperTrade => write!(f, "📝 Paper"),
        }
    }
}

/// Executes trades on Polymarket CLOB
pub struct Executor {
    trades: Vec<Trade>,
    clob_client: Option<ClobClient>,
}

impl Executor {
    pub fn new() -> Self {
        Self {
            trades: Vec::new(),
            clob_client: None,
        }
    }

    /// Initialize live trading with CLOB client
    pub async fn init_live_trading(&mut self, private_key: &str) -> Result<()> {
        let mut client = ClobClient::new(private_key)?;
        client.authenticate().await?;
        info!("🔥 Live trading initialized for {:?}", client.address());
        self.clob_client = Some(client);
        Ok(())
    }

    /// Place a buy order for an opportunity
    pub async fn place_buy_order(
        &mut self,
        opp: &MarketOpportunity,
        amount_usd: f64,
        config: &Config,
    ) -> Result<Trade> {
        let buy_price = opp.yes_price.min(opp.no_price);
        let side = if opp.yes_price <= opp.no_price { "YES" } else { "NO" };
        let num_shares = amount_usd / buy_price;

        let trade_id = uuid::Uuid::new_v4().to_string();

        if config.paper_trading {
            // Paper trade mode - simulate
            let trade = Trade {
                id: trade_id,
                condition_id: opp.condition_id.clone(),
                token_id: opp.token_id.clone(),
                question: opp.question.clone(),
                side: side.to_string(),
                price: buy_price,
                size: num_shares,
                cost_usd: amount_usd,
                status: TradeStatus::PaperTrade,
                url: opp.url.clone(),
                placed_at: Utc::now().to_rfc3339(),
                filled_at: Some(Utc::now().to_rfc3339()),
                order_id: None,
            };

            info!("📝 PAPER TRADE: {} {} @ ${:.4} ({:.0} shares, ${:.2})",
                side, opp.question, buy_price, num_shares, amount_usd);

            self.trades.push(trade.clone());
            return Ok(trade);
        }

        // === LIVE TRADING MODE ===
        info!("🔥 LIVE ORDER: {} {} @ ${:.4} ({:.0} shares, ${:.2})",
            side, opp.question, buy_price, num_shares, amount_usd);

        let clob = self.clob_client.as_ref()
            .ok_or_else(|| anyhow::anyhow!("CLOB client not initialized - set POLYMARKET_PRIVATE_KEY"))?;

        let order_side = if opp.yes_price <= opp.no_price {
            OrderSide::Buy
        } else {
            OrderSide::Buy // We always buy the cheaper side
        };

        match clob.place_limit_order(&opp.token_id, buy_price, num_shares, order_side, opp.neg_risk).await {
            Ok(resp) => {
                let status = if resp.success { TradeStatus::Pending } else { TradeStatus::Failed };

                let trade = Trade {
                    id: trade_id,
                    condition_id: opp.condition_id.clone(),
                    token_id: opp.token_id.clone(),
                    question: opp.question.clone(),
                    side: side.to_string(),
                    price: buy_price,
                    size: num_shares,
                    cost_usd: amount_usd,
                    status,
                    url: opp.url.clone(),
                    placed_at: Utc::now().to_rfc3339(),
                    filled_at: None,
                    order_id: if resp.success { Some(resp.order_id) } else { None },
                };

                if !resp.success {
                    warn!("💀 Order failed: {:?}", resp.error_msg);
                }

                self.trades.push(trade.clone());
                Ok(trade)
            }
            Err(e) => {
                warn!("💀 CLOB order error: {}", e);
                let trade = Trade {
                    id: trade_id,
                    condition_id: opp.condition_id.clone(),
                    token_id: opp.token_id.clone(),
                    question: opp.question.clone(),
                    side: side.to_string(),
                    price: buy_price,
                    size: num_shares,
                    cost_usd: amount_usd,
                    status: TradeStatus::Failed,
                    url: opp.url.clone(),
                    placed_at: Utc::now().to_rfc3339(),
                    filled_at: None,
                    order_id: None,
                };
                self.trades.push(trade.clone());
                Err(e)
            }
        }
    }

    /// Get all trades
    pub fn trades(&self) -> &[Trade] {
        &self.trades
    }

    /// Get trades by status
    pub fn trades_by_status(&self, status: TradeStatus) -> Vec<&Trade> {
        self.trades.iter().filter(|t| t.status == status).collect()
    }

    /// Count total trades placed today
    pub fn trades_today(&self) -> usize {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        self.trades.iter().filter(|t| t.placed_at.starts_with(&today)).count()
    }

    /// Total spent today
    pub fn spent_today(&self) -> f64 {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        self.trades.iter()
            .filter(|t| t.placed_at.starts_with(&today))
            .map(|t| t.cost_usd)
            .sum()
    }
}
