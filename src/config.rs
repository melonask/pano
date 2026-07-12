use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ── Re-export egress/ingress config types (unchanged public API) ──────────

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

// ── Internal column refs trait ────────────────────────────────────────────

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

// ── Shared universal config (root-level sections) ────────────────────────

/// Universal shared configuration parsed from root-level TOML sections.
/// Pano reads `[chains]`, `[assets]`, `[paths]`, `[transports]`, `[stores]`,
/// and `[pano]`. Other package
/// namespaces (`[ladon]`, `[bria]`, `[oracles]`) are silently ignored.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct UniversalConfig {
    version: Option<u64>,
    stores: BTreeMap<String, StoreConfig>,
    chains: BTreeMap<String, SharedChainConfig>,
    assets: BTreeMap<String, SharedAssetConfig>,
    paths: BTreeMap<String, PathConfig>,
    transports: TransportConfigs,
    // Package namespace — parsed selectively below.
    pano: Option<PanoRootConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
struct TransportConfigs {
    amqp: BTreeMap<String, AmqpTransportConfig>,
    webhook: BTreeMap<String, WebhookTransportConfig>,
}

// ── Shared root section types ─────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreConfig {
    driver: String,
    url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SharedChainConfig {
    caip2: String,
    rpc_urls: Vec<String>,
    confirmations: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct SharedAssetConfig {
    #[serde(default = "default_true")]
    enabled: bool,
    chain: String,
    symbol: String,
    #[serde(default)]
    contract: Option<String>,
    #[serde(default)]
    decimals: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PathConfig {
    kind: String,
    path: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AmqpTransportConfig {
    #[serde(default)]
    url: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    reconnect_secs: u64,
    #[serde(default)]
    qos_prefetch: u16,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct WebhookTransportConfig {
    #[serde(default)]
    url: String,
    #[serde(default)]
    timeout_secs: u64,
    #[serde(default)]
    max_retries: u32,
    #[serde(default)]
    retry_base_ms: u64,
}

// ── Pano package namespace config ────────────────────────────────────────

/// Root `[pano]` section. Unknown fields are rejected.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoRootConfig {
    #[serde(default)]
    chains: Vec<String>,
    #[serde(default)]
    assets: Vec<String>,
    #[serde(default)]
    server: PanoServerConfig,
    #[serde(default)]
    detector: PanoDetectorConfig,
    #[serde(default)]
    rpc_defaults: PanoRpcDefaultsConfig,
    #[serde(default)]
    overrides: PanoOverridesConfig,
    #[serde(default)]
    ingress: PanoIngressConfig,
    #[serde(default)]
    egress: PanoEgressConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoServerConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default = "default_bind")]
    bind: String,
    #[serde(default = "default_pano_port")]
    port: u16,
    #[serde(default = "default_prefix")]
    prefix: String,
    #[serde(default)]
    api_key: String,
    #[serde(default)]
    dashboard_path_ref: String,
    #[serde(default)]
    dashboard_export: bool,
    #[serde(default = "default_shutdown_timeout_secs")]
    shutdown_timeout_secs: u64,
}

impl Default for PanoServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: default_bind(),
            port: default_pano_port(),
            prefix: default_prefix(),
            api_key: String::new(),
            dashboard_path_ref: String::new(),
            dashboard_export: false,
            shutdown_timeout_secs: default_shutdown_timeout_secs(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoDetectorConfig {
    #[serde(default = "default_dedup_window_size")]
    dedup_window_size: usize,
    #[serde(default = "default_delivery_workers")]
    delivery_workers: usize,
    #[serde(default = "default_delivery_queue_capacity")]
    delivery_queue_capacity: usize,
    #[serde(default = "default_detector_command_queue_capacity")]
    command_queue_capacity: usize,
    #[serde(default = "default_stale_event_eviction_multiplier")]
    stale_event_eviction_multiplier: u64,
    #[serde(default = "default_stale_event_eviction_min_blocks")]
    stale_event_eviction_min_blocks: u64,
    #[serde(default = "default_max_decimals")]
    max_decimals: u32,
}

impl Default for PanoDetectorConfig {
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

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoRpcDefaultsConfig {
    #[serde(default = "default_max_concurrent")]
    max_concurrent: usize,
    #[serde(default)]
    delay_ms: u64,
    #[serde(default = "default_batch_size")]
    batch_size: u64,
    #[serde(default = "default_true")]
    evm_log_address_batching: bool,
    #[serde(default = "default_scan_lookback_blocks")]
    scan_lookback_blocks: u64,
    #[serde(default = "default_scan_interval_secs")]
    scan_interval_secs: u64,
    #[serde(default = "default_scan_timeout_secs")]
    scan_timeout_secs: u64,
    #[serde(default = "default_max_native_scan_per_cycle")]
    max_native_scan_per_cycle: u64,
    #[serde(default = "default_request_timeout_secs")]
    request_timeout_secs: u64,
    #[serde(default = "default_max_retries")]
    max_retries: u32,
    #[serde(default = "default_retry_base_ms")]
    retry_base_ms: u64,
    #[serde(default)]
    solana_max_supported_transaction_version: u64,
    #[serde(default = "default_solana_scan_mode_str")]
    solana_scan_mode: String,
}

impl Default for PanoRpcDefaultsConfig {
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
            solana_scan_mode: default_solana_scan_mode_str(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoOverridesConfig {
    #[serde(default)]
    chain: PanoOverrideChainConfig,
    #[serde(default)]
    egress: PanoOverrideEgressConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoOverrideChainConfig {
    #[serde(default)]
    assets: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoOverrideEgressConfig {
    #[serde(default)]
    webhook: bool,
    #[serde(default)]
    file: bool,
    #[serde(default)]
    pg: bool,
    #[serde(default)]
    sqlite: bool,
    #[serde(default)]
    queue: bool,
    #[serde(default)]
    http: bool,
}

// ── Pano ingress config ───────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoIngressConfig {
    #[serde(default = "default_ingress_command_queue_capacity")]
    command_queue_capacity: usize,
    #[serde(default)]
    file: PanoIngressFileConfig,
    #[serde(default)]
    http: PanoIngressHttpConfig,
    #[serde(default)]
    sqlite: PanoIngressSqliteConfig,
    #[serde(default)]
    pg: PanoIngressPgConfig,
    #[serde(default)]
    amqp: PanoIngressAmqpConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoIngressFileConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    path_ref: String,
    #[serde(default = "default_file_poll_interval_secs")]
    poll_interval_secs: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoIngressHttpConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default = "default_http_ingress_path")]
    path: String,
    #[serde(default = "default_http_max_body_bytes")]
    max_body_bytes: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoIngressSqliteConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    store: String,
    #[serde(default = "default_sqlite_poll_interval_secs")]
    poll_interval_secs: u64,
    #[serde(default = "default_watched_table")]
    table: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoIngressPgConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    store: String,
    #[serde(default = "default_sqlite_poll_interval_secs")]
    poll_interval_secs: u64,
    #[serde(default = "default_watched_table")]
    table: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoIngressAmqpConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    transport: String,
    #[serde(default = "default_amqp_ingress_exchange")]
    exchange: String,
    #[serde(default = "default_amqp_ingress_routing_key")]
    routing_key: String,
    #[serde(default = "default_pano_ingress_consumer_tag")]
    consumer_tag: String,
    #[serde(default)]
    qos_prefetch: u16,
}

impl Default for PanoIngressConfig {
    fn default() -> Self {
        Self {
            command_queue_capacity: default_ingress_command_queue_capacity(),
            file: PanoIngressFileConfig::default(),
            http: PanoIngressHttpConfig::default(),
            sqlite: PanoIngressSqliteConfig::default(),
            pg: PanoIngressPgConfig::default(),
            amqp: PanoIngressAmqpConfig::default(),
        }
    }
}

// ── Pano egress config ────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoEgressConfig {
    #[serde(default)]
    file: PanoEgressFileConfig,
    #[serde(default)]
    sqlite: PanoEgressSqliteConfig,
    #[serde(default)]
    pg: PanoEgressPgConfig,
    #[serde(default)]
    amqp: PanoEgressAmqpConfig,
    #[serde(default)]
    webhook: PanoEgressWebhookConfig,
    #[serde(default)]
    stream: PanoEgressStreamConfig,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoEgressFileConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    path_ref: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoEgressSqliteConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    store: String,
    #[serde(default = "default_deposit_events_table")]
    table: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoEgressPgConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    store: String,
    #[serde(default = "default_deposit_events_table")]
    table: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoEgressAmqpConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    transport: String,
    #[serde(default = "default_amqp_egress_exchange")]
    exchange: String,
    #[serde(default = "default_amqp_egress_detected_key")]
    detected_routing_key: String,
    #[serde(default = "default_amqp_egress_confirmed_key")]
    confirmed_routing_key: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoEgressWebhookConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    transport: String,
    #[serde(default)]
    secret: String,
    #[serde(default = "default_webhook_signature_header")]
    signature_header: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PanoEgressStreamConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default = "default_sse_path")]
    sse: String,
    #[serde(default = "default_websocket_path")]
    websocket: String,
    #[serde(default = "default_ws_heartbeat_secs")]
    ws_heartbeat_secs: u64,
    #[serde(default = "default_sse_keepalive_secs")]
    sse_keepalive_secs: u64,
    #[serde(default = "default_broadcast_capacity")]
    broadcast_capacity: usize,
}

// ── Application config ────────────────────────────────────────────────────

/// Top-level application configuration. This is the runtime config that all
/// Pano consumers use. It is built from the universal config + `[pano]` namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Default)]
pub struct OverrideChains {
    /// When true, callers may modify any field within chain entries
    /// AND supply custom assets. When false, `assets` must not appear.
    #[serde(default)]
    pub assets: bool,
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

// ── Defaults helpers for string/path defaults ────────────────────────────

fn default_pano_port() -> u16 {
    3210
}
fn default_file_poll_interval_secs() -> u64 {
    5
}
fn default_http_ingress_path() -> String {
    "watch".to_string()
}
fn default_http_max_body_bytes() -> u64 {
    1_048_576
}
fn default_sqlite_poll_interval_secs() -> u64 {
    5
}
fn default_watched_table() -> String {
    "watched_addresses".to_string()
}
fn default_deposit_events_table() -> String {
    "deposit_events".to_string()
}
fn default_amqp_ingress_exchange() -> String {
    "pano.ingress".to_string()
}
fn default_amqp_ingress_routing_key() -> String {
    "watch".to_string()
}
fn default_pano_ingress_consumer_tag() -> String {
    "pano-ingress".to_string()
}
fn default_amqp_egress_exchange() -> String {
    "pano.egress".to_string()
}
fn default_amqp_egress_detected_key() -> String {
    "deposit.detected".to_string()
}
fn default_amqp_egress_confirmed_key() -> String {
    "deposit.confirmed".to_string()
}
fn default_webhook_signature_header() -> String {
    "X-Pano-Signature".to_string()
}
fn default_sse_path() -> String {
    "events".to_string()
}
fn default_websocket_path() -> String {
    "ws".to_string()
}
fn default_ws_heartbeat_secs() -> u64 {
    15
}
fn default_sse_keepalive_secs() -> u64 {
    15
}
fn default_solana_scan_mode_str() -> String {
    "blocks".to_string()
}

// ── Resolution logic ──────────────────────────────────────────────────────

fn resolve_path(cfg: &UniversalConfig, path_ref: &str) -> Result<String> {
    if path_ref.is_empty() {
        return Ok(String::new());
    }
    let profile = cfg.paths.get(path_ref).ok_or_else(|| {
        anyhow::anyhow!("unknown path_ref \"{path_ref}\": no [paths.{path_ref}] section found")
    })?;
    Ok(profile.path.clone())
}

fn resolve_store_url(
    cfg: &UniversalConfig,
    store: &str,
    context: &str,
    require_driver: Option<&str>,
) -> Result<(String, String)> {
    // "driver" and "url"
    if store.is_empty() {
        return Ok((String::new(), String::new()));
    }
    let s = cfg.stores.get(store).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown store \"{store}\" referenced at {context}: no [stores.{store}] section found"
        )
    })?;
    if let Some(required) = require_driver
        && s.driver != required
    {
        anyhow::bail!(
            "store \"{store}\" at {context} has driver \"{}\" but \"{required}\" is required",
            s.driver
        );
    }
    Ok((s.driver.clone(), s.url.clone()))
}

fn resolve_amqp_transport(
    cfg: &UniversalConfig,
    transport: &str,
    context: &str,
) -> Result<AmqpTransportConfig> {
    if transport.is_empty() {
        anyhow::bail!(
            "missing transport at {context}: set transport = \"<id>\" to reference [transports.amqp.<id>]"
        );
    }
    cfg.transports.amqp.get(transport).cloned().ok_or_else(|| {
        anyhow::anyhow!(
            "unknown AMQP transport \"{transport}\" at {context}: no [transports.amqp.{transport}] section found"
        )
    })
}

fn resolve_webhook_transport(
    cfg: &UniversalConfig,
    transport: &str,
    context: &str,
) -> Result<WebhookTransportConfig> {
    if transport.is_empty() {
        anyhow::bail!(
            "missing transport at {context}: set transport = \"<id>\" to reference [transports.webhook.<id>]"
        );
    }
    cfg.transports
        .webhook
        .get(transport)
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!(
                "unknown webhook transport \"{transport}\" at {context}: no [transports.webhook.{transport}] section found"
            )
        })
}

// ── Validation ──────────────────────────────────────────────────────────

impl AppConfig {
    /// Load configuration from a TOML file, resolving environment variable references.
    /// This is the main entry point for the universal namespaced config model.
    pub fn load(path: &str) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config from {path}"))?;
        let resolved = Self::resolve_env_vars(&raw)?;
        let universal: UniversalConfig = toml_edit::de::from_str(&resolved)
            .with_context(|| format!("failed to parse config from {path}"))?;

        let pano = universal.pano.as_ref().ok_or_else(|| {
            anyhow::anyhow!("missing [pano] section in config: Pano requires a [pano] namespace with at least chains defined")
        })?;

        validate_namespace_references(&universal, pano)?;

        // ── Feature-gate checks ──────────────────────────────────────────
        check_feature_server(&pano.server, &pano.ingress, &pano.egress.stream)?;
        check_feature_amqp(&pano.ingress.amqp, &pano.egress.amqp)?;
        check_feature_postgres(&pano.ingress.pg, &pano.egress.pg)?;
        check_feature_webhook(&pano.egress.webhook)?;
        #[cfg(not(feature = "sqlite"))]
        check_feature_sqlite_disabled(&pano.ingress.sqlite, &pano.egress.sqlite)?;

        // ── Build AppConfig from universal + pano namespace ──────────────

        let server = build_server_config(&universal, pano)?;
        let detector = build_detector_config(pano);
        let chains = build_chains(&universal, pano)?;
        let ingress = build_ingress_config(&universal, pano)?;
        let egress = build_egress_config(&universal, pano)?;
        let override_ = build_override_config(pano);

        let config = AppConfig {
            server,
            detector,
            chains,
            ingress,
            egress,
            override_,
        };
        config.validate()?;
        tracing::info!(path, chains = config.chains.len(), "configuration loaded");
        Ok(config)
    }

    /// Replace `${VAR}` and `${VAR:-default}` placeholders with environment
    /// variable values in a single pass. Replacement values are never re-scanned.
    /// TOML comment lines (starting with `#`) are stripped before substitution
    /// to avoid resolving env vars in commented-out sections.
    pub fn resolve_env_vars(input: &str) -> Result<String> {
        // Strip full-line TOML comments (lines starting with optional whitespace then #),
        // but keep empty lines to preserve TOML structure.
        let filtered: String = input
            .lines()
            .filter(|line| {
                let trimmed = line.trim();
                trimmed.is_empty() || !trimmed.starts_with('#')
            })
            .collect::<Vec<_>>()
            .join("\n");

        let re = regex_lite::Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-([^}]*))?\}")
            .context("failed to compile environment variable placeholder regex")?;
        let mut result = String::with_capacity(filtered.len());
        let mut last = 0;
        for m in re.find_iter(&filtered) {
            result.push_str(&filtered[last..m.start()]);
            let full = m.as_str();
            // Strip ${ and }
            let inner = &full[2..full.len() - 1];
            if let Some((var, default)) = inner.split_once(":-") {
                let val = std::env::var(var).unwrap_or_else(|_| default.to_string());
                result.push_str(&val);
            } else {
                let var = inner;
                let val = std::env::var(var).with_context(|| {
                    format!(
                        "environment variable {var} referenced in config is not set (use ${{{var}:-default}} to provide a fallback)"
                    )
                })?;
                result.push_str(&val);
            }
            last = m.end();
        }
        result.push_str(&filtered[last..]);
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
    ///
    /// Table and column names cross a trust boundary: SQL drivers cannot bind
    /// identifiers, so they are interpolated into statements by ingress and
    /// egress modules. Validation here permits only conservative identifiers
    /// before any dynamic SQL is constructed.
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
        if self.ingress.http.enabled && self.ingress.http.max_body_bytes == 0 {
            anyhow::bail!("ingress.http.max_body_bytes must be greater than 0");
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

// ── Feature-gate checks ───────────────────────────────────────────────────

fn validate_namespace_references(universal: &UniversalConfig, pano: &PanoRootConfig) -> Result<()> {
    validate_unique_refs("pano.chains", &pano.chains)?;
    validate_unique_refs("pano.assets", &pano.assets)?;

    if !matches!(
        pano.rpc_defaults.solana_scan_mode.as_str(),
        "blocks" | "signatures"
    ) {
        anyhow::bail!("pano.rpc_defaults.solana_scan_mode must be \"blocks\" or \"signatures\"");
    }

    for chain_id in &pano.chains {
        let chain = universal.chains.get(chain_id).ok_or_else(|| {
            anyhow::anyhow!(
                "pano.chains references unknown chain \"{chain_id}\": no [chains.{chain_id}] section found"
            )
        })?;
        if chain.caip2.trim().is_empty() {
            anyhow::bail!("[chains.{chain_id}].caip2 must not be empty");
        }
    }

    for asset_id in &pano.assets {
        let asset = universal.assets.get(asset_id).ok_or_else(|| {
            anyhow::anyhow!(
                "pano.assets references unknown asset \"{asset_id}\": no [assets.{asset_id}] section found"
            )
        })?;
        if !pano.chains.iter().any(|chain_id| chain_id == &asset.chain) {
            anyhow::bail!(
                "pano.assets reference \"{asset_id}\" belongs to chain \"{}\", which is not listed in pano.chains",
                asset.chain
            );
        }
    }

    validate_enabled_store(
        universal,
        pano.ingress.sqlite.enabled,
        &pano.ingress.sqlite.store,
        "sqlite",
        "pano.ingress.sqlite.store",
    )?;
    validate_enabled_store(
        universal,
        pano.egress.sqlite.enabled,
        &pano.egress.sqlite.store,
        "sqlite",
        "pano.egress.sqlite.store",
    )?;
    validate_enabled_store(
        universal,
        pano.ingress.pg.enabled,
        &pano.ingress.pg.store,
        "postgres",
        "pano.ingress.pg.store",
    )?;
    validate_enabled_store(
        universal,
        pano.egress.pg.enabled,
        &pano.egress.pg.store,
        "postgres",
        "pano.egress.pg.store",
    )?;

    if pano.ingress.file.enabled {
        validate_file_path_reference(universal, &pano.ingress.file.path_ref, "pano.ingress.file")?;
    }
    if pano.egress.file.enabled {
        validate_file_path_reference(universal, &pano.egress.file.path_ref, "pano.egress.file")?;
    }
    if !pano.server.dashboard_path_ref.is_empty() {
        let path = universal
            .paths
            .get(&pano.server.dashboard_path_ref)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown dashboard_path_ref \"{}\": no [paths.{}] section found",
                    pano.server.dashboard_path_ref,
                    pano.server.dashboard_path_ref
                )
            })?;
        if path.path.trim().is_empty() {
            anyhow::bail!(
                "[paths.{}].path must not be empty",
                pano.server.dashboard_path_ref
            );
        }
    }
    Ok(())
}

fn validate_unique_refs(context: &str, refs: &[String]) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    for reference in refs {
        if reference.trim().is_empty() {
            anyhow::bail!("{context} must not contain an empty reference");
        }
        if !seen.insert(reference) {
            anyhow::bail!("{context} contains duplicate reference \"{reference}\"");
        }
    }
    Ok(())
}

fn validate_enabled_store(
    universal: &UniversalConfig,
    enabled: bool,
    store_id: &str,
    expected_driver: &str,
    context: &str,
) -> Result<()> {
    if !enabled {
        return Ok(());
    }
    let store = universal.stores.get(store_id).ok_or_else(|| {
        anyhow::anyhow!("unknown store \"{store_id}\" referenced at {context}: no [stores.{store_id}] section found")
    })?;
    if store.driver != expected_driver {
        anyhow::bail!(
            "store \"{store_id}\" at {context} has driver \"{}\" but \"{expected_driver}\" is required",
            store.driver
        );
    }
    if store.url.trim().is_empty() {
        anyhow::bail!("store \"{store_id}\" at {context} has an empty url");
    }
    Ok(())
}

fn validate_file_path_reference(
    universal: &UniversalConfig,
    path_ref: &str,
    context: &str,
) -> Result<()> {
    if path_ref.trim().is_empty() {
        anyhow::bail!("{context} requires path or path_ref referencing [paths.<id>]");
    }
    let path = universal.paths.get(path_ref).ok_or_else(|| {
        anyhow::anyhow!("unknown path_ref \"{path_ref}\": no [paths.{path_ref}] section found")
    })?;
    if path.kind != "file" {
        anyhow::bail!("[paths.{path_ref}].kind must be \"file\" when referenced by {context}");
    }
    if path.path.trim().is_empty() {
        anyhow::bail!("[paths.{path_ref}].path must not be empty when referenced by {context}");
    }
    Ok(())
}

fn check_feature_server(
    server: &PanoServerConfig,
    ingress: &PanoIngressConfig,
    stream: &PanoEgressStreamConfig,
) -> Result<()> {
    #[cfg(not(feature = "server"))]
    {
        if server.enabled {
            anyhow::bail!(
                "pano.server.enabled requires feature \"server\" (rebuild with --features server)"
            );
        }
        if ingress.http.enabled {
            anyhow::bail!(
                "pano.ingress.http.enabled requires feature \"server\" (rebuild with --features server)"
            );
        }
        if stream.enabled {
            anyhow::bail!(
                "pano.egress.stream.enabled requires feature \"server\" (rebuild with --features server)"
            );
        }
    }
    let _ = (server, ingress, stream);
    Ok(())
}

fn check_feature_amqp(
    ingress_amqp: &PanoIngressAmqpConfig,
    egress_amqp: &PanoEgressAmqpConfig,
) -> Result<()> {
    #[cfg(not(feature = "amqp"))]
    {
        if ingress_amqp.enabled {
            anyhow::bail!(
                "pano.ingress.amqp.enabled requires feature \"amqp\" (rebuild with --features amqp)"
            );
        }
        if egress_amqp.enabled {
            anyhow::bail!(
                "pano.egress.amqp.enabled requires feature \"amqp\" (rebuild with --features amqp)"
            );
        }
    }
    let _ = (ingress_amqp, egress_amqp);
    Ok(())
}

fn check_feature_postgres(
    ingress_pg: &PanoIngressPgConfig,
    egress_pg: &PanoEgressPgConfig,
) -> Result<()> {
    #[cfg(not(feature = "postgres"))]
    {
        if ingress_pg.enabled {
            anyhow::bail!(
                "pano.ingress.pg.enabled requires feature \"postgres\" (rebuild with --features postgres)"
            );
        }
        if egress_pg.enabled {
            anyhow::bail!(
                "pano.egress.pg.enabled requires feature \"postgres\" (rebuild with --features postgres)"
            );
        }
    }
    let _ = (ingress_pg, egress_pg);
    Ok(())
}

fn check_feature_webhook(webhook: &PanoEgressWebhookConfig) -> Result<()> {
    #[cfg(not(feature = "webhook"))]
    {
        if webhook.enabled {
            anyhow::bail!(
                "pano.egress.webhook.enabled requires feature \"webhook\" (rebuild with --features webhook)"
            );
        }
    }
    let _ = webhook;
    Ok(())
}

#[cfg(not(feature = "sqlite"))]
fn check_feature_sqlite_disabled(
    ingress_sqlite: &PanoIngressSqliteConfig,
    egress_sqlite: &PanoEgressSqliteConfig,
) -> Result<()> {
    if ingress_sqlite.enabled {
        anyhow::bail!(
            "pano.ingress.sqlite.enabled requires feature \"sqlite\" (sqlite is the default; rebuild without --no-default-features)"
        );
    }
    if egress_sqlite.enabled {
        anyhow::bail!(
            "pano.egress.sqlite.enabled requires feature \"sqlite\" (sqlite is the default; rebuild without --no-default-features)"
        );
    }
    Ok(())
}

// ── Build helpers: universal + pano → AppConfig ──────────────────────────

fn build_server_config(universal: &UniversalConfig, pano: &PanoRootConfig) -> Result<ServerConfig> {
    let s = &pano.server;
    let mut dashboard = String::new();
    if !s.dashboard_path_ref.is_empty() {
        let path = resolve_path(universal, &s.dashboard_path_ref)?;
        dashboard = path;
    }

    let _ = universal;

    Ok(ServerConfig {
        enabled: s.enabled,
        bind: s.bind.clone(),
        port: s.port,
        prefix: s.prefix.clone(),
        dashboard,
        dashboard_export: s.dashboard_export,
        api_key: s.api_key.clone(),
        shutdown_timeout_secs: s.shutdown_timeout_secs,
    })
}

fn build_detector_config(pano: &PanoRootConfig) -> DetectorConfig {
    let d = &pano.detector;
    DetectorConfig {
        dedup_window_size: d.dedup_window_size,
        delivery_workers: d.delivery_workers,
        delivery_queue_capacity: d.delivery_queue_capacity,
        command_queue_capacity: d.command_queue_capacity,
        stale_event_eviction_multiplier: d.stale_event_eviction_multiplier,
        stale_event_eviction_min_blocks: d.stale_event_eviction_min_blocks,
        max_decimals: d.max_decimals,
    }
}

fn build_chains(universal: &UniversalConfig, pano: &PanoRootConfig) -> Result<Vec<ChainConfig>> {
    let rpc_defaults = &pano.rpc_defaults;
    let mut chains = Vec::new();

    for chain_id in &pano.chains {
        let shared = universal.chains.get(chain_id).ok_or_else(|| {
            anyhow::anyhow!(
                "pano.chains references unknown chain \"{chain_id}\": no [chains.{chain_id}] section found"
            )
        })?;

        let rpc_options = RpcOptions {
            max_concurrent: rpc_defaults.max_concurrent,
            delay_ms: rpc_defaults.delay_ms,
            batch_size: rpc_defaults.batch_size,
            evm_log_address_batching: rpc_defaults.evm_log_address_batching,
            scan_lookback_blocks: rpc_defaults.scan_lookback_blocks,
            scan_interval_secs: rpc_defaults.scan_interval_secs,
            scan_timeout_secs: rpc_defaults.scan_timeout_secs,
            max_native_scan_per_cycle: rpc_defaults.max_native_scan_per_cycle,
            request_timeout_secs: rpc_defaults.request_timeout_secs,
            max_retries: rpc_defaults.max_retries,
            retry_base_ms: rpc_defaults.retry_base_ms,
            solana_max_supported_transaction_version: rpc_defaults
                .solana_max_supported_transaction_version,
            solana_scan_mode: match rpc_defaults.solana_scan_mode.as_str() {
                "signatures" => SolanaScanMode::Signatures,
                "blocks" => SolanaScanMode::Blocks,
                _ => unreachable!("validated before building chain configuration"),
            },
        };

        // Collect assets from shared [assets.<id>] that belong to this chain
        let mut chain_assets = Vec::new();
        for asset_id in &pano.assets {
            let shared_asset = universal.assets.get(asset_id).ok_or_else(|| {
                anyhow::anyhow!(
                    "pano.assets references unknown asset \"{asset_id}\": no [assets.{asset_id}] section found"
                )
            })?;
            if shared_asset.chain == *chain_id && shared_asset.enabled {
                chain_assets.push(AssetConfig {
                    symbol: shared_asset.symbol.clone(),
                    contract: shared_asset.contract.clone(),
                    token_program: None,
                    decimals: shared_asset.decimals,
                    min_amount: None,
                });
            }
        }

        if chain_assets.is_empty() {
            anyhow::bail!(
                "chain \"{chain_id}\" has no enabled assets in pano.assets; add at least one [assets.<id>] with chain = \"{chain_id}\" and include it in pano.assets"
            );
        }

        chains.push(ChainConfig {
            caip2: shared.caip2.clone(),
            start_block: None,
            end_block: None,
            confirmed_blocks: shared.confirmations,
            rpc: shared.rpc_urls.clone(),
            rpc_options: Some(rpc_options),
            assets: chain_assets,
        });
    }

    Ok(chains)
}

fn build_ingress_config(
    universal: &UniversalConfig,
    pano: &PanoRootConfig,
) -> Result<IngressConfig> {
    let pi = &pano.ingress;

    // File ingress
    let file_path = resolve_path(universal, &pi.file.path_ref)?;
    let fi = FileIngressConfig {
        enabled: pi.file.enabled,
        path: file_path,
        poll_interval_secs: pi.file.poll_interval_secs,
        authoritative: true,
    };

    // HTTP ingress
    let hi = HttpIngressConfig {
        enabled: pi.http.enabled,
        addresses: pi.http.path.clone(),
        max_body_bytes: pi.http.max_body_bytes,
    };

    // SQLite ingress
    let si = if pi.sqlite.enabled {
        let (_, sqlite_url) = resolve_store_url(
            universal,
            &pi.sqlite.store,
            "pano.ingress.sqlite.store",
            Some("sqlite"),
        )?;
        SqliteIngressConfig {
            enabled: true,
            path: extract_sqlite_path(&sqlite_url),
            poll_interval_secs: pi.sqlite.poll_interval_secs,
            table: SqliteIngressTable {
                name: pi.sqlite.table.clone(),
                columns: SqliteIngressColumns::default(),
            },
        }
    } else {
        SqliteIngressConfig::default()
    };

    // PG ingress
    let pgi = if pi.pg.enabled {
        let (_pg_driver, pg_url) = resolve_store_url(
            universal,
            &pi.pg.store,
            "pano.ingress.pg.store",
            Some("postgres"),
        )?;
        PgIngressConfig {
            enabled: true,
            url: pg_url,
            poll_interval_secs: pi.pg.poll_interval_secs,
            table: PgIngressTable {
                name: pi.pg.table.clone(),
                columns: PgIngressColumns::default(),
            },
        }
    } else {
        PgIngressConfig::default()
    };

    // AMQP ingress
    let qi = if pi.amqp.enabled {
        let amqp = resolve_amqp_transport(universal, &pi.amqp.transport, "pano.ingress.amqp")?;
        let qos = if pi.amqp.qos_prefetch > 0 {
            pi.amqp.qos_prefetch
        } else {
            amqp.qos_prefetch
        };
        QueueIngressConfig {
            enabled: true,
            url: amqp.url,
            username: amqp.username,
            password: amqp.password,
            exchange: pi.amqp.exchange.clone(),
            watch_routing_key: pi.amqp.routing_key.clone(),
            unwatch_routing_key: String::new(),
            reconnect_secs: amqp.reconnect_secs,
            qos_prefetch: qos,
            consumer_tag: pi.amqp.consumer_tag.clone(),
        }
    } else {
        QueueIngressConfig::default()
    };

    Ok(IngressConfig {
        file: fi,
        sqlite: si,
        pg: pgi,
        queue: qi,
        http: hi,
        command_queue_capacity: pi.command_queue_capacity,
    })
}

fn build_egress_config(universal: &UniversalConfig, pano: &PanoRootConfig) -> Result<EgressConfig> {
    let pe = &pano.egress;

    // File egress
    let file_path = resolve_path(universal, &pe.file.path_ref)?;
    let fe = FileEgressConfig {
        enabled: pe.file.enabled,
        path: file_path,
    };

    // SQLite egress
    let se = if pe.sqlite.enabled {
        let (_, sqlite_url) = resolve_store_url(
            universal,
            &pe.sqlite.store,
            "pano.egress.sqlite.store",
            Some("sqlite"),
        )?;
        SqliteEgressConfig {
            enabled: true,
            path: extract_sqlite_path(&sqlite_url),
            table: SqliteEgressTable {
                name: pe.sqlite.table.clone(),
                columns: SqliteEgressColumns::default(),
            },
        }
    } else {
        SqliteEgressConfig::default()
    };

    // PG egress
    let pge = if pe.pg.enabled {
        let (_pg_driver, pg_url) = resolve_store_url(
            universal,
            &pe.pg.store,
            "pano.egress.pg.store",
            Some("postgres"),
        )?;
        PgEgressConfig {
            enabled: true,
            url: pg_url,
            table: PgEgressTable {
                name: pe.pg.table.clone(),
                columns: PgEgressColumns::default(),
            },
        }
    } else {
        PgEgressConfig::default()
    };

    // AMQP egress
    let qe = if pe.amqp.enabled {
        let amqp = resolve_amqp_transport(universal, &pe.amqp.transport, "pano.egress.amqp")?;
        QueueEgressConfig {
            enabled: true,
            url: amqp.url,
            username: amqp.username,
            password: amqp.password,
            exchange: pe.amqp.exchange.clone(),
            detected_routing_key: pe.amqp.detected_routing_key.clone(),
            confirmed_routing_key: pe.amqp.confirmed_routing_key.clone(),
            reconnect_secs: amqp.reconnect_secs,
        }
    } else {
        QueueEgressConfig::default()
    };

    // Webhook egress
    let we = if pe.webhook.enabled {
        let wh =
            resolve_webhook_transport(universal, &pe.webhook.transport, "pano.egress.webhook")?;
        WebhookEgressConfig {
            enabled: true,
            url: wh.url,
            secret: pe.webhook.secret.clone(),
            signature_header: pe.webhook.signature_header.clone(),
            max_retries: wh.max_retries,
            retry_base_ms: wh.retry_base_ms,
            timeout_secs: wh.timeout_secs,
        }
    } else {
        WebhookEgressConfig::default()
    };

    // Stream egress (SSE/WS)
    let broadcast_capacity = pe.stream.broadcast_capacity.max(1);
    let he = HttpEgressConfig {
        enabled: pe.stream.enabled,
        sse: pe.stream.sse.clone(),
        websocket: pe.stream.websocket.clone(),
        ws_heartbeat_secs: pe.stream.ws_heartbeat_secs.max(1),
        sse_keepalive_secs: pe.stream.sse_keepalive_secs.max(1),
        ws_max_message_size: 64 * 1024,
        ws_max_frame_size: 64 * 1024,
    };

    Ok(EgressConfig {
        file: fe,
        sqlite: se,
        pg: pge,
        queue: qe,
        broadcast_capacity,
        http: he,
        webhook: we,
    })
}

fn build_override_config(pano: &PanoRootConfig) -> OverrideConfig {
    let chain_override = OverrideChains {
        assets: pano.overrides.chain.assets,
    };
    let chains_enabled = chain_override.assets;
    let chains = if chains_enabled {
        Some(chain_override)
    } else {
        None
    };

    OverrideConfig {
        chains,
        egress: OverrideEgress {
            webhook: pano.overrides.egress.webhook,
            file: pano.overrides.egress.file,
            pg: pano.overrides.egress.pg,
            sqlite: pano.overrides.egress.sqlite,
            queue: pano.overrides.egress.queue,
            http: pano.overrides.egress.http,
        },
    }
}

/// Extract a filesystem path from a `sqlite://...` URL.
fn extract_sqlite_path(url: &str) -> String {
    if let Some(path) = url.strip_prefix("sqlite://") {
        path.to_string()
    } else {
        url.to_string()
    }
}
