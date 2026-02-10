# Polymarket AutoTrader - Longshot Hunter

Automated trading bot for [Polymarket](https://polymarket.com), specifically designed to identify and trade "longshot" opportunities (low-priced markets with high potential upside) using a data-driven strategy.

## Features

- **Automated Market Scanning**: Real-time monitoring of new markets, volume spikes, and mispriced opportunities via Gamma API.
- **Intelligent Strategy**: Mimics high-performance "Longshot Hunter" strategies, targeting entries under 10¢.
- **Robust Risk Management**: Built-in limits for trade size, daily spending, total exposure, and per-market exposure.
- **Live & Paper Trading**: Seamlessly switch between risk-free simulation and live execution via Polymarket CLOB.
- **Telegram Integration**: Instant alerts for discoveries, trades, P&L summaries, and exit signals.
- **Hot-Reloading**: Update trading parameters in `config.json` without restarting the bot.

## Setup

### Prerequisites
- [Rust](https://www.rust-lang.org/tools/install) (latest stable)
- A Polymarket account and API credentials (for live trading)

### Installation
1. Clone the repository:
   ```bash
   git clone https://github.com/luthfanzidane/polymarket-autotrader
   cd polymarket-autotrader
   ```
2. Install dependencies:
   ```bash
   cargo build --release
   ```

### Configuration

#### 1. Environment Variables (`.env`)
Create a `.env` file in the root directory:
```env
# Telegram Notifications (Optional)
TELEGRAM_BOT_TOKEN=your_bot_token
TELEGRAM_CHAT_ID=your_chat_id

# Live Trading (Only if paper_trading is false)
POLYMARKET_PRIVATE_KEY=your_ethereum_private_key
```

#### 2. Trading Parameters (`config.json`)
Adjust your strategy settings in `config.json`:
```json
{
    "max_price_cents": 10,
    "min_liquidity_usd": 500,
    "max_per_trade_usd": 10,
    "max_daily_spend_usd": 100,
    "max_open_positions": 50,
    "auto_sell_multiplier": 3.0,
    "paper_trading": true
}
```

## Usage

Run the bot:
```bash
cargo run
```

The bot will display a dashboard of your current configuration and start scanning cycles.

## Disclaimer

This software is for educational purposes only. Cryptocurrency trading involves significant risk. **Use at your own risk.** The developers are not responsible for any financial losses.

## License
MIT License
