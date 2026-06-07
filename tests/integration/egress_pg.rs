/// Integration tests for PostgreSQL egress.
///
/// Covers: schema creation, insert, dedup via INSERT ON CONFLICT,
/// custom table/column names, connection failure handling, concurrent writes,
/// and ON CONFLICT on dedup index.
///
/// When PANO_TEST_PG_URL is set, tests run against a real Postgres database.
/// Otherwise, tests validate SQL construction helpers and connection-failure
/// handling — all tests still run and pass without external Postgres.
use super::common;

use pano::egress::pg::{
    PgEgressColumns, PgEgressTable, build_create_index_sql, build_create_table_sql,
    build_insert_sql, ensure_schema, insert_event,
};
use pano::model::{DepositData, DepositEvent};
use sqlx::Row;

// ── helpers ──────────────────────────────────────────────────────────────

/// Return the Postgres URL if PANO_TEST_PG_URL is set and non-empty.
fn pg_url() -> Option<String> {
    std::env::var("PANO_TEST_PG_URL")
        .ok()
        .filter(|s| !s.is_empty())
}

/// Connect to Postgres if PANO_TEST_PG_URL is set and reachable.
async fn connect_if_available() -> Option<sqlx::PgPool> {
    let url = pg_url()?;
    sqlx::PgPool::connect(&url).await.ok()
}

/// Build a DepositEvent for testing.
fn mk_event(tx_id: &str, amount: &str, log_index: u64, event_type: &str) -> DepositEvent {
    let data = DepositData {
        tx_id: tx_id.to_string(),
        caip2: "eip155:1".to_string(),
        symbol: "ETH".to_string(),
        address: common::EVM_ADDR.to_string(),
        block_number: 100,
        log_index,
        amount: amount.to_string(),
        sender: common::EVM_SENDER.to_string(),
        confirmations: 1,
        timestamp: "2026-06-04T00:00:00Z".to_string(),
        internal_egress: None,
    };
    match event_type {
        "detected" => DepositEvent::detected(data).expect("valid detected"),
        "confirmed" => {
            let detected = DepositEvent::detected(data).expect("valid detected");
            DepositEvent::confirmed_from(&detected, 12).expect("valid confirmed")
        }
        _ => panic!("unknown event type"),
    }
}

// ── Schema creation on empty database ────────────────────────────────────

#[test]
fn schema_creation_sql_default_table() {
    let table = PgEgressTable::default();
    let sql = build_create_table_sql(&table);

    assert!(
        sql.contains("CREATE TABLE IF NOT EXISTS deposit_events"),
        "should create default table name"
    );
    assert!(
        sql.contains("event_id TEXT PRIMARY KEY"),
        "should have event_id PK"
    );
    assert!(sql.contains("event TEXT NOT NULL"));
    assert!(sql.contains("version INTEGER NOT NULL"));
    assert!(sql.contains("occurred_at TEXT NOT NULL"));
    assert!(sql.contains("tx_id TEXT NOT NULL"));
    assert!(sql.contains("block_number BIGINT NOT NULL"));
    assert!(
        sql.contains("log_index BIGINT NOT NULL DEFAULT 0"),
        "should default log_index to 0"
    );
}

#[test]
fn schema_creation_sql_default_index() {
    let table = PgEgressTable::default();
    let sql = build_create_index_sql(&table);

    assert!(
        sql.contains("CREATE UNIQUE INDEX IF NOT EXISTS idx_deposit_events_dedup"),
        "should create dedup index with default table name"
    );
    assert!(sql.contains("ON deposit_events"));

    // Verify dedup index includes all 8 key columns
    let c = &table.columns;
    for col in [
        &c.tx_id,
        &c.caip2,
        &c.symbol,
        &c.address,
        &c.amount,
        &c.log_index,
        &c.block_number,
        &c.event,
    ] {
        assert!(
            sql.contains(col.as_str()),
            "dedup index should include column: {col}"
        );
    }
}

#[tokio::test]
async fn schema_creation_with_pg() {
    let Some(pool) = connect_if_available().await else {
        return; // PG unavailable — SQL builders already covered above
    };

    let table = PgEgressTable::default();

    // Clean up from any previous run
    let _ = sqlx::query("DROP TABLE IF EXISTS deposit_events CASCADE")
        .execute(&pool)
        .await;

    ensure_schema(&pool, &table).await.expect("ensure schema");

    // Verify table exists via pg_catalog
    let tbl_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pg_catalog.pg_tables WHERE schemaname = 'public' AND tablename = 'deposit_events'",
    )
    .fetch_one(&pool)
    .await
    .expect("query pg_tables");
    assert_eq!(
        tbl_count.0, 1,
        "deposit_events table should exist in public schema"
    );

    // Verify dedup index exists
    let idx_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pg_catalog.pg_indexes WHERE schemaname = 'public' AND indexname = 'idx_deposit_events_dedup'",
    )
    .fetch_one(&pool)
    .await
    .expect("query pg_indexes");
    assert_eq!(idx_count.0, 1, "dedup index should exist");

    // Verify all expected columns exist
    let columns: Vec<(String,)> = sqlx::query_as(
        "SELECT column_name FROM information_schema.columns WHERE table_schema = 'public' AND table_name = 'deposit_events' ORDER BY ordinal_position",
    )
    .fetch_all(&pool)
    .await
    .expect("query columns");
    let col_names: Vec<&str> = columns.iter().map(|(c,)| c.as_str()).collect();
    for expected in &[
        "event_id",
        "event",
        "version",
        "occurred_at",
        "tx_id",
        "caip2",
        "symbol",
        "address",
        "block_number",
        "log_index",
        "amount",
        "sender",
        "confirmations",
        "timestamp",
    ] {
        assert!(
            col_names.contains(expected),
            "column '{expected}' should exist; found: {col_names:?}"
        );
    }

    // Clean up
    let _ = sqlx::query("DROP TABLE IF EXISTS deposit_events CASCADE")
        .execute(&pool)
        .await;
}

// ── Insert event with correct columns ────────────────────────────────────

#[test]
fn insert_sql_correct_columns() {
    let table = PgEgressTable::default();
    let sql = build_insert_sql(&table);

    assert!(
        sql.starts_with("INSERT INTO deposit_events"),
        "should insert into default table"
    );

    let c = &table.columns;
    let all_cols = [
        &c.event_id,
        &c.event,
        &c.version,
        &c.occurred_at,
        &c.tx_id,
        &c.caip2,
        &c.symbol,
        &c.address,
        &c.block_number,
        &c.log_index,
        &c.amount,
        &c.sender,
        &c.confirmations,
        &c.timestamp,
    ];
    for col in &all_cols {
        assert!(
            sql.contains(col.as_str()),
            "INSERT should include column: {col}"
        );
    }

    // Verify parameter placeholders $1..$14
    for i in 1..=14 {
        assert!(
            sql.contains(&format!("${i}")),
            "INSERT should contain parameter ${i}"
        );
    }

    assert!(
        sql.contains("ON CONFLICT DO NOTHING"),
        "should use ON CONFLICT DO NOTHING (not a specific conflict target)"
    );

    // Guard: verify NO specific conflict target is named
    assert!(
        !sql.contains("ON CONFLICT ("),
        "should not specify a conflict target — must handle ALL unique violations"
    );
}

#[tokio::test]
async fn insert_event_correct_columns_with_pg() {
    let Some(pool) = connect_if_available().await else {
        return; // PG unavailable — SQL builder already covered above
    };

    let table = PgEgressTable::default();

    let _ = sqlx::query("DROP TABLE IF EXISTS deposit_events CASCADE")
        .execute(&pool)
        .await;
    ensure_schema(&pool, &table).await.expect("schema");

    let event = mk_event("0xtx_insert_pg", "1500000000000000000", 0, "detected");
    insert_event(&pool, &event, &table).await.expect("insert");

    let c = &table.columns;
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {c0},{c1},{c2},{c3},{c4},{c5},{c6},{c7},{c8},{c9},{c10},{c11},{c12},{c13} \
         FROM deposit_events",
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
    )))
    .fetch_one(&pool)
    .await
    .expect("fetch row");

    assert_eq!(row.get::<String, _>(c.event_id.as_str()), event.event_id);
    assert_eq!(
        row.get::<String, _>(c.event.as_str()),
        "pano.deposit.detected"
    );
    assert_eq!(row.get::<i32, _>(c.version.as_str()), 1);
    assert_eq!(row.get::<String, _>(c.tx_id.as_str()), "0xtx_insert_pg");
    assert_eq!(row.get::<String, _>(c.caip2.as_str()), "eip155:1");
    assert_eq!(row.get::<String, _>(c.symbol.as_str()), "ETH");
    assert_eq!(row.get::<String, _>(c.address.as_str()), common::EVM_ADDR);
    assert_eq!(row.get::<i64, _>(c.block_number.as_str()), 100);
    assert_eq!(row.get::<i64, _>(c.log_index.as_str()), 0);
    assert_eq!(
        row.get::<String, _>(c.amount.as_str()),
        "1500000000000000000"
    );
    assert_eq!(row.get::<String, _>(c.sender.as_str()), common::EVM_SENDER);
    assert_eq!(row.get::<i32, _>(c.confirmations.as_str()), 1);
    assert_eq!(
        row.get::<String, _>(c.timestamp.as_str()),
        "2026-06-04T00:00:00Z"
    );

    let _ = sqlx::query("DROP TABLE IF EXISTS deposit_events CASCADE")
        .execute(&pool)
        .await;
}

// ── Dedup via INSERT ON CONFLICT ─────────────────────────────────────────

#[tokio::test]
async fn insert_event_dedup_ignores_duplicate_row() {
    let Some(pool) = connect_if_available().await else {
        // Without PG: verify SQL builder handles dedup correctly
        let table = PgEgressTable::default();
        let sql = build_insert_sql(&table);
        assert!(
            sql.contains("ON CONFLICT DO NOTHING"),
            "INSERT should use ON CONFLICT DO NOTHING for dedup"
        );
        return;
    };

    let table = PgEgressTable::default();

    let _ = sqlx::query("DROP TABLE IF EXISTS deposit_events CASCADE")
        .execute(&pool)
        .await;
    ensure_schema(&pool, &table).await.expect("schema");

    let event = mk_event("0xtx_dedup_pg", "999000", 0, "detected");

    // Insert the same event twice
    insert_event(&pool, &event, &table)
        .await
        .expect("first insert");
    insert_event(&pool, &event, &table)
        .await
        .expect("second insert (should be ignored by ON CONFLICT)");

    // Only one row should exist
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM deposit_events WHERE tx_id = '0xtx_dedup_pg'")
            .fetch_one(&pool)
            .await
            .expect("count");
    assert_eq!(
        count.0, 1,
        "duplicate should be ignored by ON CONFLICT DO NOTHING"
    );

    let _ = sqlx::query("DROP TABLE IF EXISTS deposit_events CASCADE")
        .execute(&pool)
        .await;
}

// ── Custom table and column names ────────────────────────────────────────

#[test]
fn custom_table_and_column_names_in_sql_builders() {
    let custom_table = PgEgressTable {
        name: "my_deposits".to_string(),
        columns: PgEgressColumns {
            event_id: "ev_id".to_string(),
            event: "ev_type".to_string(),
            version: "ver".to_string(),
            occurred_at: "occ_at".to_string(),
            tx_id: "tx".to_string(),
            caip2: "chain".to_string(),
            symbol: "sym".to_string(),
            address: "addr".to_string(),
            block_number: "blk".to_string(),
            log_index: "log_idx".to_string(),
            amount: "amt".to_string(),
            sender: "sndr".to_string(),
            confirmations: "confs".to_string(),
            timestamp: "ts".to_string(),
        },
    };

    let create_sql = build_create_table_sql(&custom_table);
    assert!(
        create_sql.contains("CREATE TABLE IF NOT EXISTS my_deposits"),
        "should use custom table name"
    );
    assert!(
        create_sql.contains("ev_id TEXT PRIMARY KEY"),
        "should use custom event_id column"
    );
    assert!(
        create_sql.contains("ev_type TEXT NOT NULL"),
        "should use custom event column"
    );
    assert!(
        create_sql.contains("blk BIGINT NOT NULL"),
        "should use custom block_number column"
    );

    let index_sql = build_create_index_sql(&custom_table);
    assert!(
        index_sql.contains("CREATE UNIQUE INDEX IF NOT EXISTS idx_my_deposits_dedup"),
        "dedup index name should use custom table name"
    );
    assert!(
        index_sql.contains("ON my_deposits"),
        "dedup index should reference custom table"
    );
    // Verify custom column names in the index
    for col in &[
        "tx", "chain", "sym", "addr", "amt", "log_idx", "blk", "ev_type",
    ] {
        assert!(
            index_sql.contains(col),
            "dedup index with custom columns should include: {col}"
        );
    }

    let insert_sql = build_insert_sql(&custom_table);
    assert!(
        insert_sql.starts_with("INSERT INTO my_deposits"),
        "INSERT should use custom table name"
    );
    for col in &[
        "ev_id", "ev_type", "ver", "occ_at", "tx", "chain", "sym", "addr", "blk", "log_idx", "amt",
        "sndr", "confs", "ts",
    ] {
        assert!(
            insert_sql.contains(col),
            "INSERT with custom columns should include: {col}"
        );
    }
}

#[tokio::test]
async fn custom_table_and_column_names_with_pg() {
    let Some(pool) = connect_if_available().await else {
        return; // SQL builder already covered above
    };

    let custom_table = PgEgressTable {
        name: "my_deposits".to_string(),
        columns: PgEgressColumns {
            event_id: "ev_id".to_string(),
            event: "ev_type".to_string(),
            version: "ver".to_string(),
            occurred_at: "occ_at".to_string(),
            tx_id: "tx".to_string(),
            caip2: "chain".to_string(),
            symbol: "sym".to_string(),
            address: "addr".to_string(),
            block_number: "blk".to_string(),
            log_index: "log_idx".to_string(),
            amount: "amt".to_string(),
            sender: "sndr".to_string(),
            confirmations: "confs".to_string(),
            timestamp: "ts".to_string(),
        },
    };

    let _ = sqlx::query("DROP TABLE IF EXISTS my_deposits CASCADE")
        .execute(&pool)
        .await;

    ensure_schema(&pool, &custom_table).await.expect("schema");

    // Verify custom table exists
    let tbl_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pg_catalog.pg_tables WHERE schemaname = 'public' AND tablename = 'my_deposits'",
    )
    .fetch_one(&pool)
    .await
    .expect("query");
    assert_eq!(tbl_count.0, 1, "custom table should exist");

    // Verify custom dedup index exists
    let idx_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pg_catalog.pg_indexes WHERE schemaname = 'public' AND indexname = 'idx_my_deposits_dedup'",
    )
    .fetch_one(&pool)
    .await
    .expect("query index");
    assert_eq!(idx_count.0, 1, "custom dedup index should exist");

    // Insert and read back using custom column names
    let event = mk_event("0xcustom_pg", "5000", 0, "detected");
    insert_event(&pool, &event, &custom_table)
        .await
        .expect("insert into custom table");

    let row = sqlx::query(sqlx::AssertSqlSafe(
        "SELECT tx, amt, sym, chain FROM my_deposits".to_string(),
    ))
    .fetch_one(&pool)
    .await
    .expect("fetch from custom table");

    assert_eq!(row.get::<String, _>("tx"), "0xcustom_pg");
    assert_eq!(row.get::<String, _>("amt"), "5000");
    assert_eq!(row.get::<String, _>("sym"), "ETH");
    assert_eq!(row.get::<String, _>("chain"), "eip155:1");

    let _ = sqlx::query("DROP TABLE IF EXISTS my_deposits CASCADE")
        .execute(&pool)
        .await;
}

// ── Connection failure handling ──────────────────────────────────────────

#[tokio::test]
async fn connection_failure_on_bad_url() {
    // Attempt to connect to a non-existent host — should fail, not panic
    let result = sqlx::PgPool::connect("postgres://nonexistent:5432/nope").await;
    assert!(
        result.is_err(),
        "connecting to nonexistent host should return Err"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("failed to connect")
            || err.contains("error")
            || err.contains("resolve")
            || err.contains("timeout"),
        "error message should indicate connection failure, got: {err}"
    );
}

#[tokio::test]
async fn connection_failure_on_invalid_syntax() {
    // Syntactically invalid URL
    let result = sqlx::PgPool::connect("not_a_valid_url_%%%").await;
    assert!(
        result.is_err(),
        "connecting with invalid URL syntax should return Err"
    );
}

#[tokio::test]
async fn ensure_schema_idempotent_sql() {
    // The SQL builders always produce IF NOT EXISTS, guaranteeing idempotency
    let table = PgEgressTable::default();

    let create1 = build_create_table_sql(&table);
    let create2 = build_create_table_sql(&table);
    assert_eq!(create1, create2, "CREATE TABLE SQL should be deterministic");

    assert!(
        create1.contains("IF NOT EXISTS"),
        "CREATE TABLE should use IF NOT EXISTS"
    );

    let idx1 = build_create_index_sql(&table);
    let idx2 = build_create_index_sql(&table);
    assert_eq!(idx1, idx2, "CREATE INDEX SQL should be deterministic");

    assert!(
        idx1.contains("IF NOT EXISTS"),
        "CREATE INDEX should use IF NOT EXISTS"
    );
}

#[tokio::test]
async fn ensure_schema_idempotent_with_pg() {
    let Some(pool) = connect_if_available().await else {
        return; // SQL idempotency already verified above
    };

    let table = PgEgressTable::default();

    let _ = sqlx::query("DROP TABLE IF EXISTS deposit_events CASCADE")
        .execute(&pool)
        .await;

    // Call twice — no errors
    ensure_schema(&pool, &table)
        .await
        .expect("first ensure schema");
    ensure_schema(&pool, &table)
        .await
        .expect("second ensure schema (idempotent)");

    // Table should still exist exactly once
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM pg_catalog.pg_tables WHERE schemaname = 'public' AND tablename = 'deposit_events'",
    )
    .fetch_one(&pool)
    .await
    .expect("query");
    assert_eq!(count.0, 1, "table should exist once after idempotent calls");

    let _ = sqlx::query("DROP TABLE IF EXISTS deposit_events CASCADE")
        .execute(&pool)
        .await;
}

// ── Concurrent write serialization ───────────────────────────────────────

#[tokio::test]
async fn concurrent_inserts_all_succeed_with_pg() {
    let Some(pool) = connect_if_available().await else {
        // Without PG: verify that the INSERT SQL uses ON CONFLICT DO NOTHING
        // which handles concurrent duplicate inserts safely
        let table = PgEgressTable::default();
        let sql = build_insert_sql(&table);
        assert!(
            sql.contains("ON CONFLICT DO NOTHING"),
            "concurrent safety relies on ON CONFLICT DO NOTHING"
        );
        return;
    };

    let table = PgEgressTable::default();

    let _ = sqlx::query("DROP TABLE IF EXISTS deposit_events CASCADE")
        .execute(&pool)
        .await;
    ensure_schema(&pool, &table).await.expect("schema");

    // Spawn multiple concurrent inserts with unique tx_ids
    let mut handles = Vec::new();
    for i in 0..16 {
        let pool = pool.clone();
        let table = table.clone();
        handles.push(tokio::spawn(async move {
            let event = mk_event(
                &format!("0xconcurrent_{i}"),
                &format!("{}", 1000 + i),
                0,
                "detected",
            );
            insert_event(&pool, &event, &table).await
        }));
    }

    for handle in handles {
        let result = handle.await.expect("task join");
        assert!(
            result.is_ok(),
            "concurrent insert should succeed: {result:?}"
        );
    }

    // Verify all 16 rows were inserted
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM deposit_events")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count.0, 16, "all concurrent inserts should be persisted");

    let _ = sqlx::query("DROP TABLE IF EXISTS deposit_events CASCADE")
        .execute(&pool)
        .await;
}

#[tokio::test]
async fn concurrent_duplicate_inserts_are_safe_with_pg() {
    let Some(pool) = connect_if_available().await else {
        // Without PG: verify ON CONFLICT DO NOTHING handles duplicates
        let table = PgEgressTable::default();
        let sql = build_insert_sql(&table);
        assert!(
            sql.contains("ON CONFLICT DO NOTHING"),
            "dedup safety relies on ON CONFLICT DO NOTHING"
        );
        return;
    };

    let table = PgEgressTable::default();

    let _ = sqlx::query("DROP TABLE IF EXISTS deposit_events CASCADE")
        .execute(&pool)
        .await;
    ensure_schema(&pool, &table).await.expect("schema");

    // Spawn 8 concurrent inserts of the SAME event
    let event = mk_event("0xconcurrent_dup", "999000", 0, "detected");
    let mut handles = Vec::new();
    for _ in 0..8 {
        let pool = pool.clone();
        let table = table.clone();
        let event = event.clone();
        handles.push(tokio::spawn(async move {
            insert_event(&pool, &event, &table).await
        }));
    }

    for handle in handles {
        let result = handle.await.expect("task join");
        assert!(
            result.is_ok(),
            "concurrent duplicate insert should not error"
        );
    }

    // Only one row should exist
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM deposit_events WHERE tx_id = '0xconcurrent_dup'")
            .fetch_one(&pool)
            .await
            .expect("count");
    assert_eq!(
        count.0, 1,
        "concurrent duplicates should result in exactly one row"
    );

    let _ = sqlx::query("DROP TABLE IF EXISTS deposit_events CASCADE")
        .execute(&pool)
        .await;
}

// ── ON CONFLICT on dedup index (not just PK) ────────────────────────────

#[test]
fn on_conflict_handles_all_unique_violations_not_just_pk() {
    // Fix: ON CONFLICT DO NOTHING (no specific target) handles
    // violations of ALL unique constraints — both the PK and the dedup index.
    let table = PgEgressTable::default();
    let sql = build_insert_sql(&table);

    // The fix: no specific conflict target
    assert!(
        sql.contains("ON CONFLICT DO NOTHING"),
        "should use ON CONFLICT DO NOTHING (all unique constraints)"
    );

    // Should NOT name a specific conflict target like ON CONFLICT (event_id)
    assert!(
        !sql.contains("ON CONFLICT ("),
        "should NOT target a specific constraint (must handle all UNIQUE violations)"
    );
}

#[tokio::test]
async fn on_conflict_skips_dedup_index_violation_with_pg() {
    // Scenario: two events with different event_ids (different PK)
    // but same dedup-index values. ON CONFLICT DO NOTHING should silently
    // skip the second one instead of raising a constraint violation error.
    let Some(pool) = connect_if_available().await else {
        // Without PG: SQL builder verified above — ON CONFLICT DO NOTHING
        // handles all unique constraint violations.
        return;
    };

    let table = PgEgressTable::default();

    let _ = sqlx::query("DROP TABLE IF EXISTS deposit_events CASCADE")
        .execute(&pool)
        .await;
    ensure_schema(&pool, &table).await.expect("schema");

    // Two events with different event_id (different PK) but identical dedup-key values
    let ev1 = mk_event("0xtx_dedupidx_pg", "500000", 0, "detected");
    let ev2 = mk_event("0xtx_dedupidx_pg", "500000", 0, "detected");

    // These have different event_ids (ULIDs generated at creation time)
    assert_ne!(
        ev1.event_id, ev2.event_id,
        "different event IDs should have been generated"
    );

    // All dedup-index columns are identical: tx_id, caip2, symbol, address,
    // amount, log_index, block_number, event
    assert_eq!(ev1.data.tx_id, ev2.data.tx_id);
    assert_eq!(ev1.data.caip2, ev2.data.caip2);
    assert_eq!(ev1.data.symbol, ev2.data.symbol);
    assert_eq!(ev1.data.address, ev2.data.address);
    assert_eq!(ev1.data.amount, ev2.data.amount);
    assert_eq!(ev1.data.log_index, ev2.data.log_index);
    assert_eq!(ev1.data.block_number, ev2.data.block_number);
    assert_eq!(ev1.event, ev2.event);

    // Insert first event
    insert_event(&pool, &ev1, &table)
        .await
        .expect("insert first event");

    // Insert second event — has different PK but violates dedup UNIQUE index
    // This should NOT return an error; ON CONFLICT DO NOTHING should silently skip
    let result = insert_event(&pool, &ev2, &table).await;
    assert!(
        result.is_ok(),
        "insert of dedup-index-violating event should succeed (ON CONFLICT DO NOTHING), got: {result:?}"
    );

    // Verify only one row exists
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM deposit_events WHERE tx_id = '0xtx_dedupidx_pg'")
            .fetch_one(&pool)
            .await
            .expect("count");
    assert_eq!(
        count.0, 1,
        "dedup-index violation should be silently skipped (ON CONFLICT DO NOTHING)"
    );

    // Verify the stored row has ev1's event_id (first writer wins)
    let stored_id: (String,) =
        sqlx::query_as("SELECT event_id FROM deposit_events WHERE tx_id = '0xtx_dedupidx_pg'")
            .fetch_one(&pool)
            .await
            .expect("fetch stored event_id");
    assert_eq!(
        stored_id.0, ev1.event_id,
        "first event_id should be the one stored"
    );

    let _ = sqlx::query("DROP TABLE IF EXISTS deposit_events CASCADE")
        .execute(&pool)
        .await;
}

// ── Dedup index composition (different event type = different row) ──────

#[tokio::test]
async fn dedup_index_allows_different_event_types_with_pg() {
    let Some(pool) = connect_if_available().await else {
        // Without PG: verify dedup index includes the 'event' column,
        // so different event types (detected vs confirmed) produce different rows
        let table = PgEgressTable::default();
        let idx_sql = build_create_index_sql(&table);
        assert!(
            idx_sql.contains(&table.columns.event),
            "dedup index should include event type column"
        );
        return;
    };

    let table = PgEgressTable::default();

    let _ = sqlx::query("DROP TABLE IF EXISTS deposit_events CASCADE")
        .execute(&pool)
        .await;
    ensure_schema(&pool, &table).await.expect("schema");

    // Same tx_id and dedup-key values, but different event types
    let ev_detected = mk_event("0xsame_pg", "3000", 0, "detected");
    let ev_confirmed = mk_event("0xsame_pg", "3000", 0, "confirmed");

    insert_event(&pool, &ev_detected, &table)
        .await
        .expect("insert detected");
    insert_event(&pool, &ev_confirmed, &table)
        .await
        .expect("insert confirmed");

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM deposit_events")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(
        count.0, 2,
        "detected and confirmed are different rows (dedup index includes event type)"
    );

    // Verify both event types are present
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT event FROM deposit_events WHERE tx_id = '0xsame_pg' ORDER BY event")
            .fetch_all(&pool)
            .await
            .expect("fetch");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "pano.deposit.confirmed"); // 'c' < 'd'
    assert_eq!(rows[1].0, "pano.deposit.detected");

    let _ = sqlx::query("DROP TABLE IF EXISTS deposit_events CASCADE")
        .execute(&pool)
        .await;
}

// ── Dedup index — different log_index → two rows ────────────────────────

#[tokio::test]
async fn dedup_index_blocks_exact_duplicate_but_not_different_log_index_with_pg() {
    let Some(pool) = connect_if_available().await else {
        // Without PG: verify log_index is part of the dedup index
        let table = PgEgressTable::default();
        let idx_sql = build_create_index_sql(&table);
        assert!(
            idx_sql.contains(&table.columns.log_index),
            "dedup index should include log_index column"
        );
        return;
    };

    let table = PgEgressTable::default();

    let _ = sqlx::query("DROP TABLE IF EXISTS deposit_events CASCADE")
        .execute(&pool)
        .await;
    ensure_schema(&pool, &table).await.expect("schema");

    // Two events differing only in log_index
    let ev0 = mk_event("0xtx_dedup2_pg", "4000", 0, "detected");
    let ev1 = mk_event("0xtx_dedup2_pg", "4000", 1, "detected");

    insert_event(&pool, &ev0, &table)
        .await
        .expect("insert first");
    insert_event(&pool, &ev1, &table)
        .await
        .expect("insert second (different log_index)");

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM deposit_events")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count.0, 2, "different log_index → two distinct rows");

    let _ = sqlx::query("DROP TABLE IF EXISTS deposit_events CASCADE")
        .execute(&pool)
        .await;
}
