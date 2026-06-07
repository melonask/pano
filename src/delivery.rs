use crate::config::QueueEgressConfig;
use crate::egress::{file, pg, queue, sqlite, webhook};
use crate::model::DepositEvent;
use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Normalize a connection URL for use as a pool cache key.
/// Strips trailing slashes, sorts query parameters, and lowercases
/// the host. Does NOT normalize credentials (those are identity-sensitive).
pub fn normalize_pool_key(raw_url: &str) -> String {
    match url::Url::parse(raw_url) {
        Ok(mut parsed) => {
            if parsed.path().ends_with('/') && parsed.path().len() > 1 {
                let trimmed = parsed.path().trim_end_matches('/').to_string();
                parsed.set_path(&trimmed);
            }
            let mut pairs: Vec<(String, String)> = parsed
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
            parsed.set_query(None);
            if !pairs.is_empty() {
                let mut query_writer = parsed.query_pairs_mut();
                for (k, v) in pairs {
                    query_writer.append_pair(&k, &v);
                }
            }
            if let Some(host) = parsed.host_str() {
                parsed.set_host(Some(&host.to_lowercase())).ok();
            }
            parsed.to_string()
        }
        Err(_) => raw_url.to_string(),
    }
}

/// Handles per-address egress overrides with connection pool reuse.
///
/// Maintains caches for PostgreSQL connection pools keyed by database URL
/// and SQLite connection pools keyed by database path so that repeated
/// delivery to the same target does not create new connections each time.
/// Pool keys are normalized via `normalize_pool_key` to prevent duplicates
/// from URL variants.
#[derive(Debug, Clone)]
pub struct EgressRouter {
    /// Shared HTTP client for webhook override delivery.
    http: reqwest::Client,
    /// PostgreSQL connection pools keyed by normalized database URL.
    pg_pools: Arc<RwLock<hashbrown::HashMap<String, sqlx::PgPool>>>,
    /// SQLite connection pools keyed by normalized database path.
    sqlite_pools: Arc<RwLock<hashbrown::HashMap<String, sqlx::SqlitePool>>>,
    /// AMQP connections keyed by normalized `url|exchange`.
    queue_connections: Arc<RwLock<hashbrown::HashMap<String, queue::QueueConnection>>>,
    /// Per-path locks for JSON file egress override writes.
    file_locks: file::FileWriteLocks,
}

impl Default for EgressRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl EgressRouter {
    /// Create a new empty egress router.
    pub fn new() -> Self {
        Self::with_http_timeout_secs(crate::config::WebhookEgressConfig::default().timeout_secs)
    }

    pub fn with_http_timeout_secs(timeout_secs: u64) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_secs.max(1)))
            .build()
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "failed to build configured egress HTTP client; using default client");
                reqwest::Client::new()
            });
        Self {
            http,
            pg_pools: Arc::new(RwLock::new(hashbrown::HashMap::new())),
            sqlite_pools: Arc::new(RwLock::new(hashbrown::HashMap::new())),
            queue_connections: Arc::new(RwLock::new(hashbrown::HashMap::new())),
            file_locks: file::shared_write_locks(),
        }
    }

    /// Dispatch a deposit event to the configured per-address egress override.
    /// Routes all non-None override channels additively.
    pub async fn route(&self, event: &DepositEvent) {
        let Some(egress) = event.data.internal_egress.as_ref() else {
            return;
        };

        if let Some(webhook_override) = &egress.webhook {
            self.route_webhook(webhook_override, event).await;
        }
        if let Some(file_override) = &egress.file {
            self.route_file(file_override, event).await;
        }
        if let Some(sqlite_override) = &egress.sqlite {
            self.route_sqlite(sqlite_override, event).await;
        }
        if let Some(pg_override) = &egress.pg {
            self.route_pg(pg_override, event).await;
        }
        if let Some(queue_override) = &egress.queue {
            self.route_queue(queue_override, event).await;
        }
        // http SSE/WS override is not applied per-address — those are server-level.
    }

    async fn route_file(&self, file_override: &crate::model::FileOverride, event: &DepositEvent) {
        if let Err(e) =
            file::write_event_to_path_with_locks(&self.file_locks, &file_override.path, event).await
        {
            tracing::error!(error = %e, event_id = %event.event_id, path = %file_override.path, "file egress override failed");
        }
    }

    async fn route_sqlite(
        &self,
        sqlite_override: &crate::model::SqliteOverride,
        event: &DepositEvent,
    ) {
        let pool = self.get_sqlite_pool(&sqlite_override.path).await;
        match pool {
            Ok(pool) => {
                let table = sqlite_override
                    .table
                    .as_ref()
                    .map(|t| crate::egress::sqlite::SqliteEgressTable {
                        name: t.name.clone(),
                        ..Default::default()
                    })
                    .unwrap_or_default();
                if let Err(e) = sqlite::ensure_schema(&pool, &table).await {
                    tracing::error!(error = %e, event_id = %event.event_id, path = %sqlite_override.path, table = %table.name, "sqlite egress override failed to ensure schema");
                    return;
                }
                if let Err(e) = sqlite::insert_event(&pool, event, &table).await {
                    tracing::error!(error = %e, event_id = %event.event_id, path = %sqlite_override.path, "sqlite egress override failed");
                }
            }
            Err(e) => {
                tracing::error!(error = %e, event_id = %event.event_id, path = %sqlite_override.path, "sqlite egress override failed to open database");
            }
        }
    }

    async fn route_pg(&self, pg_override: &crate::model::PgOverride, event: &DepositEvent) {
        let pool = self.get_pg_pool(&pg_override.url).await;
        match pool {
            Ok(pool) => {
                let table = pg_override
                    .table
                    .as_ref()
                    .map(|t| crate::egress::pg::PgEgressTable {
                        name: t.name.clone(),
                        ..Default::default()
                    })
                    .unwrap_or_default();
                if let Err(e) = pg::ensure_schema(&pool, &table).await {
                    tracing::error!(error = %e, event_id = %event.event_id, url = %pg_override.url, table = %table.name, "pg egress override failed to ensure schema");
                    return;
                }
                if let Err(e) = pg::insert_event(&pool, event, &table).await {
                    tracing::error!(error = %e, event_id = %event.event_id, "pg egress override failed");
                }
            }
            Err(e) => {
                tracing::error!(error = %e, event_id = %event.event_id, url = %pg_override.url, "pg egress override failed to connect");
            }
        }
    }

    async fn route_queue(
        &self,
        queue_override: &crate::model::QueueOverride,
        event: &DepositEvent,
    ) {
        let url = queue_override.url.as_str();
        let exchange = queue_override.exchange.as_deref().unwrap_or("");
        if url.is_empty() || exchange.is_empty() {
            tracing::warn!(event_id = %event.event_id, "queue egress override missing required options: url and exchange");
            return;
        }
        let key = format!("{}|{}", normalize_pool_key(url), exchange);
        let result = self.publish_with_key(&key, url, exchange, event).await;
        match result {
            Ok(()) => return,
            Err(e) => {
                tracing::warn!(error = %e, event_id = %event.event_id, "queue egress publish failed, evicting stale connection and retrying");
            }
        }
        self.queue_connections.write().await.remove(&key);
        match self.publish_with_key(&key, url, exchange, event).await {
            Ok(()) => {}
            Err(e) => {
                tracing::error!(error = %e, event_id = %event.event_id, "queue egress override failed after reconnect retry");
            }
        }
    }

    async fn publish_with_key(
        &self,
        key: &str,
        url: &str,
        exchange: &str,
        event: &DepositEvent,
    ) -> Result<()> {
        let conn = self.get_queue_connection(key, url, exchange).await?;
        conn.publish(event).await
    }

    async fn route_webhook(
        &self,
        webhook_override: &crate::model::WebhookOverride,
        event: &DepositEvent,
    ) {
        if webhook_override.url.is_empty() {
            tracing::warn!(event_id = %event.event_id, "webhook egress override missing required option: url");
            return;
        }
        let cfg = crate::config::WebhookEgressConfig {
            enabled: true,
            url: webhook_override.url.clone(),
            secret: webhook_override.secret.clone(),
            ..Default::default()
        };
        if let Err(e) = webhook::deliver_single_with_client(
            &self.http,
            &webhook_override.url,
            &webhook_override.secret,
            event,
            &cfg,
        )
        .await
        {
            tracing::error!(error = %e, event_id = %event.event_id, url = %webhook_override.url, "webhook egress override failed");
        }
    }

    /// Get or create a PostgreSQL connection pool for the given URL.
    async fn get_pg_pool(&self, raw_url: &str) -> Result<sqlx::PgPool> {
        let key = normalize_pool_key(raw_url);
        {
            let pools = self.pg_pools.read().await;
            if let Some(pool) = pools.get(&key) {
                return Ok(pool.clone());
            }
        }
        let mut pools = self.pg_pools.write().await;
        if let Some(existing) = pools.get(&key) {
            return Ok(existing.clone());
        }
        let pool = crate::shared::db::connect_pg(raw_url).await?;
        pools.insert(key, pool.clone());
        Ok(pool)
    }

    /// Get or create a SQLite connection pool for the given path.
    async fn get_sqlite_pool(&self, raw_path: &str) -> Result<sqlx::SqlitePool> {
        let key = normalize_pool_key(raw_path);
        {
            let pools = self.sqlite_pools.read().await;
            if let Some(pool) = pools.get(&key) {
                return Ok(pool.clone());
            }
        }
        let mut pools = self.sqlite_pools.write().await;
        if let Some(existing) = pools.get(&key) {
            return Ok(existing.clone());
        }
        let pool = open_sqlite_pool_for_router(raw_path).await?;
        pools.insert(key, pool.clone());
        Ok(pool)
    }

    /// Get or create an AMQP queue connection for the given key.
    async fn get_queue_connection(
        &self,
        key: &str,
        url: &str,
        exchange: &str,
    ) -> Result<queue::QueueConnection> {
        {
            let conns = self.queue_connections.read().await;
            if let Some(conn) = conns.get(key) {
                return Ok(conn.clone());
            }
        }
        let cfg = QueueEgressConfig {
            enabled: true,
            url: url.to_string(),
            exchange: exchange.to_string(),
            ..QueueEgressConfig::default()
        };
        let conn = queue::QueueConnection::connect(cfg).await?;
        let mut conns = self.queue_connections.write().await;
        if let Some(existing) = conns.get(key) {
            return Ok(existing.clone());
        }
        conns.insert(key.to_owned(), conn.clone());
        Ok(conn)
    }
}

/// Open a SQLite connection pool using the filename-based approach.
///
/// Uses `SqliteConnectOptions::filename()` to avoid URI-parsing ambiguity
/// with absolute paths on different platforms.
async fn open_sqlite_pool_for_router(path: &str) -> Result<sqlx::SqlitePool> {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .with_context(|| format!("failed to open sqlite database at {path}"))?;

    sqlx::query("PRAGMA journal_mode = WAL;")
        .execute(&pool)
        .await
        .context("failed to enable sqlite WAL mode")?;
    sqlx::query("PRAGMA synchronous = NORMAL;")
        .execute(&pool)
        .await
        .context("failed to set sqlite synchronous=NORMAL")?;

    Ok(pool)
}
