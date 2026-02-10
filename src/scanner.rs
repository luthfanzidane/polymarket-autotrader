use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn, debug};
use std::collections::HashMap;

use crate::config::Config;

/// Represents a discovered market opportunity
#[derive(Debug, Clone, Serialize)]
pub struct MarketOpportunity {
    pub condition_id: String,
    pub token_id: String,
    pub question: String,
    pub slug: String,
    pub event_slug: String,
    pub category: String,
    pub yes_price: f64,
    pub no_price: f64,
    pub liquidity: f64,
    pub volume_24h: f64,
    pub volume_total: f64,
    pub end_date: Option<String>,
    pub url: String,
    pub discovery_type: DiscoveryType,
    pub created_at: Option<String>,
    pub score: f64,
    pub neg_risk: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum DiscoveryType {
    NewMarket,
    Longshot,
    VolumeSurge,
    Mispriced,
}

impl std::fmt::Display for DiscoveryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscoveryType::NewMarket => write!(f, "🆕 New Market"),
            DiscoveryType::Longshot => write!(f, "🎯 Longshot"),
            DiscoveryType::VolumeSurge => write!(f, "📈 Volume Surge"),
            DiscoveryType::Mispriced => write!(f, "⚡ Mispriced"),
        }
    }
}

/// Raw market data from Gamma API
#[derive(Debug, Deserialize, Default, Clone)]
pub struct GammaMarket {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(rename = "conditionId", default)]
    pub condition_id: String,
    #[serde(rename = "outcomePrices", default)]
    pub outcome_prices: Option<String>,
    #[serde(default)]
    pub liquidity: Option<String>,
    #[serde(default)]
    pub volume: Option<String>,
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub closed: bool,
    #[serde(default)]
    pub resolved: Option<bool>,
    #[serde(rename = "endDateIso", default)]
    pub end_date_iso: Option<String>,
    #[serde(rename = "createdAt", default)]
    pub created_at: Option<String>,
    #[serde(rename = "volume24hr", default)]
    pub volume_24hr: Option<f64>,
    #[serde(rename = "clobTokenIds", default)]
    pub clob_token_ids: Option<String>,
    #[serde(default)]
    pub events: Vec<GammaEvent>,
    #[serde(rename = "acceptingOrders", default)]
    pub accepting_orders: bool,
    #[serde(rename = "negRisk", default)]
    pub neg_risk: bool,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct GammaEvent {
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

pub struct Scanner {
    client: reqwest::Client,
    known_market_ids: std::collections::HashSet<String>,
    last_full_scan: Option<DateTime<Utc>>,
    /// Track previous volume for spike detection
    volume_history: HashMap<String, f64>,
}

impl Scanner {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap(),
            known_market_ids: std::collections::HashSet::new(),
            last_full_scan: None,
            volume_history: HashMap::new(),
        }
    }

    /// Scan for new markets created recently
    pub async fn scan_new_markets(&mut self, config: &Config) -> Result<Vec<MarketOpportunity>> {
        info!("🔍 Scanning for new markets...");
        let mut opportunities = Vec::new();

        let url = "https://gamma-api.polymarket.com/markets?limit=100&closed=false&order=createdAt&ascending=false";

        let response = self.client.get(url).send().await?;
        if response.status() != 200 {
            warn!("Gamma API returned {}", response.status());
            return Ok(opportunities);
        }

        let markets: Vec<GammaMarket> = response.json().await?;
        info!("📊 Fetched {} recent markets", markets.len());

        for market in markets {
            if market.closed || market.resolved.unwrap_or(false) || !market.accepting_orders {
                continue;
            }

            // Check if this is a new market we haven't seen
            let is_new = !self.known_market_ids.contains(&market.id);

            if let Some(opp) = self.evaluate_market(&market, config, if is_new { DiscoveryType::NewMarket } else { DiscoveryType::Longshot }) {
                if is_new || opp.yes_price <= config.max_price_decimal() {
                    opportunities.push(opp);
                }
            }

            self.known_market_ids.insert(market.id.clone());
        }

        info!("✅ Found {} opportunities from new market scan", opportunities.len());
        Ok(opportunities)
    }

    /// Full scan for longshot markets (low-price high-upside)
    pub async fn scan_longshots(&mut self, config: &Config) -> Result<Vec<MarketOpportunity>> {
        info!("🎯 Scanning for longshot markets...");
        let mut opportunities = Vec::new();
        let mut offset = 0;
        let limit = 100;
        let max_pages = 20;

        for page in 0..max_pages {
            let url = format!(
                "https://gamma-api.polymarket.com/markets?limit={}&offset={}&closed=false&order=volume24hr&ascending=false",
                limit, offset
            );

            let response = self.client.get(&url).send().await?;
            if response.status() != 200 {
                break;
            }

            let markets: Vec<GammaMarket> = response.json().await?;
            if markets.is_empty() {
                break;
            }

            debug!("Longshot scan page {} - {} markets", page + 1, markets.len());

            for market in &markets {
                if market.closed || market.resolved.unwrap_or(false) || !market.accepting_orders {
                    continue;
                }

                if let Some(opp) = self.evaluate_market(market, config, DiscoveryType::Longshot) {
                    if opp.yes_price <= config.max_price_decimal() && opp.liquidity >= config.min_liquidity_usd {
                        opportunities.push(opp);
                    }
                }

                self.known_market_ids.insert(market.id.clone());
            }

            offset += limit;
            if markets.len() < limit as usize {
                break;
            }
        }

        self.last_full_scan = Some(Utc::now());
        info!("✅ Found {} longshot opportunities", opportunities.len());
        Ok(opportunities)
    }

    /// Evaluate a single market for opportunity potential
    fn evaluate_market(&self, market: &GammaMarket, config: &Config, discovery_type: DiscoveryType) -> Option<MarketOpportunity> {
        // Parse outcome prices
        let prices_str = market.outcome_prices.as_ref()?;
        let prices: Vec<String> = serde_json::from_str(prices_str).ok()?;
        if prices.len() < 2 {
            return None;
        }

        let yes_price: f64 = prices[0].parse().ok()?;
        let no_price: f64 = prices[1].parse().ok()?;

        // Skip zero-price or fully-priced markets
        if yes_price <= 0.001 || yes_price >= 0.99 {
            return None;
        }

        // Skip if above max price
        if yes_price > config.max_price_decimal() && no_price > config.max_price_decimal() {
            return None;
        }

        let liquidity: f64 = market.liquidity.as_ref()
            .and_then(|l| l.parse().ok())
            .unwrap_or(0.0);

        let volume_total: f64 = market.volume.as_ref()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);

        let volume_24h = market.volume_24hr.unwrap_or(0.0);

        // Parse token IDs
        let token_id = market.clob_token_ids.as_ref()
            .and_then(|ids| {
                let parsed: Vec<String> = serde_json::from_str(ids).ok()?;
                // If YES price is cheaper, buy YES (token 0); else buy NO (token 1)
                if yes_price <= no_price {
                    parsed.first().cloned()
                } else {
                    parsed.get(1).cloned()
                }
            })
            .unwrap_or_default();

        // Build URL
        let event_slug = market.events.first()
            .and_then(|e| e.slug.clone())
            .unwrap_or_default();
        let market_slug = market.slug.clone().unwrap_or_default();
        let url = if !event_slug.is_empty() && !market_slug.is_empty() {
            format!("https://polymarket.com/event/{}/{}", event_slug, market_slug)
        } else if !event_slug.is_empty() {
            format!("https://polymarket.com/event/{}", event_slug)
        } else {
            format!("https://polymarket.com/market/{}", market.id)
        };

        // Buy the cheaper side
        let (buy_price, buy_side) = if yes_price <= no_price {
            (yes_price, "YES")
        } else {
            (no_price, "NO")
        };

        // Score the opportunity (higher = better)
        let score = self.score_opportunity(buy_price, liquidity, volume_24h, volume_total, &discovery_type);

        Some(MarketOpportunity {
            condition_id: market.condition_id.clone(),
            token_id,
            question: market.question.clone(),
            slug: market_slug,
            event_slug,
            category: String::new(),
            yes_price,
            no_price,
            liquidity,
            volume_24h,
            volume_total,
            end_date: market.end_date_iso.clone(),
            url,
            discovery_type,
            created_at: market.created_at.clone(),
            score,
            neg_risk: market.neg_risk,
        })
    }

    /// Score an opportunity: higher score = better trade
    fn score_opportunity(&self, price: f64, liquidity: f64, vol_24h: f64, vol_total: f64, discovery_type: &DiscoveryType) -> f64 {
        let mut score = 0.0;

        // Lower price = higher potential upside (max 40 pts)
        if price <= 0.02 { score += 40.0; }
        else if price <= 0.05 { score += 30.0; }
        else if price <= 0.10 { score += 20.0; }
        else if price <= 0.15 { score += 10.0; }

        // Higher liquidity = safer trade (max 25 pts)
        if liquidity >= 10000.0 { score += 25.0; }
        else if liquidity >= 5000.0 { score += 20.0; }
        else if liquidity >= 1000.0 { score += 15.0; }
        else if liquidity >= 500.0 { score += 10.0; }

        // Volume shows interest (max 20 pts)
        if vol_24h >= 10000.0 { score += 20.0; }
        else if vol_24h >= 1000.0 { score += 15.0; }
        else if vol_24h >= 100.0 { score += 10.0; }

        // Discovery type bonus (max 15 pts)
        match discovery_type {
            DiscoveryType::NewMarket => score += 15.0,
            DiscoveryType::VolumeSurge => score += 12.0,
            DiscoveryType::Mispriced => score += 10.0,
            DiscoveryType::Longshot => score += 5.0,
        }

        score
    }

    /// Scan for volume spike opportunities (low-priced markets with sudden volume increase)
    pub async fn scan_volume_spikes(&mut self, config: &Config) -> Result<Vec<MarketOpportunity>> {
        info!("📈 Scanning for volume spikes...");
        let mut opportunities = Vec::new();

        let url = "https://gamma-api.polymarket.com/markets?limit=200&closed=false&order=volume24hr&ascending=false";

        let response = self.client.get(url).send().await?;
        if response.status() != 200 {
            return Ok(opportunities);
        }

        let markets: Vec<GammaMarket> = response.json().await?;

        for market in &markets {
            if market.closed || market.resolved.unwrap_or(false) || !market.accepting_orders {
                continue;
            }

            let vol_24h = market.volume_24hr.unwrap_or(0.0);
            let prev_vol = self.volume_history.get(&market.id).copied().unwrap_or(0.0);

            // Detect volume spike: current 24h vol is 3x+ previous recorded
            let is_spike = prev_vol > 100.0 && vol_24h > prev_vol * 3.0;

            // Update volume history
            self.volume_history.insert(market.id.clone(), vol_24h);

            if is_spike {
                if let Some(mut opp) = self.evaluate_market(market, config, DiscoveryType::VolumeSurge) {
                    let buy_price = opp.yes_price.min(opp.no_price);
                    // Only consider low-priced markets with spikes (< 20¢)
                    if buy_price <= 0.20 && opp.liquidity >= config.min_liquidity_usd {
                        info!("📈 Volume spike detected: {} (vol {:.0} -> {:.0})", opp.question, prev_vol, vol_24h);
                        opp.score += 15.0; // Bonus for volume spike
                        opportunities.push(opp);
                    }
                }
            }

            self.known_market_ids.insert(market.id.clone());
        }

        info!("✅ Found {} volume spike opportunities", opportunities.len());
        Ok(opportunities)
    }

    /// Scan for mispriced markets (YES + NO prices significantly != 1.0)
    pub async fn scan_mispriced(&mut self, config: &Config) -> Result<Vec<MarketOpportunity>> {
        info!("⚡ Scanning for mispriced markets...");
        let mut opportunities = Vec::new();

        let url = "https://gamma-api.polymarket.com/markets?limit=200&closed=false&order=volume24hr&ascending=false";

        let response = self.client.get(url).send().await?;
        if response.status() != 200 {
            return Ok(opportunities);
        }

        let markets: Vec<GammaMarket> = response.json().await?;

        for market in &markets {
            if market.closed || market.resolved.unwrap_or(false) || !market.accepting_orders {
                continue;
            }

            // Parse prices
            let prices_str = match market.outcome_prices.as_ref() {
                Some(p) => p,
                None => continue,
            };
            let prices: Vec<String> = match serde_json::from_str(prices_str) {
                Ok(p) => p,
                Err(_) => continue,
            };
            if prices.len() < 2 {
                continue;
            }

            let yes_price: f64 = prices[0].parse().unwrap_or(0.0);
            let no_price: f64 = prices[1].parse().unwrap_or(0.0);

            // Mispriced: YES + NO should be ~1.0 in an efficient market
            // If sum < 0.95, there's an arbitrage / mispricing opportunity
            let price_sum = yes_price + no_price;
            if price_sum < 0.95 && price_sum > 0.0 {
                let discount = 1.0 - price_sum;
                if let Some(mut opp) = self.evaluate_market(market, config, DiscoveryType::Mispriced) {
                    if opp.liquidity >= config.min_liquidity_usd {
                        info!("⚡ Mispriced market: {} (YES {:.4} + NO {:.4} = {:.4}, discount {:.1}%)",
                            opp.question, yes_price, no_price, price_sum, discount * 100.0);
                        opp.score += discount * 100.0; // Bigger discount = higher score bonus
                        opportunities.push(opp);
                    }
                }
            }

            self.known_market_ids.insert(market.id.clone());
        }

        info!("✅ Found {} mispriced opportunities", opportunities.len());
        Ok(opportunities)
    }

    /// Fetch current prices for tracked positions (by condition_id)
    pub async fn fetch_current_prices(&self, condition_ids: &[String]) -> Result<Vec<(String, f64)>> {
        if condition_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut price_updates = Vec::new();

        // Fetch markets in batches
        for chunk in condition_ids.chunks(20) {
            for cid in chunk {
                let url = format!(
                    "https://gamma-api.polymarket.com/markets?condition_id={}&closed=false",
                    cid
                );

                match self.client.get(&url).send().await {
                    Ok(resp) => {
                        if let Ok(markets) = resp.json::<Vec<GammaMarket>>().await {
                            for market in &markets {
                                if let Some(prices_str) = &market.outcome_prices {
                                    if let Ok(prices) = serde_json::from_str::<Vec<String>>(prices_str) {
                                        if prices.len() >= 2 {
                                            let yes_price: f64 = prices[0].parse().unwrap_or(0.0);
                                            let no_price: f64 = prices[1].parse().unwrap_or(0.0);
                                            // Use the cheaper side (the one we would have bought)
                                            let buy_price = yes_price.min(no_price);
                                            price_updates.push((cid.clone(), buy_price));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        debug!("Failed to fetch price for {}: {}", cid, e);
                    }
                }
            }

            // Small delay between batches to avoid rate limiting
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        Ok(price_updates)
    }

    /// Check if a full longshot scan is needed
    pub fn needs_full_scan(&self, config: &Config) -> bool {
        match self.last_full_scan {
            None => true,
            Some(last) => {
                let elapsed = Utc::now().signed_duration_since(last).num_seconds() as u64;
                elapsed >= config.longshot_scan_interval_secs
            }
        }
    }
}
