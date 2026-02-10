mod config;
mod scanner;
mod strategy;
mod risk;
mod executor;
mod positions;
mod telegram;
mod clob;

use anyhow::Result;
use tracing::{info, warn, error};
use tracing_subscriber::EnvFilter;

use config::Config;
use scanner::Scanner;
use strategy::Strategy;
use risk::RiskManager;
use executor::Executor;
use positions::PositionTracker;
use telegram::TelegramNotifier;

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env file
    dotenv::dotenv().ok();

    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info"))
        )
        .with_target(false)
        .init();

    // Load config
    let config = Config::load();
    let mode = if config.paper_trading { "📝 PAPER TRADING" } else { "🔥 LIVE TRADING" };

    println!("╔══════════════════════════════════════════════════╗");
    println!("║     🎯 Polymarket AutoTrader - Longshot Hunter   ║");
    println!("╠══════════════════════════════════════════════════╣");
    println!("║ Mode: {:<42} ║", mode);
    println!("║ Max/trade: ${:<39.2} ║", config.max_per_trade_usd);
    println!("║ Max/day: ${:<41.2} ║", config.max_daily_spend_usd);
    println!("║ Buy price: ≤ {}¢{:>38} ║", config.max_price_cents, "");
    println!("║ Auto-sell: {}x entry{:>33} ║", config.auto_sell_multiplier, "");
    println!("║ Scan: {}s new | {}s full{:>28} ║", config.scan_interval_secs, config.longshot_scan_interval_secs, "");
    println!("╚══════════════════════════════════════════════════╝");
    println!();

    // Initialize components
    let mut scanner = Scanner::new();
    let mut risk_manager = RiskManager::new();
    let mut executor = Executor::new();
    let mut position_tracker = PositionTracker::new();
    let notifier = TelegramNotifier::new(&config);

    // Initialize CLOB client for live trading
    if !config.paper_trading {
        let private_key = std::env::var("POLYMARKET_PRIVATE_KEY")
            .unwrap_or_default();
        if private_key.is_empty() {
            error!("❌ POLYMARKET_PRIVATE_KEY not set but paper_trading is false!");
            error!("   Set paper_trading: true in config.json or add your private key to .env");
            return Ok(());
        }
        match executor.init_live_trading(&private_key).await {
            Ok(_) => info!("🔥 CLOB client authenticated - live trading ready"),
            Err(e) => {
                error!("❌ Failed to initialize live trading: {}", e);
                error!("   Falling back to paper trading mode");
            }
        }
    }

    // Send startup notification
    if notifier.is_configured() {
        notifier.send_startup(&config).await?;
    } else {
        warn!("⚠️ Telegram not configured - running without notifications");
        warn!("  Set TELEGRAM_BOT_TOKEN and TELEGRAM_CHAT_ID in .env");
    }

    let mut cycle = 0u64;

    loop {
        cycle += 1;
        info!("━━━ Cycle {} ━━━", cycle);

        // Reload config each cycle for hot-reloading
        let config = Config::load();

        // Step 1: Scan for opportunities
        let mut all_opportunities = Vec::new();

        // Always scan new markets
        match scanner.scan_new_markets(&config).await {
            Ok(opps) => all_opportunities.extend(opps),
            Err(e) => warn!("New market scan error: {}", e),
        }

        // Full longshot scan periodically
        if scanner.needs_full_scan(&config) {
            match scanner.scan_longshots(&config).await {
                Ok(opps) => all_opportunities.extend(opps),
                Err(e) => warn!("Longshot scan error: {}", e),
            }
        }

        // Volume spike scan
        match scanner.scan_volume_spikes(&config).await {
            Ok(opps) => all_opportunities.extend(opps),
            Err(e) => warn!("Volume spike scan error: {}", e),
        }

        // Mispriced market scan (every full scan cycle)
        if scanner.needs_full_scan(&config) {
            match scanner.scan_mispriced(&config).await {
                Ok(opps) => all_opportunities.extend(opps),
                Err(e) => warn!("Mispriced scan error: {}", e),
            }
        }

        // Step 2: Filter through strategy
        let existing_positions = position_tracker.position_ids();
        let filtered = Strategy::filter_opportunities(all_opportunities, &config, &existing_positions);

        if !filtered.is_empty() {
            info!("🎯 {} tradeable opportunities found", filtered.len());

            // Notify about discoveries
            if let Err(e) = notifier.send_discoveries(&filtered).await {
                warn!("Failed to send discovery notification: {}", e);
            }
        }

        // Step 3: Execute trades through risk manager
        for opp in &filtered {
            let trade_amount = config.max_per_trade_usd;

            match risk_manager.check_trade(opp, trade_amount, &config) {
                Ok(approved_amount) => {
                    match executor.place_buy_order(opp, approved_amount, &config).await {
                        Ok(trade) => {
                            risk_manager.record_trade(&opp.condition_id, approved_amount);
                            position_tracker.add_from_trade(&trade);

                            if let Err(e) = notifier.send_trade(&trade, opp).await {
                                warn!("Failed to send trade notification: {}", e);
                            }
                        }
                        Err(e) => {
                            warn!("Failed to place trade: {}", e);
                        }
                    }
                }
                Err(e) => {
                    info!("⛔ Trade blocked by risk manager: {}", e);
                    break; // Stop trading this cycle if risk limits hit
                }
            }
        }

        // Step 4: Update position prices from Gamma API
        let position_ids = position_tracker.position_ids();
        if !position_ids.is_empty() {
            match scanner.fetch_current_prices(&position_ids).await {
                Ok(price_updates) => {
                    if !price_updates.is_empty() {
                        info!("📡 Updated prices for {} positions", price_updates.len());
                        position_tracker.update_prices(&price_updates);
                    }
                }
                Err(e) => warn!("Failed to fetch position prices: {}", e),
            }
        }

        // Step 5: Check for exit signals
        let exit_signals = position_tracker.check_exits(&config);
        for signal in &exit_signals {
            info!("🎯 Exit signal: {} {} @ ${:.4} (entry ${:.4}, {:+.1}%)",
                signal.signal_type, signal.question, signal.current_price, signal.entry_price, signal.pnl_pct);

            if let Err(e) = notifier.send_exit_signal(signal).await {
                warn!("Failed to send exit notification: {}", e);
            }
        }

        // Step 6: Log status
        let risk_summary = risk_manager.summary(&config);
        let portfolio_summary = position_tracker.summary();
        info!("{}", risk_summary);
        info!("{}", portfolio_summary);
        info!("📊 Trades today: {} | Total spent: ${:.2}",
            executor.trades_today(), executor.spent_today());

        // Wait for next scan
        info!("⏳ Next scan in {}s...\n", config.scan_interval_secs);
        tokio::time::sleep(tokio::time::Duration::from_secs(config.scan_interval_secs)).await;
    }
}
