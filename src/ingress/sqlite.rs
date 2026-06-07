use crate::ingress::db::{WatchedAddressRow, WatchedAddressTuple, into_resolved};
use crate::model::{Command, ResolvedWatch};
use crate::shared::db as shared_db;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

// ── Ingress SQLite configuration ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SqliteIngressConfig {
    pub enabled: bool,
    pub path: String,
    pub poll_interval_secs: u64,
    pub table: SqliteIngressTable,
}

impl Default for SqliteIngressConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: String::new(),
            poll_interval_secs: 5,
            table: SqliteIngressTable::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SqliteIngressTable {
    pub name: String,
    pub columns: SqliteIngressColumns,
}

impl Default for SqliteIngressTable {
    fn default() -> Self {
        Self {
            name: "watched_addresses".to_string(),
            columns: SqliteIngressColumns::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SqliteIngressColumns {
    pub address: String,
    pub caip2: String,
    pub symbol: String,
    pub asset_config: String,
    pub chain_config: String,
    pub egress: String,
}

impl Default for SqliteIngressColumns {
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

// ── Implementation ──────────────────────────────────────────────────────

/// Load watched addresses from a SQLite database once and exit.
pub async fn load(config: SqliteIngressConfig, tx: mpsc::Sender<Command>) -> Result<()> {
    let pool = shared_db::open_sqlite_pool(&config.path, 1).await?;
    let rows = load_addresses(&pool, &config.table).await?;
    let resolved: Vec<ResolvedWatch> =
        rows.into_iter().map(into_resolved).collect::<Result<_>>()?;
    if !resolved.is_empty() {
        let _ = tx.send(Command::SyncAll(resolved)).await;
    }
    Ok(())
}

/// Poll a SQLite database for watched address changes at runtime.
pub async fn watch(config: SqliteIngressConfig, tx: mpsc::Sender<Command>) -> Result<()> {
    let poll_interval = std::time::Duration::from_secs(config.poll_interval_secs.max(1));
    let pool = shared_db::open_sqlite_pool(&config.path, 1).await?;
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
            Err(e) => {
                tracing::error!(error = %e, path = %config.path, "failed to poll sqlite ingress")
            }
        }
        tokio::select! {
            _ = tx.closed() => return Ok(()),
            _ = tokio::time::sleep(poll_interval) => {}
        }
    }
}

async fn load_addresses(
    pool: &sqlx::SqlitePool,
    table: &SqliteIngressTable,
) -> Result<Vec<WatchedAddressRow>> {
    ensure_schema(pool, table).await?;

    let query = format!(
        "SELECT {addr}, {caip2}, {symbol}, {asset_config}, {chain_config}, {egress} FROM {table_name} ORDER BY {addr}, {caip2}, {symbol}",
        addr = table.columns.address,
        caip2 = table.columns.caip2,
        symbol = table.columns.symbol,
        asset_config = table.columns.asset_config,
        chain_config = table.columns.chain_config,
        egress = table.columns.egress,
        table_name = table.name,
    );
    let rows: Vec<WatchedAddressTuple> = sqlx::query_as(sqlx::AssertSqlSafe(query))
        .fetch_all(pool)
        .await
        .context("failed to query watched addresses")?;

    Ok(rows.into_iter().map(WatchedAddressRow::from).collect())
}

async fn ensure_schema(pool: &sqlx::SqlitePool, table: &SqliteIngressTable) -> Result<()> {
    let table_name = &table.name;

    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE TABLE IF NOT EXISTS {table_name} (
            {col_address} TEXT NOT NULL,
            {col_caip2} TEXT NOT NULL,
            {col_symbol} TEXT NOT NULL,
            {col_asset_config} TEXT,
            {col_chain_config} TEXT,
            {col_egress} TEXT,
            PRIMARY KEY ({col_address}, {col_caip2}, {col_symbol})
        )",
        col_address = table.columns.address,
        col_caip2 = table.columns.caip2,
        col_symbol = table.columns.symbol,
        col_asset_config = table.columns.asset_config,
        col_chain_config = table.columns.chain_config,
        col_egress = table.columns.egress,
        table_name = table_name,
    )))
    .execute(pool)
    .await
    .with_context(|| format!("failed to ensure {table_name} schema"))?;

    Ok(())
}
