use anyhow::{Result, Context, bail};
use hmac::{Hmac, Mac};
use k256::ecdsa::{SigningKey, signature::hazmat::PrehashSigner, RecoveryId};
use reqwest::Client;
use serde::Deserialize;
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};
use tiny_keccak::{Hasher, Keccak};
use tracing::{info, warn, debug};

const CLOB_URL: &str = "https://clob.polymarket.com";
const CHAIN_ID: u64 = 137;
const CTF_EXCHANGE: &str = "C5d563A36AE78145C45a50134d48A1215220f80a";
const NEG_RISK_CTF_EXCHANGE: &str = "4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E";

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OrderSide { Buy, Sell }

pub struct ClobClient {
    http: Client,
    signing_key: SigningKey,
    address: [u8; 20],
    api_key: String,
    api_secret: String,
    api_passphrase: String,
    authenticated: bool,
}

#[derive(Debug, Deserialize)]
pub struct OrderResponse {
    #[serde(default)]
    pub success: bool,
    #[serde(rename = "orderID", default)]
    pub order_id: String,
    #[serde(rename = "errorMsg", default)]
    pub error_msg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiKeyResponse {
    #[serde(rename = "apiKey")]
    api_key: String,
    secret: String,
    passphrase: String,
}

impl ClobClient {
    pub fn new(private_key: &str) -> Result<Self> {
        let key_hex = private_key.strip_prefix("0x").unwrap_or(private_key);
        let key_bytes = hex::decode(key_hex).context("Invalid private key hex")?;
        let signing_key = SigningKey::from_slice(&key_bytes).context("Invalid private key")?;
        let address = pubkey_to_address(&signing_key);

        Ok(Self {
            http: Client::builder().timeout(std::time::Duration::from_secs(30)).build()?,
            signing_key, address,
            api_key: String::new(), api_secret: String::new(), api_passphrase: String::new(),
            authenticated: false,
        })
    }

    pub fn is_authenticated(&self) -> bool { self.authenticated }
    pub fn address(&self) -> String { format!("0x{}", hex::encode(self.address)) }

    /// Derive API credentials via EIP-712 ClobAuth signature
    pub async fn authenticate(&mut self) -> Result<()> {
        info!("🔑 Deriving CLOB API key for {}...", self.address());

        let timestamp = current_timestamp();
        let message = "This message attests that I control the given wallet";

        let domain_sep = domain_separator("ClobAuthDomain", "1", CHAIN_ID, None);
        let struct_hash = clob_auth_hash(&self.address, &timestamp, &[0u8; 32], message);
        let digest = eip712_digest(&domain_sep, &struct_hash);

        let sig_hex = self.sign_digest(&digest)?;

        let body = serde_json::json!({
            "address": self.address(),
            "timestamp": timestamp,
            "nonce": "0",
            "message": message,
            "signature": sig_hex,
        });

        let resp = self.http
            .post(format!("{}/auth/derive-api-key", CLOB_URL))
            .json(&body).send().await
            .context("Failed to call derive-api-key")?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("API key derivation failed: {}", text);
        }

        let creds: ApiKeyResponse = resp.json().await.context("Failed to parse API key response")?;
        self.api_key = creds.api_key;
        self.api_secret = creds.secret;
        self.api_passphrase = creds.passphrase;
        self.authenticated = true;

        info!("✅ CLOB API key derived successfully");
        Ok(())
    }

    /// Place a GTC limit order
    pub async fn place_limit_order(
        &self, token_id: &str, price: f64, size: f64, side: OrderSide, neg_risk: bool,
    ) -> Result<OrderResponse> {
        if !self.authenticated { bail!("Not authenticated"); }

        let exchange_hex = if neg_risk { NEG_RISK_CTF_EXCHANGE } else { CTF_EXCHANGE };
        let exchange_bytes = hex::decode(exchange_hex)?;

        let salt: u64 = rand::random();
        let factor = 1_000_000u64; // 6 decimals

        let (maker_amt, taker_amt) = match side {
            OrderSide::Buy => (
                (price * size * factor as f64) as u64,
                (size * factor as f64) as u64,
            ),
            OrderSide::Sell => (
                (size * factor as f64) as u64,
                (price * size * factor as f64) as u64,
            ),
        };

        let side_num: u8 = if side == OrderSide::Buy { 0 } else { 1 };

        let order_domain = domain_separator("CTF Exchange", "1", CHAIN_ID, Some(&exchange_bytes));
        let order_hash = order_struct_hash(
            &u64_to_bytes32(salt), &self.address, &self.address, &[0u8; 20],
            token_id, maker_amt, taker_amt, 0, 0, 100, side_num, 2,
        );
        let digest = eip712_digest(&order_domain, &order_hash);
        let sig_hex = self.sign_digest(&digest)?;

        let side_str = if side == OrderSide::Buy { "BUY" } else { "SELL" };
        let addr_str = self.address();
        let zero_addr = format!("0x{}", "0".repeat(40));

        let payload = serde_json::json!({
            "order": {
                "salt": salt.to_string(),
                "maker": addr_str,
                "signer": addr_str,
                "taker": zero_addr,
                "tokenId": token_id,
                "makerAmount": maker_amt.to_string(),
                "takerAmount": taker_amt.to_string(),
                "expiration": "0",
                "nonce": "0",
                "feeRateBps": "100",
                "side": side_str,
                "signatureType": 2,
                "signature": sig_hex,
            },
            "owner": addr_str,
            "orderType": "GTC",
        });

        let headers = self.l2_headers("POST", "/order", &serde_json::to_string(&payload)?)?;
        let mut req = self.http.post(format!("{}/order", CLOB_URL)).json(&payload);
        for (k, v) in &headers { req = req.header(k, v); }

        let resp = req.send().await.context("Failed to post order")?;
        let order_resp: OrderResponse = resp.json().await.context("Failed to parse order response")?;

        if order_resp.success {
            info!("✅ Order placed: {}", order_resp.order_id);
        } else {
            warn!("⚠️ Order failed: {:?}", order_resp.error_msg);
        }
        Ok(order_resp)
    }

    /// Cancel an order
    pub async fn cancel_order(&self, order_id: &str) -> Result<bool> {
        if !self.authenticated { bail!("Not authenticated"); }
        let payload = serde_json::json!({ "orderID": order_id });
        let body_str = serde_json::to_string(&payload)?;
        let headers = self.l2_headers("DELETE", "/order", &body_str)?;
        let mut req = self.http.delete(format!("{}/order", CLOB_URL)).json(&payload);
        for (k, v) in &headers { req = req.header(k, v); }
        let resp = req.send().await?;
        Ok(resp.status().is_success())
    }

    fn sign_digest(&self, digest: &[u8; 32]) -> Result<String> {
        let (sig, recid): (k256::ecdsa::Signature, RecoveryId) =
            self.signing_key.sign_prehash(digest)
                .map_err(|e| anyhow::anyhow!("Signing failed: {}", e))?;
        let mut bytes = [0u8; 65];
        bytes[..64].copy_from_slice(&sig.to_bytes());
        bytes[64] = recid.to_byte() + 27; // Ethereum v value
        Ok(format!("0x{}", hex::encode(bytes)))
    }

    fn l2_headers(&self, method: &str, path: &str, body: &str) -> Result<Vec<(String, String)>> {
        let timestamp = current_timestamp();
        let message = format!("{}{}{}{}", timestamp, method.to_uppercase(), path, body);
        let secret_bytes = base64::Engine::decode(
            &base64::engine::general_purpose::URL_SAFE, &self.api_secret
        ).context("Failed to decode API secret")?;
        let mut mac = HmacSha256::new_from_slice(&secret_bytes).context("Invalid HMAC key")?;
        mac.update(message.as_bytes());
        let sig = base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE, mac.finalize().into_bytes()
        );
        Ok(vec![
            ("POLY-ADDRESS".into(), self.address()),
            ("POLY-SIGNATURE".into(), sig),
            ("POLY-TIMESTAMP".into(), timestamp),
            ("POLY-API-KEY".into(), self.api_key.clone()),
            ("POLY-PASSPHRASE".into(), self.api_passphrase.clone()),
        ])
    }
}

// === Crypto Helpers ===

fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak::v256();
    let mut out = [0u8; 32];
    hasher.update(data);
    hasher.finalize(&mut out);
    out
}

fn current_timestamp() -> String {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs().to_string()
}

fn pubkey_to_address(key: &SigningKey) -> [u8; 20] {
    let pubkey = key.verifying_key();
    let pubkey_bytes = pubkey.to_encoded_point(false);
    let hash = keccak256(&pubkey_bytes.as_bytes()[1..]); // skip 0x04 prefix
    let mut addr = [0u8; 20];
    addr.copy_from_slice(&hash[12..]);
    addr
}

fn u256_bytes(v: u64) -> [u8; 32] {
    let mut b = [0u8; 32];
    b[24..].copy_from_slice(&v.to_be_bytes());
    b
}

fn u64_to_bytes32(v: u64) -> [u8; 32] { u256_bytes(v) }

fn addr_to_bytes32(a: &[u8]) -> [u8; 32] {
    let mut b = [0u8; 32];
    let start = 32 - a.len().min(20);
    b[start..start + a.len().min(20)].copy_from_slice(&a[..a.len().min(20)]);
    b
}

fn domain_separator(name: &str, version: &str, chain_id: u64, contract: Option<&[u8]>) -> [u8; 32] {
    let type_hash = if contract.is_some() {
        keccak256(b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)")
    } else {
        keccak256(b"EIP712Domain(string name,string version,uint256 chainId)")
    };
    let mut enc = Vec::new();
    enc.extend_from_slice(&type_hash);
    enc.extend_from_slice(&keccak256(name.as_bytes()));
    enc.extend_from_slice(&keccak256(version.as_bytes()));
    enc.extend_from_slice(&u256_bytes(chain_id));
    if let Some(c) = contract { enc.extend_from_slice(&addr_to_bytes32(c)); }
    keccak256(&enc)
}

fn clob_auth_hash(address: &[u8; 20], timestamp: &str, nonce: &[u8; 32], message: &str) -> [u8; 32] {
    let type_hash = keccak256(
        b"ClobAuth(address address,string timestamp,uint256 nonce,string message)"
    );
    let mut enc = Vec::new();
    enc.extend_from_slice(&type_hash);
    enc.extend_from_slice(&addr_to_bytes32(address));
    enc.extend_from_slice(&keccak256(timestamp.as_bytes()));
    enc.extend_from_slice(nonce);
    enc.extend_from_slice(&keccak256(message.as_bytes()));
    keccak256(&enc)
}

#[allow(clippy::too_many_arguments)]
fn order_struct_hash(
    salt: &[u8; 32], maker: &[u8; 20], signer: &[u8; 20], taker: &[u8; 20],
    token_id: &str, maker_amount: u64, taker_amount: u64,
    expiration: u64, nonce: u64, fee_rate_bps: u64, side: u8, sig_type: u8,
) -> [u8; 32] {
    let type_hash = keccak256(
        b"Order(uint256 salt,address maker,address signer,address taker,uint256 tokenId,uint256 makerAmount,uint256 takerAmount,uint256 expiration,uint256 nonce,uint256 feeRateBps,uint8 side,uint8 signatureType)"
    );
    // Parse token_id as decimal string to big-endian bytes
    let token_id_val: u128 = token_id.parse().unwrap_or(0);
    let mut token_id_bytes = [0u8; 32];
    token_id_bytes[16..].copy_from_slice(&token_id_val.to_be_bytes());

    let mut enc = Vec::new();
    enc.extend_from_slice(&type_hash);
    enc.extend_from_slice(salt);
    enc.extend_from_slice(&addr_to_bytes32(maker));
    enc.extend_from_slice(&addr_to_bytes32(signer));
    enc.extend_from_slice(&addr_to_bytes32(taker));
    enc.extend_from_slice(&token_id_bytes);
    enc.extend_from_slice(&u256_bytes(maker_amount));
    enc.extend_from_slice(&u256_bytes(taker_amount));
    enc.extend_from_slice(&u256_bytes(expiration));
    enc.extend_from_slice(&u256_bytes(nonce));
    enc.extend_from_slice(&u256_bytes(fee_rate_bps));
    enc.extend_from_slice(&u256_bytes(side as u64));
    enc.extend_from_slice(&u256_bytes(sig_type as u64));
    keccak256(&enc)
}

fn eip712_digest(domain: &[u8; 32], struct_hash: &[u8; 32]) -> [u8; 32] {
    let mut msg = Vec::with_capacity(66);
    msg.push(0x19);
    msg.push(0x01);
    msg.extend_from_slice(domain);
    msg.extend_from_slice(struct_hash);
    keccak256(&msg)
}
