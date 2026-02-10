use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Maximum price in cents to buy (e.g., 10 = only buy at ≤ 10¢)
    #[serde(default = "default_max_price_cents")]
    pub max_price_cents: u32,

    /// Minimum liquidity in USD for a market to be considered
    #[serde(default = "default_min_liquidity")]
    pub min_liquidity_usd: f64,

    /// Maximum USDC to spend per single trade
    #[serde(default = "default_max_per_trade")]
    pub max_per_trade_usd: f64,

    /// Maximum USDC to spend per day
    #[serde(default = "default_max_daily_spend")]
    pub max_daily_spend_usd: f64,

    /// Maximum number of open positions at any time
    #[serde(default = "default_max_open_positions")]
    pub max_open_positions: usize,

    /// Maximum exposure per single market
    #[serde(default = "default_max_per_market")]
    pub max_per_market_usd: f64,

    /// Maximum total capital at risk
    #[serde(default = "default_max_total_exposure")]
    pub max_total_exposure_usd: f64,

    /// Categories to trade (empty = all)
    #[serde(default)]
    pub categories: Vec<String>,

    /// Scan interval in seconds for new markets
    #[serde(default = "default_scan_interval")]
    pub scan_interval_secs: u64,

    /// Longshot scan interval in seconds (full scan)
    #[serde(default = "default_longshot_interval")]
    pub longshot_scan_interval_secs: u64,

    /// Auto-sell when price reaches this multiple of entry price
    #[serde(default = "default_auto_sell_multiplier")]
    pub auto_sell_multiplier: f64,

    /// Sell half at this multiplier to lock profits (free ride remaining)
    #[serde(default = "default_partial_sell_multiplier")]
    pub partial_sell_multiplier: f64,

    /// Paper trading mode (no real orders)
    #[serde(default = "default_paper_trading")]
    pub paper_trading: bool,

    /// Telegram bot token
    #[serde(default)]
    pub telegram_bot_token: String,

    /// Telegram chat ID
    #[serde(default)]
    pub telegram_chat_id: String,

    /// Minimum volume in last 24h to consider
    #[serde(default = "default_min_volume_24h")]
    pub min_volume_24h: f64,
}

fn default_max_price_cents() -> u32 { 10 }
fn default_min_liquidity() -> f64 { 500.0 }
fn default_max_per_trade() -> f64 { 10.0 }
fn default_max_daily_spend() -> f64 { 100.0 }
fn default_max_open_positions() -> usize { 50 }
fn default_max_per_market() -> f64 { 20.0 }
fn default_max_total_exposure() -> f64 { 500.0 }
fn default_scan_interval() -> u64 { 30 }
fn default_longshot_interval() -> u64 { 300 }
fn default_auto_sell_multiplier() -> f64 { 3.0 }
fn default_partial_sell_multiplier() -> f64 { 2.0 }
fn default_paper_trading() -> bool { true }
fn default_min_volume_24h() -> f64 { 0.0 }

impl Config {
    pub fn load() -> Self {
        let path = Path::new("config.json");
        if path.exists() {
            let content = std::fs::read_to_string(path).unwrap_or_default();
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Self::default()
        }
    }

    pub fn max_price_decimal(&self) -> f64 {
        self.max_price_cents as f64 / 100.0
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_price_cents: default_max_price_cents(),
            min_liquidity_usd: default_min_liquidity(),
            max_per_trade_usd: default_max_per_trade(),
            max_daily_spend_usd: default_max_daily_spend(),
            max_open_positions: default_max_open_positions(),
            max_per_market_usd: default_max_per_market(),
            max_total_exposure_usd: default_max_total_exposure(),
            categories: vec![],
            scan_interval_secs: default_scan_interval(),
            longshot_scan_interval_secs: default_longshot_interval(),
            auto_sell_multiplier: default_auto_sell_multiplier(),
            partial_sell_multiplier: default_partial_sell_multiplier(),
            paper_trading: default_paper_trading(),
            telegram_bot_token: String::new(),
            telegram_chat_id: String::new(),
            min_volume_24h: default_min_volume_24h(),
        }
    }
}
