use serde::{Deserialize, Serialize};
use ulid::Ulid;

// ── TargetMap — scanner-facing resolved asset map ────────────────────────

/// Resolved asset information passed to scanners at the scan boundary.
/// Carries contract address and decimals so that custom assets
/// (not in static ChainConfig) are included in RPC filter queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedAsset {
    pub symbol: String,
    pub contract: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_program: Option<String>,
    pub decimals: Option<u32>,
}

/// Maps a normalized address to the list of assets tracked for that address
/// on the chain being scanned. Each entry carries full contract/decimals data.
pub type TargetMap = hashbrown::HashMap<String, Vec<ResolvedAsset>>;

// ── Deposit event types ──────────────────────────────────────────────────

/// Deposit status — determines event type and routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DepositStatus {
    Detected,
    Confirmed,
}

impl DepositStatus {
    /// Full event name following CloudEvents domain-event convention.
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::Detected => "pano.deposit.detected",
            Self::Confirmed => "pano.deposit.confirmed",
        }
    }

    /// Queue routing key under the `pano.deposits` exchange.
    pub fn routing_key(&self) -> &'static str {
        match self {
            Self::Detected => "detected",
            Self::Confirmed => "confirmed",
        }
    }
}

/// Universal deposit event — the single standardized format across all egress methods.
///
/// Follows the CloudEvents-inspired envelope pattern:
/// separate `event` type for detected vs confirmed allows consumers
/// to subscribe to exactly what they need.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DepositEvent {
    /// Sorted unique identifier (ULID).
    pub event_id: String,
    /// Event type: "pano.deposit.detected" or "pano.deposit.confirmed".
    pub event: String,
    /// Event schema version.
    pub version: u32,
    /// When this event was created (ISO 8601).
    pub occurred_at: String,
    /// Deposit payload.
    pub data: DepositData,
}

impl DepositEvent {
    /// Create a new deposit event with generated ULID and current timestamp.
    pub fn new(status: DepositStatus, data: DepositData) -> anyhow::Result<Self> {
        validate_deposit_amount(&data.amount)?;
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        Ok(Self {
            event_id: Ulid::new().to_string(),
            event: status.event_name().to_string(),
            version: 1,
            occurred_at: now,
            data,
        })
    }

    /// Create a detected event.
    pub fn detected(data: DepositData) -> anyhow::Result<Self> {
        Self::new(DepositStatus::Detected, data)
    }

    /// Create a confirmed event from a detected event.
    pub fn confirmed_from(detected: &DepositEvent, confirmations: u32) -> anyhow::Result<Self> {
        let mut data = detected.data.clone();
        data.confirmations = confirmations;
        Self::new(DepositStatus::Confirmed, data)
    }

    /// Derive the deposit status from the event type string.
    pub fn status(&self) -> DepositStatus {
        match self.event.as_str() {
            "pano.deposit.detected" => DepositStatus::Detected,
            "pano.deposit.confirmed" => DepositStatus::Confirmed,
            other => {
                tracing::warn!(event = %other, event_id = %self.event_id, "unknown deposit event type, falling back to Detected");
                DepositStatus::Detected
            }
        }
    }
}

/// Deposit payload — the actual deposit information.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DepositData {
    /// Transaction identifier (hex for EVM/BTC, base58 for Solana).
    pub tx_id: String,
    /// CAIP-2 chain identifier (e.g. "eip155:1").
    pub caip2: String,
    /// Asset ticker symbol (e.g. "ETH", "USDC").
    pub symbol: String,
    /// Watched address that received the deposit.
    pub address: String,
    /// Block number containing the transaction.
    pub block_number: u64,
    /// Network-level index within the transaction (for example EVM logIndex,
    /// Bitcoin vout n, or Solana account index) used to distinguish multiple
    /// identical transfers in the same transaction.
    #[serde(default)]
    pub log_index: u64,
    /// Raw amount in smallest unit (no decimal point).
    pub amount: String,
    /// Sender address (if available).
    pub sender: String,
    /// Number of confirmations at the time of this event.
    pub confirmations: u32,
    /// Block timestamp (ISO 8601).
    pub timestamp: String,
    /// Optional per-address egress override — internal routing data,
    /// excluded from all serialized event output (JSON, SSE, WS, file, SQLite, queue, webhook).
    #[serde(default, skip)]
    pub internal_egress: Option<EgressOverride>,
}

fn validate_deposit_amount(amount: &str) -> anyhow::Result<()> {
    if amount.is_empty() || !amount.chars().all(|c| c.is_ascii_digit()) {
        anyhow::bail!(
            "deposit amount must be a non-empty digit string, got: {:?}",
            amount
        );
    }
    if amount == "0" || (amount.len() > 1 && amount.starts_with('0')) {
        anyhow::bail!(
            "deposit amount must be a positive integer without leading zeros, got: {amount:?}"
        );
    }
    Ok(())
}

/// Normalize address casing for map keys and lookups without changing
/// case-sensitive Bitcoin Base58 or Solana addresses.
pub fn normalize_address_key(addr: &str) -> String {
    let trimmed = addr.trim();
    let lower = trimmed.to_lowercase();
    if lower.starts_with("0x")
        || lower.starts_with("bc1")
        || lower.starts_with("tb1")
        || lower.starts_with("bcrt1")
    {
        lower
    } else {
        trimmed.to_string()
    }
}

// ── WatchSpec ingress types ──────────────────────────────────────────────

/// A watch specification — the unified ingress payload.
///
/// Mirrors the `chains` and `egress` sections of `AppConfig`,
/// with `address` added at root, chain, and asset levels for tracking.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WatchSpec {
    /// Shorthand address: when `chains` is absent, expand to all matching
    /// chains; when `chains` is present, serves as fallback for entries
    /// that omit their own `address`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,

    /// One entry per chain. Mirrors [[chains]] in TOML config.
    /// Requires [override.chains] present in config.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chains: Vec<ChainEntry>,

    /// Override egress settings. Requires override.egress.* = true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress: Option<EgressOverride>,
}

/// One chain entry in a WatchSpec. Fields mirror the overridable subset
/// of ChainConfig, plus `address` which is an ingress concern.
///
/// NOTE: `rpc` and `rpc_options` are intentionally excluded.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChainEntry {
    /// CAIP-2 chain identifier (e.g. "eip155:1"). Required.
    pub caip2: String,

    /// Address to watch on this chain. Falls back to root `address`.
    /// NOT in ChainConfig — ingress concern only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,

    /// Block to start scanning from (overrides ChainConfig).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_block: Option<u64>,

    /// Block to stop at; 0 = follow chain tip (overrides ChainConfig).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_block: Option<u64>,

    /// Standard confirmation count required (overrides ChainConfig).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmed_blocks: Option<u32>,

    /// Assets to track on this chain (overrides ChainConfig.assets).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<AssetEntry>,
}

/// One asset entry in a ChainEntry. Fields mirror AssetConfig exactly,
/// plus `address` which is an ingress concern.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AssetEntry {
    /// Ticker symbol (e.g. "USDT").
    pub symbol: String,

    /// Address override for this specific token/asset.
    /// Falls back to chain.address, then root.address.
    /// NOT in AssetConfig — ingress concern only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,

    /// Token contract address. Required for custom assets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,

    /// Solana SPL token program id. Defaults to the classic SPL Token program
    /// when omitted. Only meaningful for Solana token assets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_program: Option<String>,

    /// Decimal places for the token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u32>,

    /// Minimum deposit amount in smallest unit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_amount: Option<String>,
}

/// Per-spec egress override. Structure mirrors EgressConfig minus
/// `enabled` flags and runtime tuning fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct EgressOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook: Option<WebhookOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<FileOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pg: Option<PgOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sqlite: Option<SqliteOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue: Option<QueueOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<HttpOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct WebhookOverride {
    pub url: String,
    #[serde(default)]
    pub secret: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FileOverride {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PgOverride {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<TableOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SqliteOverride {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<TableOverride>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct QueueOverride {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exchange: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HttpOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sse: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub websocket: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TableOverride {
    pub name: String,
}

// ── ResolvedWatch — flat active watch record ─────────────────────────────

/// Resolved watch entry: one address on one chain for one asset,
/// with effective settings after merging WatchSpec overrides with
/// ChainConfig defaults.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedWatch {
    /// The address being watched (normalized).
    pub address: String,
    /// CAIP-2 chain identifier.
    pub caip2: String,
    /// Asset symbol.
    pub symbol: String,
    /// Contract address for ERC-20/SPL tokens. None for native assets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,
    /// Solana SPL token program id. Defaults to the classic SPL Token program
    /// when omitted. Only meaningful for Solana token assets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_program: Option<String>,
    /// Token decimals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decimals: Option<u32>,
    /// Effective start_block after merging.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_block: Option<u64>,
    /// Effective end_block after merging. 0 or None = follow chain tip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_block: Option<u64>,
    /// Effective confirmed_blocks after merging.
    pub confirmed_blocks: u32,
    /// Minimum amount in smallest unit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_amount: Option<String>,
    /// Per-address egress override (if any).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub egress: Option<EgressOverride>,
}

// ── Unwatch request ──────────────────────────────────────────────────────

/// Request to remove an address watch from queue ingress.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UnwatchAddressRequest {
    pub address: String,
}

// ── ChainKind ────────────────────────────────────────────────────────────

/// Chain type derived from the CAIP-2 namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChainKind {
    Evm,
    Solana,
    Bitcoin,
}

impl ChainKind {
    /// Infer chain type from a CAIP-2 identifier.
    pub fn from_caip2(id: &str) -> Option<Self> {
        let namespace = id.split(':').next()?;
        match namespace {
            "eip155" => Some(Self::Evm),
            "solana" => Some(Self::Solana),
            "bip122" => Some(Self::Bitcoin),
            _ => None,
        }
    }
}

pub fn validate_address_for_chain(kind: ChainKind, addr: &str) -> bool {
    match kind {
        ChainKind::Evm => validate_evm_address(addr),
        ChainKind::Solana => validate_base58_address(addr, 32, 44),
        ChainKind::Bitcoin => validate_bitcoin_address(addr),
    }
}

fn validate_evm_address(addr: &str) -> bool {
    addr.len() == 42 && addr.starts_with("0x") && addr[2..].chars().all(|c| c.is_ascii_hexdigit())
}

fn validate_base58_address(addr: &str, min_len: usize, max_len: usize) -> bool {
    const BASE58: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    (min_len..=max_len).contains(&addr.len()) && addr.chars().all(|c| BASE58.contains(c))
}

/// Canonical BIP-0173 bech32 data character set (32 chars, lowercase).
const BECH32_CHARS: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
const MIN_BECH32_ADDRESS_LEN: usize = 14;

fn validate_bitcoin_address(addr: &str) -> bool {
    let lower = addr.to_lowercase();
    if lower.starts_with("bc1") || lower.starts_with("tb1") || lower.starts_with("bcrt1") {
        let Some((_, payload)) = lower.rsplit_once('1') else {
            return false;
        };
        return (MIN_BECH32_ADDRESS_LEN..=90).contains(&addr.len())
            && !payload.is_empty()
            && payload.chars().all(|c| BECH32_CHARS.contains(c));
    }
    (addr.starts_with('1')
        || addr.starts_with('3')
        || addr.starts_with('m')
        || addr.starts_with('n')
        || addr.starts_with('2'))
        && validate_base58_address(addr, 26, 35)
}

// ── Internal command ─────────────────────────────────────────────────────

/// Internal command passed between subsystems.
#[derive(Debug, Clone)]
pub enum Command {
    /// Add a new watch specification.
    Watch(Box<WatchSpec>),
    /// Remove a watched address (all triads).
    Unwatch { address: String },
    /// Replace watched address state with a complete ingress snapshot
    /// of flat resolved watches.
    SyncAll(Vec<ResolvedWatch>),
    /// Shut down the detector loop.
    Shutdown,
}
