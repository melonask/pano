use serde::{Deserialize, Serialize};

// ── Ingress PostgreSQL configuration ────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PgIngressConfig {
    pub enabled: bool,
    pub url: String,
    #[serde(default = "default_poll_interval_secs")]
    pub poll_interval_secs: u64,
    pub table: PgIngressTable,
}

impl Default for PgIngressConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: String::new(),
            poll_interval_secs: default_poll_interval_secs(),
            table: PgIngressTable::default(),
        }
    }
}

fn default_poll_interval_secs() -> u64 {
    5
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PgIngressTable {
    pub name: String,
    pub columns: PgIngressColumns,
}

impl Default for PgIngressTable {
    fn default() -> Self {
        Self {
            name: "watched_addresses".to_string(),
            columns: PgIngressColumns::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PgIngressColumns {
    pub address: String,
    pub caip2: String,
    pub symbol: String,
    pub asset_config: String,
    pub chain_config: String,
    pub egress: String,
}

impl Default for PgIngressColumns {
    fn default() -> Self {
        Self {
            address: "address".to_string(),
            caip2: "caip2".to_string(),
            symbol: "symbol".to_string(),
            asset_config: "asset_config".to_string(),
            chain_config: "chain_config".to_string(),
            egress: "egress".to_string(),
        }
    }
}

// ── Implementation (requires postgres feature) ──────────────────────────
//
// Table configuration is an administrative trust boundary. PostgreSQL does
// not support bound identifiers, so validated table and column identifiers
// are interpolated below; values remain bound through sqlx.

#[cfg(feature = "postgres")]
mod imp {
    use super::{PgIngressConfig, PgIngressTable};
    use crate::ingress::db::{WatchedAddressRow, WatchedAddressTuple, into_resolved};
    use crate::model::{Command, ResolvedWatch};
    use crate::shared::db as shared_db;
    use anyhow::{Context, Result};
    use tokio::sync::mpsc;

    /// Load watched addresses from a PostgreSQL database once and exit.
    pub async fn load(config: PgIngressConfig, tx: mpsc::Sender<Command>) -> Result<()> {
        let pool = shared_db::connect_pg(&config.url).await?;
        let rows = load_addresses(&pool, &config.table).await?;
        let resolved: Vec<ResolvedWatch> =
            rows.into_iter().map(into_resolved).collect::<Result<_>>()?;
        if !resolved.is_empty() {
            let _ = tx.send(Command::SyncAll(resolved)).await;
        }
        Ok(())
    }

    /// Poll a PostgreSQL database for watched address changes at runtime.
    pub async fn watch(config: PgIngressConfig, tx: mpsc::Sender<Command>) -> Result<()> {
        let poll_interval = std::time::Duration::from_secs(config.poll_interval_secs.max(1));
        let pool = shared_db::connect_pg(&config.url).await?;
        let mut last_rows: Option<Vec<ResolvedWatch>> = None;

        loop {
            if tx.is_closed() {
                return Ok(());
            }
            match load_addresses(&pool, &config.table).await {
                Ok(rows) => {
                    let resolved: Vec<ResolvedWatch> =
                        rows.into_iter().map(into_resolved).collect::<Result<_>>()?;
                    if last_rows.as_ref() != Some(&resolved) {
                        if tx.send(Command::SyncAll(resolved.clone())).await.is_err() {
                            return Ok(());
                        }
                        last_rows = Some(resolved);
                    }
                }
                Err(e) => tracing::error!(error = %e, "failed to poll pg ingress"),
            }
            tokio::select! {
                _ = tx.closed() => return Ok(()),
                _ = tokio::time::sleep(poll_interval) => {}
            }
        }
    }

    async fn load_addresses(
        pool: &sqlx::PgPool,
        table: &PgIngressTable,
    ) -> Result<Vec<WatchedAddressRow>> {
        let query = format!(
            "SELECT {}, {}, {}, {}, {}, {} FROM {} ORDER BY {}, {}, {}",
            table.columns.address,
            table.columns.caip2,
            table.columns.symbol,
            table.columns.asset_config,
            table.columns.chain_config,
            table.columns.egress,
            table.name,
            table.columns.address,
            table.columns.caip2,
            table.columns.symbol
        );
        let rows: Vec<WatchedAddressTuple> = sqlx::query_as(sqlx::AssertSqlSafe(query))
            .fetch_all(pool)
            .await
            .with_context(|| format!("failed to query {}", table.name))?;

        Ok(rows.into_iter().map(WatchedAddressRow::from).collect())
    }
}

#[cfg(feature = "postgres")]
pub use imp::{load, watch};
