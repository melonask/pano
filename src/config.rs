use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub use crate::egress::file::FileEgressConfig;
pub use crate::egress::pg::{PgEgressColumns, PgEgressConfig, PgEgressTable};
pub use crate::egress::queue::QueueEgressConfig;
pub use crate::egress::sqlite::{SqliteEgressColumns, SqliteEgressConfig, SqliteEgressTable};
pub use crate::egress::webhook::WebhookEgressConfig;
pub use crate::egress::ws::HttpEgressConfig;
pub use crate::ingress::api::HttpIngressConfig;
pub use crate::ingress::file::FileIngressConfig;
pub use crate::ingress::pg::{PgIngressColumns, PgIngressConfig, PgIngressTable};
pub use crate::ingress::queue::QueueIngressConfig;
pub use crate::ingress::sqlite::{SqliteIngressColumns, SqliteIngressConfig, SqliteIngressTable};

trait EgressColumnRefs {
    fn egress_column_refs(&self) -> [(&'static str, &str); 14];
}

macro_rules! impl_egress_column_refs {
    ($type:ty) => {
        impl EgressColumnRefs for $type {
            fn egress_column_refs(&self) -> [(&'static str, &str); 14] {
                [
                    ("event_id", &self.event_id),
                    ("event", &self.event),
                    ("version", &self.version),
                    ("occurred_at", &self.occurred_at),
                    ("tx_id", &self.tx_id),
                    ("caip2", &self.caip2),
                    ("symbol", &self.symbol),
                    ("address", &self.address),
                    ("block_number", &self.block_number),
                    ("log_index", &self.log_index),
                    ("amount", &self.amount),
                    ("sender", &self.sender),
                    ("confirmations", &self.confirmations),
                    ("timestamp", &self.timestamp),
                ]
            }
        }
    };
}

impl_egress_column_refs!(SqliteEgressColumns);
impl_egress_column_refs!(PgEgressColumns);

/// Top-level application configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub detector: DetectorConfig,
    pub chains: Vec<ChainConfig>,
    #[serde(default)]
    pub ingress: IngressConfig,
    #[serde(default)]
    pub egress: EgressConfig,
    #[serde(default, rename = "override")]
    pub override_: OverrideConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectorConfig {
    /// Maximum number of recent deposit event keys retained for in-memory
    /// deduplication. Set to 0 to disable eviction (unbounded memory growth).
    #[serde(default = "default_dedup_window_size")]
    pub dedup_window_size: usize,
    /// Number of async workers used for per-address delivery overrides.
    #[serde(default = "default_delivery_workers")]
    pub delivery_workers: usize,
    /// Bounded internal queue capacity for per-address delivery overrides.
    #[serde(default = "default_delivery_queue_capacity")]
    pub delivery_queue_capacity: usize,
    /// Bounded command queue capacity between ingress and detector.
    #[serde(default = "default_detector_command_queue_capacity")]
    pub command_queue_capacity: usize,
    /// Multiplier used when evicting stale unconfirmed events.
    #[serde(default = "default_stale_event_eviction_multiplier")]
    pub stale_event_eviction_multiplier: u64,
    /// Minimum block/slot distance before stale unconfirmed events are evicted.
    #[serde(default = "default_stale_event_eviction_min_blocks")]
    pub stale_event_eviction_min_blocks: u64,
    /// Maximum asset decimal places accepted by config and runtime watch overrides.
    #[serde(default = "default_max_decimals")]
    pub max_decimals: u32,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            dedup_window_size: default_dedup_window_size(),
            delivery_workers: default_delivery_workers(),
            delivery_queue_capacity: default_delivery_queue_capacity(),
            command_queue_capacity: default_detector_command_queue_capacity(),
            stale_event_eviction_multiplier: default_stale_event_eviction_multiplier(),
            stale_event_eviction_min_blocks: default_stale_event_eviction_min_blocks(),
            max_decimals: default_max_decimals(),
        }
    }
}

fn default_dedup_window_size() -> usize {
    100_000
}
fn default_delivery_workers() -> usize {
    8
}
fn default_delivery_queue_capacity() -> usize {
    4096
}
fn default_detector_command_queue_capacity() -> usize {
    256
}
fn default_stale_event_eviction_multiplier() -> u64 {
    10
}
fn default_stale_event_eviction_min_blocks() -> u64 {
    1_000
}
fn default_max_decimals() -> u32 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_prefix")]
    pub prefix: String,
    #[serde(default)]
    pub dashboard: String,
    /// Export masked config and watched addresses into the dashboard directory.
    #[serde(default)]
    pub dashboard_export: bool,
    /// Optional shared API key. When set, all HTTP routes require either
    /// `Authorization: Bearer <key>` or `X-Pano-API-Key: <key>`.
    #[serde(default)]
    pub api_key: String,
    /// Seconds to wait for background tasks during graceful shutdown.
    #[serde(default = "default_shutdown_timeout_secs")]
    pub shutdown_timeout_secs: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: default_bind(),
            port: default_port(),
            prefix: default_prefix(),
            dashboard: String::new(),
            dashboard_export: false,
            api_key: String::new(),
            shutdown_timeout_secs: default_shutdown_timeout_secs(),
        }
    }
}

pub(crate) fn default_true() -> bool {
    true
}
fn default_bind() -> String {
    "0.0.0.0".to_string()
}
fn default_port() -> u16 {
    3210
}
fn default_prefix() -> String {
    "v1".to_string()
}
fn default_shutdown_timeout_secs() -> u64 {
    1
}

/// Chain definition identified by CAIP-2 identifier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainConfig {
    /// CAIP-2 chain ID (e.g. "eip155:1", "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp").
    pub caip2: String,
    /// Block to start scanning from.
    #[serde(default)]
    pub start_block: Option<u64>,
    /// Block to stop at; 0 = follow chain tip.
    #[serde(default)]
    pub end_block: Option<u64>,
    /// Standard confirmation count required before emitting "confirmed" event.
    /// A transaction in the current tip block has 1 confirmation.
    pub confirmed_blocks: u32,
    /// RPC endpoint URLs (failover order). May contain `${VAR}` env refs.
    pub rpc: Vec<String>,
    /// Rate-limiting options (optional, chain-specific).
    pub rpc_options: Option<RpcOptions>,
    /// Assets available on this chain.
    pub assets: Vec<AssetConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcOptions {
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
    #[serde(default)]
    pub delay_ms: u64,
    #[serde(default = "default_batch_size")]
    pub batch_size: u64,
    /// EVM only. Query multiple ERC-20 contract addresses in one eth_getLogs call.
    /// Disable for local/dev RPCs that do not support address-array filters.
    #[serde(default = "default_true")]
    pub evm_log_address_batching: bool,
    /// Re-scan this many recent blocks/slots on each cycle.
    #[serde(default = "default_scan_lookback_blocks")]
    pub scan_lookback_blocks: u64,
    /// Seconds between detector scan attempts for this chain.
    #[serde(default = "default_scan_interval_secs")]
    pub scan_interval_secs: u64,
    /// Maximum wall-clock seconds allowed for one chain scan attempt.
    #[serde(default = "default_scan_timeout_secs")]
    pub scan_timeout_secs: u64,
    /// Maximum native EVM blocks to scan in one detector cycle. ERC-20 scans use batch_size.
    #[serde(default = "default_max_native_scan_per_cycle")]
    pub max_native_scan_per_cycle: u64,
    /// Per-request RPC HTTP timeout in seconds.
    #[serde(default = "default_request_timeout_secs")]
    pub request_timeout_secs: u64,
    /// Number of retry rounds after the initial attempt across all configured endpoints.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Base delay for exponential RPC retry backoff in milliseconds.
    #[serde(default = "default_retry_base_ms")]
    pub retry_base_ms: u64,
    /// Solana getTransaction maxSupportedTransactionVersion.
    #[serde(default)]
    pub solana_max_supported_transaction_version: u64,
    /// Solana scan mode: "signatures" (per-address getSignaturesForAddress)
    /// or "blocks" (per-slot getBlock). Block mode avoids dependency on
    /// RPC signature indexing at the cost of higher bandwidth per cycle.
    /// Block mode is the default for Solana chains.
    #[serde(default = "default_solana_scan_mode")]
    pub solana_scan_mode: SolanaScanMode,
}

/// Solana chain scan strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SolanaScanMode {
    /// Per-address getSignaturesForAddress + getTransaction (current behaviour).
    #[default]
    Signatures,
    /// Per-slot getBlock with transactionDetails=full. No dependency on RPC
    /// signature indexing. One RPC call per slot regardless of watched address count.
    Blocks,
}

impl Default for RpcOptions {
    fn default() -> Self {
        Self {
            max_concurrent: default_max_concurrent(),
            delay_ms: 0,
            batch_size: default_batch_size(),
            evm_log_address_batching: true,
            scan_lookback_blocks: default_scan_lookback_blocks(),
            scan_interval_secs: default_scan_interval_secs(),
            scan_timeout_secs: default_scan_timeout_secs(),
            max_native_scan_per_cycle: default_max_native_scan_per_cycle(),
            request_timeout_secs: default_request_timeout_secs(),
            max_retries: default_max_retries(),
            retry_base_ms: default_retry_base_ms(),
            solana_max_supported_transaction_version: 0,
            solana_scan_mode: default_solana_scan_mode(),
        }
    }
}

impl ChainConfig {
    pub fn rpc_options_or_default(&self) -> RpcOptions {
        self.rpc_options.clone().unwrap_or_default()
    }

    pub fn effective_scan_lookback_blocks(&self) -> u64 {
        let lookback = self.rpc_options_or_default().scan_lookback_blocks;
        if self.caip2.starts_with("solana:") && lookback == default_scan_lookback_blocks() {
            500
        } else {
            lookback
        }
    }
}

fn default_max_concurrent() -> usize {
    10
}
fn default_batch_size() -> u64 {
    200
}
fn default_scan_lookback_blocks() -> u64 {
    50
}
fn default_scan_interval_secs() -> u64 {
    5
}
fn default_scan_timeout_secs() -> u64 {
    60
}
fn default_max_native_scan_per_cycle() -> u64 {
    100
}
fn default_request_timeout_secs() -> u64 {
    15
}
fn default_max_retries() -> u32 {
    3
}
fn default_retry_base_ms() -> u64 {
    500
}
fn default_solana_scan_mode() -> SolanaScanMode {
    SolanaScanMode::Blocks
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetConfig {
    pub symbol: String,
    pub contract: Option<String>,
    /// Solana SPL token program id. Defaults to the classic SPL Token program
    /// when omitted. Only meaningful for Solana token assets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_program: Option<String>,
    pub decimals: u32,
    /// Minimum amount in smallest unit (e.g. wei). None = track all deposits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_amount: Option<String>,
}

// ── Override config ──────────────────────────────────────────────────────

/// Controls which WatchSpec fields callers are permitted to override.
/// Hierarchy mirrors the relevant AppConfig sections exactly.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OverrideConfig {
    /// If present, chain overrides are enabled. If absent, chains key
    /// must not appear in requests. Uses Option so absence disables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chains: Option<OverrideChains>,
    #[serde(default)]
    pub egress: OverrideEgress,
}

/// Mirrors AppConfig.chains structure.
/// Section present = chain overrides enabled.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OverrideChains {
    /// When true, callers may modify any field within chain entries
    /// AND supply custom assets. When false, `assets` must not appear.
    #[serde(default)]
    pub assets: bool,
}

impl Default for OverrideChains {
    fn default() -> Self {
        Self { assets: false }
    }
}

/// Mirrors AppConfig.egress structure.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct OverrideEgress {
    #[serde(default)]
    pub webhook: bool,
    #[serde(default)]
    pub file: bool,
    #[serde(default)]
    pub pg: bool,
    #[serde(default)]
    pub sqlite: bool,
    #[serde(default)]
    pub queue: bool,
    #[serde(default)]
    pub http: bool,
}

// ── Ingress config ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngressConfig {
    #[serde(default)]
    pub file: FileIngressConfig,
    #[serde(default)]
    pub sqlite: SqliteIngressConfig,
    #[serde(default)]
    pub pg: PgIngressConfig,
    #[serde(default)]
    pub queue: QueueIngressConfig,
    #[serde(default)]
    pub http: HttpIngressConfig,
    #[serde(default = "default_ingress_command_queue_capacity")]
    pub command_queue_capacity: usize,
}

impl Default for IngressConfig {
    fn default() -> Self {
        Self {
            file: FileIngressConfig::default(),
            sqlite: SqliteIngressConfig::default(),
            pg: PgIngressConfig::default(),
            queue: QueueIngressConfig::default(),
            http: HttpIngressConfig::default(),
            command_queue_capacity: default_ingress_command_queue_capacity(),
        }
    }
}

fn default_ingress_command_queue_capacity() -> usize {
    4096
}

// ── Egress config ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EgressConfig {
    #[serde(default)]
    pub file: FileEgressConfig,
    #[serde(default)]
    pub sqlite: SqliteEgressConfig,
    #[serde(default)]
    pub pg: PgEgressConfig,
    #[serde(default)]
    pub queue: QueueEgressConfig,
    #[serde(default = "default_broadcast_capacity")]
    pub broadcast_capacity: usize,
    #[serde(default)]
    pub http: HttpEgressConfig,
    #[serde(default)]
    pub webhook: WebhookEgressConfig,
}

impl Default for EgressConfig {
    fn default() -> Self {
        Self {
            file: FileEgressConfig::default(),
            sqlite: SqliteEgressConfig::default(),
            pg: PgEgressConfig::default(),
            queue: QueueEgressConfig::default(),
            broadcast_capacity: default_broadcast_capacity(),
            http: HttpEgressConfig::default(),
            webhook: WebhookEgressConfig::default(),
        }
    }
}

fn default_broadcast_capacity() -> usize {
    4096
}

// ── Validation ──────────────────────────────────────────────────────────

impl AppConfig {
    /// Load configuration from a TOML file, resolving environment variable references.
    pub fn load(path: &str) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config from {path}"))?;
        let resolved = Self::resolve_env_vars(&raw)?;
        let config: Self = toml_edit::de::from_str(&resolved)
            .with_context(|| format!("failed to parse config from {path}"))?;
        config.validate()?;
        tracing::info!(path, chains = config.chains.len(), "configuration loaded");
        Ok(config)
    }

    /// Replace `${VAR}` placeholders with environment variable values
    /// in a single pass: the replacement values are never re-scanned for
    /// additional `${...}` patterns.
    pub fn resolve_env_vars(input: &str) -> Result<String> {
        let re = regex_lite::Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}")
            .context("failed to compile environment variable placeholder regex")?;
        let mut result = String::with_capacity(input.len());
        let mut last = 0;
        for m in re.find_iter(input) {
            result.push_str(&input[last..m.start()]);
            let full = m.as_str();
            let var = &full[2..full.len() - 1];
            let val = std::env::var(var).with_context(|| {
                format!("environment variable {var} referenced in config is not set")
            })?;
            result.push_str(&val);
            last = m.end();
        }
        result.push_str(&input[last..]);
        Ok(result)
    }

    pub fn is_valid_sql_identifier(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= 63
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && name
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
    }

    fn validate_sql_identifier(path: &str, value: &str) -> Result<()> {
        if !Self::is_valid_sql_identifier(value) {
            anyhow::bail!("{path} is not a valid SQL identifier: {value}");
        }
        Ok(())
    }

    fn validate_sql_table(path: &str, table_name: &str, columns: &[(&str, &str)]) -> Result<()> {
        Self::validate_sql_identifier(&format!("{path}.name"), table_name)?;
        for (field, value) in columns {
            Self::validate_sql_identifier(&format!("{path}.columns.{field}"), value)?;
        }
        Ok(())
    }

    fn is_valid_min_amount(s: &str) -> bool {
        !s.is_empty()
            && s.chars().all(|c| c.is_ascii_digit())
            && s != "0"
            && (s.len() == 1 || !s.starts_with('0'))
    }

    /// Validate configuration consistency.
    fn validate(&self) -> Result<()> {
        if self.chains.is_empty() {
            anyhow::bail!("at least one chain must be configured");
        }
        if self.detector.delivery_workers == 0 {
            anyhow::bail!("detector.delivery_workers must be greater than 0");
        }
        if self.detector.delivery_queue_capacity == 0 {
            anyhow::bail!("detector.delivery_queue_capacity must be greater than 0");
        }
        if self.detector.command_queue_capacity == 0 {
            anyhow::bail!("detector.command_queue_capacity must be greater than 0");
        }
        if self.detector.stale_event_eviction_multiplier == 0 {
            anyhow::bail!("detector.stale_event_eviction_multiplier must be greater than 0");
        }
        if self.detector.stale_event_eviction_min_blocks == 0 {
            anyhow::bail!("detector.stale_event_eviction_min_blocks must be greater than 0");
        }
        if self.detector.max_decimals == 0 {
            anyhow::bail!("detector.max_decimals must be greater than 0");
        }
        if self.ingress.command_queue_capacity == 0 {
            anyhow::bail!("ingress.command_queue_capacity must be greater than 0");
        }
        let mut caip2s = hashbrown::HashSet::new();
        for chain in &self.chains {
            if !caip2s.insert(&chain.caip2) {
                anyhow::bail!("duplicate chain caip2: {}", chain.caip2);
            }
            if chain.rpc.is_empty() {
                anyhow::bail!("chain {} has no RPC endpoints", chain.caip2);
            }
            if chain.confirmed_blocks == 0 {
                anyhow::bail!(
                    "chain {} confirmed_blocks must be greater than 0",
                    chain.caip2
                );
            }
            if let (Some(start), Some(end)) = (chain.start_block, chain.end_block)
                && end > 0
                && start > end
            {
                anyhow::bail!(
                    "chain {} start_block is greater than end_block",
                    chain.caip2
                );
            }
            for rpc_url in &chain.rpc {
                let url = url::Url::parse(rpc_url).with_context(|| {
                    format!("invalid RPC URL for chain {}: {rpc_url}", chain.caip2)
                })?;
                if !matches!(url.scheme(), "http" | "https") {
                    anyhow::bail!(
                        "RPC URL for chain {} must use http(s): {rpc_url}",
                        chain.caip2
                    );
                }
            }
        }
        // Validate chain-scoped options separately (needs chain.caip2 for messages)
        for chain in &self.chains {
            if let Some(options) = &chain.rpc_options {
                if options.max_concurrent == 0 {
                    anyhow::bail!(
                        "chain {} rpc_options.max_concurrent must be greater than 0",
                        chain.caip2
                    );
                }
                if options.batch_size == 0 {
                    anyhow::bail!(
                        "chain {} rpc_options.batch_size must be greater than 0",
                        chain.caip2
                    );
                }
                if options.retry_base_ms == 0 {
                    anyhow::bail!(
                        "chain {} rpc_options.retry_base_ms must be greater than 0",
                        chain.caip2
                    );
                }
                if options.request_timeout_secs == 0 {
                    anyhow::bail!(
                        "chain {} rpc_options.request_timeout_secs must be greater than 0",
                        chain.caip2
                    );
                }
                if options.scan_timeout_secs == 0 {
                    anyhow::bail!(
                        "chain {} rpc_options.scan_timeout_secs must be greater than 0",
                        chain.caip2
                    );
                }
                if chain.start_block.is_none() && options.scan_lookback_blocks == 0 {
                    tracing::warn!(
                        caip2 = %chain.caip2,
                        "chain has no start_block and scan_lookback_blocks=0; initial scan starts at the current tip and ignores earlier history"
                    );
                }
                if chain.caip2.starts_with("solana:")
                    && chain.start_block.is_none()
                    && options.scan_lookback_blocks > 0
                    && options.scan_lookback_blocks < 500
                {
                    tracing::warn!(
                        caip2 = %chain.caip2,
                        scan_lookback_blocks = options.scan_lookback_blocks,
                        "Solana slots are fast; consider scan_lookback_blocks >= 500 or set start_block explicitly"
                    );
                }
            }
            for asset in &chain.assets {
                if asset.symbol.trim().is_empty() {
                    anyhow::bail!("chain {} has asset with empty symbol", chain.caip2);
                }
                if asset.decimals > self.detector.max_decimals {
                    anyhow::bail!(
                        "asset {} on {} has decimals above maximum {}",
                        asset.symbol,
                        chain.caip2,
                        self.detector.max_decimals
                    );
                }
                if let Some(min_amount) = &asset.min_amount
                    && !Self::is_valid_min_amount(min_amount)
                {
                    anyhow::bail!(
                        "asset {} on {} has invalid min_amount: must be a positive integer without leading zeros",
                        asset.symbol,
                        chain.caip2
                    );
                }
            }
        }

        // Validate SQL identifiers for ingress
        if self.ingress.sqlite.enabled {
            let t = &self.ingress.sqlite.table;
            Self::validate_sql_table(
                "ingress.sqlite.table",
                &t.name,
                &[
                    ("address", &t.columns.address),
                    ("caip2", &t.columns.caip2),
                    ("symbol", &t.columns.symbol),
                    ("asset_config", &t.columns.asset_config),
                    ("chain_config", &t.columns.chain_config),
                    ("egress", &t.columns.egress),
                ],
            )?;
        }
        if self.ingress.pg.enabled {
            let t = &self.ingress.pg.table;
            Self::validate_sql_table(
                "ingress.pg.table",
                &t.name,
                &[
                    ("address", &t.columns.address),
                    ("caip2", &t.columns.caip2),
                    ("symbol", &t.columns.symbol),
                    ("asset_config", &t.columns.asset_config),
                    ("chain_config", &t.columns.chain_config),
                    ("egress", &t.columns.egress),
                ],
            )?;
            let url = &self.ingress.pg.url;
            if !url.starts_with("postgres://") && !url.starts_with("postgresql://") {
                anyhow::bail!("ingress.pg.url must start with postgres:// or postgresql://");
            }
        }

        // Validate SQL identifiers for egress
        if self.egress.sqlite.enabled {
            let t = &self.egress.sqlite.table;
            Self::validate_sql_table(
                "egress.sqlite.table",
                &t.name,
                &t.columns.egress_column_refs(),
            )?;
        }
        if self.egress.pg.enabled {
            let t = &self.egress.pg.table;
            Self::validate_sql_table("egress.pg.table", &t.name, &t.columns.egress_column_refs())?;
            let url = &self.egress.pg.url;
            if !url.starts_with("postgres://") && !url.starts_with("postgresql://") {
                anyhow::bail!("egress.pg.url must start with postgres:// or postgresql://");
            }
        }

        let http_ingress_active =
            self.ingress.http.enabled && !self.ingress.http.addresses.trim().is_empty();
        let http_egress_active = self.egress.http.enabled
            && (!self.egress.http.sse.trim().is_empty()
                || !self.egress.http.websocket.trim().is_empty());
        if !self.server.enabled && (http_ingress_active || http_egress_active) {
            anyhow::bail!(
                "server.enabled=false, but HTTP ingress or egress routes are active; enable the server or disable [ingress.http] and [egress.http] routes"
            );
        }
        if self.server.enabled && self.server.port == 0 {
            anyhow::bail!("server.port must be greater than 0");
        }
        if self.server.dashboard_export && self.server.dashboard.trim().is_empty() {
            anyhow::bail!("server.dashboard is required when server.dashboard_export is true");
        }
        if self.server.shutdown_timeout_secs == 0 {
            anyhow::bail!("server.shutdown_timeout_secs must be greater than 0");
        }
        if self.egress.http.ws_heartbeat_secs == 0 {
            anyhow::bail!("egress.http.ws_heartbeat_secs must be greater than 0");
        }
        if self.egress.http.sse_keepalive_secs == 0 {
            anyhow::bail!("egress.http.sse_keepalive_secs must be greater than 0");
        }
        if self.egress.http.ws_max_message_size == 0 {
            anyhow::bail!("egress.http.ws_max_message_size must be greater than 0");
        }
        if self.egress.http.ws_max_frame_size == 0 {
            anyhow::bail!("egress.http.ws_max_frame_size must be greater than 0");
        }
        if self.egress.webhook.enabled {
            if self.egress.webhook.url.trim().is_empty() {
                anyhow::bail!("egress.webhook.url is required when webhook egress is enabled");
            }
            let url = url::Url::parse(&self.egress.webhook.url)
                .context("egress.webhook.url is not a valid URL")?;
            if !matches!(url.scheme(), "http" | "https") {
                anyhow::bail!("egress.webhook.url must use http(s)");
            }
            if self.egress.webhook.timeout_secs == 0 {
                anyhow::bail!("egress.webhook.timeout_secs must be greater than 0");
            }
        }
        if self.ingress.file.enabled
            && self.egress.file.enabled
            && !self.ingress.file.path.trim().is_empty()
            && self.ingress.file.path == self.egress.file.path
        {
            anyhow::bail!("ingress.file.path and egress.file.path must be different");
        }
        Ok(())
    }

    /// Look up a chain configuration by its CAIP-2 identifier.
    pub fn chain_by_caip2(&self, caip2: &str) -> Option<&ChainConfig> {
        self.chains.iter().find(|c| c.caip2 == caip2)
    }
}
