use tracing::{info, debug};
use crate::config::Config;
use crate::scanner::MarketOpportunity;

/// Filters and ranks opportunities based on strategy rules
pub struct Strategy;

impl Strategy {
    /// Filter opportunities based on config rules and return only tradeable ones
    pub fn filter_opportunities(
        opportunities: Vec<MarketOpportunity>,
        config: &Config,
        existing_positions: &[String], // condition_ids of existing positions
    ) -> Vec<MarketOpportunity> {
        let mut filtered: Vec<MarketOpportunity> = opportunities
            .into_iter()
            .filter(|opp| {
                // 1. Price check - buy side must be within max price
                let buy_price = opp.yes_price.min(opp.no_price);
                if buy_price > config.max_price_decimal() {
                    debug!("Skipping {} - price {:.4} above max {:.2}", opp.question, buy_price, config.max_price_decimal());
                    return false;
                }

                // 2. Liquidity check
                if opp.liquidity < config.min_liquidity_usd {
                    debug!("Skipping {} - liquidity ${:.0} below min ${:.0}", opp.question, opp.liquidity, config.min_liquidity_usd);
                    return false;
                }

                // 3. Duplicate check - don't buy same market twice
                if existing_positions.contains(&opp.condition_id) {
                    debug!("Skipping {} - already have position", opp.question);
                    return false;
                }

                // 4. Must have valid token ID for trading
                if opp.token_id.is_empty() {
                    debug!("Skipping {} - no token ID", opp.question);
                    return false;
                }

                // 5. Category filter (if configured)
                if !config.categories.is_empty() {
                    let question_lower = opp.question.to_lowercase();
                    let matches_category = config.categories.iter().any(|cat| {
                        let keywords = category_keywords(cat);
                        keywords.iter().any(|kw| question_lower.contains(kw))
                    });
                    if !matches_category {
                        return false;
                    }
                }

                // 6. Volume filter
                if opp.volume_24h < config.min_volume_24h {
                    return false;
                }

                true
            })
            .collect();

        // Sort by score (highest first)
        filtered.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        info!("📋 Strategy: {} opportunities passed filters", filtered.len());
        filtered
    }
}

/// Get keywords for a category name
fn category_keywords(category: &str) -> Vec<&str> {
    match category.to_lowercase().as_str() {
        "politics" => vec!["president", "election", "congress", "senate", "vote", "trump", "biden", "democrat", "republican", "governor", "mayor", "parliament"],
        "crypto" => vec!["bitcoin", "btc", "ethereum", "eth", "solana", "sol", "crypto", "blockchain", "defi", "token", "coin", "nft"],
        "sports" => vec!["nba", "nfl", "mlb", "nhl", "soccer", "football", "basketball", "baseball", "championship", "playoffs", "super bowl", "world cup", "finals"],
        "geopolitics" => vec!["war", "invasion", "strike", "ceasefire", "nato", "sanctions", "nuclear", "missile", "iran", "ukraine", "russia", "china", "taiwan", "israel"],
        "economics" | "economy" => vec!["fed", "interest rate", "inflation", "gdp", "recession", "unemployment", "cpi", "s&p", "nasdaq", "dow", "stock", "tariff"],
        "tech" | "ai" => vec!["ai", "agi", "openai", "google", "apple", "microsoft", "meta", "nvidia", "chatgpt", "artificial intelligence"],
        _ => vec![],
    }
}
