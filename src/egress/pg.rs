use crate::model::DepositEvent;
use crate::shared::db;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

// ── Egress PostgreSQL configuration ─────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PgEgressConfig {
    pub enabled: bool,
    pub url: String,
    pub table: PgEgressTable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PgEgressTable {
    pub name: String,
    pub columns: PgEgressColumns,
}

impl Default for PgEgressTable {
    fn default() -> Self {
        Self {
            name: "deposit_events".to_string(),
            columns: PgEgressColumns::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PgEgressColumns {
    pub event_id: String,
    pub event: String,
    pub version: String,
    pub occurred_at: String,
    pub tx_id: String,
    pub caip2: String,
    pub symbol: String,
    pub address: String,
    pub block_number: String,
    pub log_index: String,
    pub amount: String,
    pub sender: String,
    pub confirmations: String,
    pub timestamp: String,
}

impl Default for PgEgressColumns {
    fn default() -> Self {
        Self {
            event_id: "event_id".to_string(),
            event: "event".to_string(),
            version: "version".to_string(),
            occurred_at: "occurred_at".to_string(),
            tx_id: "tx_id".to_string(),
            caip2: "caip2".to_string(),
            symbol: "symbol".to_string(),
            address: "address".to_string(),
            block_number: "block_number".to_string(),
            log_index: "log_index".to_string(),
            amount: "amount".to_string(),
            sender: "sender".to_string(),
            confirmations: "confirmations".to_string(),
            timestamp: "timestamp".to_string(),
        }
    }
}

// ── Implementation ──────────────────────────────────────────────────────

/// Write deposit events to a PostgreSQL database.
pub async fn write_events(
    config: PgEgressConfig,
    rx: &mut broadcast::Receiver<DepositEvent>,
) -> Result<()> {
    let pool = db::connect_pg(&config.url).await?;
    ensure_schema(&pool, &config.table).await?;
    while let Some(ev) = super::recv_event(rx).await {
        if let Err(e) = insert_event(&pool, &ev, &config.table).await {
            tracing::error!(error = %e, event_id = %ev.event_id, "failed to insert event into pg");
        }
    }
    Ok(())
}

/// Build the CREATE TABLE SQL statement (exposed for testing/auditing).
pub fn build_create_table_sql(table: &PgEgressTable) -> String {
    let t = &table.name;
    let c = &table.columns;
    format!(
        "CREATE TABLE IF NOT EXISTS {t} (
            {c0} TEXT PRIMARY KEY,
            {c1} TEXT NOT NULL,
            {c2} INTEGER NOT NULL,
            {c3} TEXT NOT NULL,
            {c4} TEXT NOT NULL,
            {c5} TEXT NOT NULL,
            {c6} TEXT NOT NULL,
            {c7} TEXT NOT NULL,
            {c8} BIGINT NOT NULL,
            {c9} BIGINT NOT NULL DEFAULT 0,
            {c10} TEXT NOT NULL,
            {c11} TEXT NOT NULL,
            {c12} INTEGER NOT NULL,
            {c13} TEXT NOT NULL
        )",
        c0 = c.event_id,
        c1 = c.event,
        c2 = c.version,
        c3 = c.occurred_at,
        c4 = c.tx_id,
        c5 = c.caip2,
        c6 = c.symbol,
        c7 = c.address,
        c8 = c.block_number,
        c9 = c.log_index,
        c10 = c.amount,
        c11 = c.sender,
        c12 = c.confirmations,
        c13 = c.timestamp,
        t = t,
    )
}

/// Build the CREATE UNIQUE INDEX SQL statement (exposed for testing/auditing).
pub fn build_create_index_sql(table: &PgEgressTable) -> String {
    let t = &table.name;
    let c = &table.columns;
    format!(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_{t}_dedup
         ON {t}({c4}, {c5}, {c6}, {c7}, {c10}, {c9}, {c8}, {c1})",
        t = t,
        c4 = c.tx_id,
        c5 = c.caip2,
        c6 = c.symbol,
        c7 = c.address,
        c10 = c.amount,
        c9 = c.log_index,
        c8 = c.block_number,
        c1 = c.event,
    )
}

/// Build the INSERT SQL statement (exposed for testing/auditing).
pub fn build_insert_sql(table: &PgEgressTable) -> String {
    let t = &table.name;
    let c = &table.columns;
    format!(
        "INSERT INTO {t} ({c0}, {c1}, {c2}, {c3}, {c4}, {c5}, {c6}, {c7}, {c8}, {c9}, {c10}, {c11}, {c12}, {c13}) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) \
         ON CONFLICT DO NOTHING",
        c0 = c.event_id,
        c1 = c.event,
        c2 = c.version,
        c3 = c.occurred_at,
        c4 = c.tx_id,
        c5 = c.caip2,
        c6 = c.symbol,
        c7 = c.address,
        c8 = c.block_number,
        c9 = c.log_index,
        c10 = c.amount,
        c11 = c.sender,
        c12 = c.confirmations,
        c13 = c.timestamp,
        t = t,
    )
}

pub async fn ensure_schema(pool: &sqlx::PgPool, table: &PgEgressTable) -> Result<()> {
    sqlx::query(sqlx::AssertSqlSafe(build_create_table_sql(table)))
        .execute(pool)
        .await
        .context("failed to create pg deposit events table")?;

    sqlx::query(sqlx::AssertSqlSafe(build_create_index_sql(table)))
        .execute(pool)
        .await
        .context("failed to create pg deposit event deduplication index")?;

    Ok(())
}

pub async fn insert_event(
    pool: &sqlx::PgPool,
    ev: &DepositEvent,
    table: &PgEgressTable,
) -> Result<()> {
    sqlx::query(sqlx::AssertSqlSafe(build_insert_sql(table)))
        .bind(&ev.event_id)
        .bind(&ev.event)
        .bind(ev.version as i32)
        .bind(&ev.occurred_at)
        .bind(&ev.data.tx_id)
        .bind(&ev.data.caip2)
        .bind(&ev.data.symbol)
        .bind(&ev.data.address)
        .bind(ev.data.block_number as i64)
        .bind(ev.data.log_index as i64)
        .bind(&ev.data.amount)
        .bind(&ev.data.sender)
        .bind(ev.data.confirmations as i32)
        .bind(&ev.data.timestamp)
        .execute(pool)
        .await?;

    Ok(())
}
