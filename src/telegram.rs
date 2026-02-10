use anyhow::Result;
use tracing::{info, warn};

use crate::config::Config;
use crate::executor::Trade;
use crate::positions::ExitSignal;
use crate::scanner::MarketOpportunity;

pub struct TelegramNotifier {
    client: reqwest::Client,
    bot_token: String,
    chat_id: String,
}

impl TelegramNotifier {
    pub fn new(config: &Config) -> Self {
        let bot_token = if config.telegram_bot_token.is_empty() {
            std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default()
        } else {
            config.telegram_bot_token.clone()
        };

        let chat_id = if config.telegram_chat_id.is_empty() {
            std::env::var("TELEGRAM_CHAT_ID").unwrap_or_default()
        } else {
            config.telegram_chat_id.clone()
        };

        Self {
            client: reqwest::Client::new(),
            bot_token,
            chat_id,
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.bot_token.is_empty() && !self.chat_id.is_empty()
    }

    /// Send startup notification
    pub async fn send_startup(&self, config: &Config) -> Result<()> {
        let mode = if config.paper_trading { "📝 PAPER TRADING" } else { "🔥 LIVE TRADING" };
        let msg = format!(
            "🚀 *Polymarket AutoTrader Started*\n\n\
            Mode: {}\n\
            💰 Max/trade: ${:.2}\n\
            📊 Max/day: ${:.2}\n\
            🎯 Buy price: ≤ {}¢\n\
            📈 Auto-sell: {}x entry\n\
            🔄 Scan interval: {}s\n\
            📁 Categories: {}",
            mode,
            config.max_per_trade_usd,
            config.max_daily_spend_usd,
            config.max_price_cents,
            config.auto_sell_multiplier,
            config.scan_interval_secs,
            if config.categories.is_empty() { "All".to_string() } else { config.categories.join(", ") },
        );

        self.send_message(&msg).await
    }

    /// Send notification for a new trade
    pub async fn send_trade(&self, trade: &Trade, opp: &MarketOpportunity) -> Result<()> {
        let emoji = match trade.status {
            crate::executor::TradeStatus::PaperTrade => "📝",
            crate::executor::TradeStatus::Filled => "✅",
            crate::executor::TradeStatus::Pending => "⏳",
            _ => "📊",
        };

        let msg = format!(
            "{} *{} Trade Placed*\n\n\
            {} {}\n\
            📊 {} @ ${:.4}\n\
            📦 {:.0} shares = ${:.2}\n\
            🏷️ Score: {:.0}/100\n\
            💧 Liquidity: ${:.0}\n\
            📈 Vol 24h: ${:.0}\n\
            🔗 [View Market]({})",
            emoji,
            opp.discovery_type,
            trade.side,
            trade.question,
            trade.side,
            trade.price,
            trade.size,
            trade.cost_usd,
            opp.score,
            opp.liquidity,
            opp.volume_24h,
            trade.url,
        );

        self.send_message(&msg).await
    }

    /// Send exit signal notification
    pub async fn send_exit_signal(&self, signal: &ExitSignal) -> Result<()> {
        let msg = format!(
            "🎯 *Exit Signal: {}*\n\n\
            {} {}\n\
            📊 Entry: ${:.4} → Now: ${:.4}\n\
            📈 P/L: {:+.1}%\n\
            🔄 Action: {} ({:.0} shares)",
            signal.signal_type,
            signal.side,
            signal.question,
            signal.entry_price,
            signal.current_price,
            signal.pnl_pct,
            signal.signal_type,
            signal.shares_to_sell,
        );

        self.send_message(&msg).await
    }

    /// Send daily portfolio summary
    pub async fn send_daily_summary(&self, summary: &str, risk_summary: &str) -> Result<()> {
        let msg = format!(
            "📊 *Daily Summary*\n\n{}\n{}",
            summary, risk_summary
        );

        self.send_message(&msg).await
    }

    /// Send a discovery notification (opportunities found but not yet traded)
    pub async fn send_discoveries(&self, opps: &[MarketOpportunity]) -> Result<()> {
        if opps.is_empty() {
            return Ok(());
        }

        let top = opps.iter().take(5);
        let mut msg = format!("🔍 *Found {} Opportunities*\n\n", opps.len());

        for (i, opp) in top.enumerate() {
            let buy_price = opp.yes_price.min(opp.no_price);
            let side = if opp.yes_price <= opp.no_price { "YES" } else { "NO" };
            msg.push_str(&format!(
                "{}. {} {} @ ${:.4} (Score: {:.0})\n{}\n\n",
                i + 1,
                side,
                truncate(&opp.question, 50),
                buy_price,
                opp.score,
                opp.url,
            ));
        }

        self.send_message(&msg).await
    }

    async fn send_message(&self, text: &str) -> Result<()> {
        if !self.is_configured() {
            info!("📨 [Telegram disabled] {}", text.chars().take(100).collect::<String>());
            return Ok(());
        }

        let url = format!(
            "https://api.telegram.org/bot{}/sendMessage",
            self.bot_token
        );

        let body = serde_json::json!({
            "chat_id": self.chat_id,
            "text": text,
            "parse_mode": "Markdown",
            "disable_web_page_preview": true,
        });

        match self.client.post(&url).json(&body).send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    info!("📨 Telegram notification sent");
                } else {
                    warn!("⚠️ Telegram API error: {}", resp.status());
                }
            }
            Err(e) => {
                warn!("⚠️ Telegram send failed: {}", e);
            }
        }

        Ok(())
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
