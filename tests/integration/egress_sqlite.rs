/// Integration tests for SQLite egress.
///
/// Covers: schema creation, idempotent ensure_schema, insert, dedup via
/// INSERT OR IGNORE, custom table/column names, and dedup index composition.
use super::common;

use pano::egress::sqlite::{SqliteEgressColumns, SqliteEgressTable, ensure_schema, insert_event};
use pano::model::{DepositData, DepositEvent};
use sqlx::Row;
use tempfile::TempDir;

// ── helpers ──────────────────────────────────────────────────────────────

async fn open_pool(path: &str) -> sqlx::SqlitePool {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("open sqlite pool")
}

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

// ── ensure_schema — creates table on empty DB ────────────────────────────

#[tokio::test]
async fn ensure_schema_creates_table_on_empty_db() {
    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("test.db");
    let path_s = db_path.to_str().expect("utf8");

    let pool = open_pool(path_s).await;
    let table = SqliteEgressTable::default();

    ensure_schema(&pool, &table).await.expect("ensure schema");

    // Verify table exists by querying sqlite_master
    let count: (i32,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='deposit_events'",
    )
    .fetch_one(&pool)
    .await
    .expect("query sqlite_master");
    assert_eq!(count.0, 1, "deposit_events table should exist");

    // Verify dedup index exists
    let idx_count: (i32,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_deposit_events_dedup'",
    )
    .fetch_one(&pool)
    .await
    .expect("query index");
    assert_eq!(idx_count.0, 1, "dedup index should exist");
}

// ── ensure_schema — idempotent on existing schema ────────────────────────

#[tokio::test]
async fn ensure_schema_is_idempotent() {
    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("test.db");
    let path_s = db_path.to_str().expect("utf8");

    let pool = open_pool(path_s).await;
    let table = SqliteEgressTable::default();

    // Call twice — no errors
    ensure_schema(&pool, &table).await.expect("first ensure");
    ensure_schema(&pool, &table).await.expect("second ensure");

    // Table should still exist exactly once
    let count: (i32,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='deposit_events'",
    )
    .fetch_one(&pool)
    .await
    .expect("query");
    assert_eq!(count.0, 1);
}

// ── insert_event — inserts row with correct columns ──────────────────────

#[tokio::test]
async fn insert_event_inserts_row_with_correct_columns() {
    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("test.db");
    let path_s = db_path.to_str().expect("utf8");

    let pool = open_pool(path_s).await;
    let table = SqliteEgressTable::default();
    ensure_schema(&pool, &table).await.expect("schema");

    let event = mk_event("0xtx_insert", "1500000000000000000", 0, "detected");
    insert_event(&pool, &event, &table).await.expect("insert");

    let c = &table.columns;
    let row = sqlx::query(sqlx::AssertSqlSafe(format!(
        "SELECT {c0},{c1},{c2},{c3},{c4},{c5},{c6},{c7},{c8},{c9},{c10},{c11},{c12},{c13} FROM deposit_events",
        c0 = c.event_id, c1 = c.event, c2 = c.version, c3 = c.occurred_at,
        c4 = c.tx_id, c5 = c.caip2, c6 = c.symbol, c7 = c.address,
        c8 = c.block_number, c9 = c.log_index, c10 = c.amount,
        c11 = c.sender, c12 = c.confirmations, c13 = c.timestamp,
    )))
    .fetch_one(&pool)
    .await
    .expect("fetch row");

    // sqlx Row::get requires &str, so dereference the String fields
    assert_eq!(row.get::<String, _>(c.event_id.as_str()), event.event_id);
    assert_eq!(
        row.get::<String, _>(c.event.as_str()),
        "pano.deposit.detected"
    );
    assert_eq!(row.get::<i32, _>(c.version.as_str()), 1);
    assert_eq!(row.get::<String, _>(c.tx_id.as_str()), "0xtx_insert");
    assert_eq!(row.get::<String, _>(c.caip2.as_str()), "eip155:1");
    assert_eq!(row.get::<String, _>(c.symbol.as_str()), "ETH");
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
}

// ── insert_event — dedup via INSERT OR IGNORE ────────────────────────────

#[tokio::test]
async fn insert_event_dedup_ignores_duplicate() {
    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("test.db");
    let path_s = db_path.to_str().expect("utf8");

    let pool = open_pool(path_s).await;
    let table = SqliteEgressTable::default();
    ensure_schema(&pool, &table).await.expect("schema");

    let event = mk_event("0xtx_dedup", "999000", 0, "detected");

    // Insert the same event twice
    insert_event(&pool, &event, &table)
        .await
        .expect("first insert");
    insert_event(&pool, &event, &table)
        .await
        .expect("second insert (should ignore)");

    // Only one row should exist
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM deposit_events")
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(
        count.0, 1,
        "duplicate should be ignored by INSERT OR IGNORE"
    );
}

// ── Custom table and column names ────────────────────────────────────────

#[tokio::test]
async fn custom_table_and_column_names() {
    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("test.db");
    let path_s = db_path.to_str().expect("utf8");

    let pool = open_pool(path_s).await;

    let custom_table = SqliteEgressTable {
        name: "my_deposits".to_string(),
        columns: SqliteEgressColumns {
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

    ensure_schema(&pool, &custom_table).await.expect("schema");

    // Verify custom table exists
    let tbl_count: (i32,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='my_deposits'",
    )
    .fetch_one(&pool)
    .await
    .expect("query");
    assert_eq!(tbl_count.0, 1, "custom table should exist");

    // Insert into custom table
    let event = mk_event("0xcustom", "5000", 0, "detected");
    insert_event(&pool, &event, &custom_table)
        .await
        .expect("insert into custom table");

    let row = sqlx::query(sqlx::AssertSqlSafe(
        "SELECT tx, amt, sym, chain FROM my_deposits".to_string(),
    ))
    .fetch_one(&pool)
    .await
    .expect("fetch from custom table");

    assert_eq!(row.get::<String, _>("tx"), "0xcustom");
    assert_eq!(row.get::<String, _>("amt"), "5000");
    assert_eq!(row.get::<String, _>("sym"), "ETH");
    assert_eq!(row.get::<String, _>("chain"), "eip155:1");

    // Verify dedup index on custom table
    let idx_count: (i32,) = sqlx::query_as(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_my_deposits_dedup'",
    )
    .fetch_one(&pool)
    .await
    .expect("query index");
    assert_eq!(idx_count.0, 1, "custom dedup index should exist");
}

// ── Dedup index composition (different event type = different row) ───────

#[tokio::test]
async fn dedup_index_allows_different_event_types() {
    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("test.db");
    let path_s = db_path.to_str().expect("utf8");

    let pool = open_pool(path_s).await;
    let table = SqliteEgressTable::default();
    ensure_schema(&pool, &table).await.expect("schema");

    // Same tx_id, caip2, symbol, address, amount, log_index, block_number,
    // but different `event` type (detected vs confirmed)
    let ev_detected = mk_event("0xsame", "3000", 0, "detected");
    let ev_confirmed = mk_event("0xsame", "3000", 0, "confirmed");

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
        sqlx::query_as("SELECT event FROM deposit_events WHERE tx_id = '0xsame' ORDER BY event")
            .fetch_all(&pool)
            .await
            .expect("fetch");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, "pano.deposit.confirmed"); // 'c' < 'd'
    assert_eq!(rows[1].0, "pano.deposit.detected");
}

// ── Dedup index — different log_index → two rows ────────────────────────

#[tokio::test]
async fn dedup_index_blocks_exact_duplicate_but_not_different_log_index() {
    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("test.db");
    let path_s = db_path.to_str().expect("utf8");

    let pool = open_pool(path_s).await;
    let table = SqliteEgressTable::default();
    ensure_schema(&pool, &table).await.expect("schema");

    // Two events differing only in log_index
    let ev0 = mk_event("0xtx_dedup2", "4000", 0, "detected");
    let ev1 = mk_event("0xtx_dedup2", "4000", 1, "detected");

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
}

// ── Edge: database file permission denied or invalid path ────────────────

#[tokio::test]
async fn invalid_db_path_returns_error() {
    // Use sqlx directly (not open_pool which panics) to get a Result
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename("/nonexistent_dir_xyz_12345/subdir/db.sqlite")
        .create_if_missing(true);
    let result = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await;

    // Opening DB on a path with nonexistent parent directory should fail
    assert!(result.is_err(), "opening DB on invalid path should fail");
}

#[tokio::test]
async fn read_only_directory_prevents_db_creation() {
    let dir = TempDir::new().expect("temp dir");

    // Create a read-only subdirectory
    let ro_dir = dir.path().join("readonly_db");
    std::fs::create_dir(&ro_dir).expect("create ro dir");

    let mut perms = std::fs::metadata(&ro_dir).expect("metadata").permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&ro_dir, perms).expect("set readonly");

    let db_path = ro_dir.join("test.db");
    let path_s = db_path.to_str().expect("utf8");

    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path_s)
        .create_if_missing(true);
    let result = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await;

    // On most systems, writing to a read-only directory fails.
    // If running as root, it may succeed — we accept either outcome.
    match result {
        Ok(_) => {
            eprintln!(
                "read_only_directory_prevents_db_creation: pool opened (likely running as root)"
            );
        }
        Err(error) => {
            let err = error.to_string();
            assert!(
                err.contains("permission denied")
                    || err.contains("read-only")
                    || err.contains("unable to open")
                    || err.to_lowercase().contains("perm")
                    || err.to_lowercase().contains("open"),
                "error should indicate filesystem issue: {err}"
            );
        }
    }
}

// ── Edge: concurrent access from multiple pools ──────────────────────────

#[tokio::test]
async fn concurrent_access_from_multiple_pools() {
    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("concurrent.db");
    let path_s = db_path.to_str().expect("utf8");

    // Open two independent pools to the same database file
    let pool1 = open_pool(path_s).await;
    let pool2 = open_pool(path_s).await;

    let table = SqliteEgressTable::default();

    // Create schema from pool1
    ensure_schema(&pool1, &table)
        .await
        .expect("schema via pool1");

    // Insert from pool1
    let e1 = mk_event("0xpool1", "1000", 0, "detected");
    insert_event(&pool1, &e1, &table)
        .await
        .expect("insert via pool1");

    // Insert from pool2 concurrently
    let e2 = mk_event("0xpool2", "2000", 0, "detected");
    insert_event(&pool2, &e2, &table)
        .await
        .expect("insert via pool2");

    // Verify both rows exist
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM deposit_events")
        .fetch_one(&pool1)
        .await
        .expect("count via pool1");
    assert_eq!(count.0, 2, "both inserts should be visible");

    // Verify from pool2 as well
    let count2: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM deposit_events")
        .fetch_one(&pool2)
        .await
        .expect("count via pool2");
    assert_eq!(count2.0, 2, "both rows visible from pool2");
}

// ── Edge: SQL injection via custom table/column names rejected ───────────

#[tokio::test]
async fn sql_injection_via_custom_table_name_does_not_execute_malicious_sql() {
    use pano::config::AppConfig;

    // Verify is_valid_sql_identifier rejects injection patterns
    assert!(!AppConfig::is_valid_sql_identifier(
        "deposits; DROP TABLE deposit_events; --"
    ));
    assert!(!AppConfig::is_valid_sql_identifier(
        "t; DELETE FROM deposit_events WHERE 1=1; --"
    ));

    // Now verify that even if validation were bypassed, the real data is safe.
    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("inject_test.db");
    let path_s = db_path.to_str().expect("utf8");

    let pool = open_pool(path_s).await;

    // Create a legitimate table with data we want to protect
    let real_table = SqliteEgressTable::default();
    ensure_schema(&pool, &real_table)
        .await
        .expect("create real table");

    let event = mk_event("0xprotect", "5000", 0, "detected");
    insert_event(&pool, &event, &real_table)
        .await
        .expect("insert real event");

    // Attempt to use an injection name as a custom table
    let malicious_table = SqliteEgressTable {
        name: "deposits; DROP TABLE deposit_events; --".to_string(),
        columns: SqliteEgressColumns::default(),
    };

    // This should either fail or create a weirdly-named table.
    // It must NOT drop the real table or delete its data.
    let schema_result = ensure_schema(&pool, &malicious_table).await;

    // Verify the real table and its data are intact
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM deposit_events")
        .fetch_one(&pool)
        .await
        .expect("count real table");
    assert_eq!(
        count.0, 1,
        "real table should not be affected by injection attempt"
    );

    // Verify the row is still there with correct data
    let row: (String,) = sqlx::query_as("SELECT tx_id FROM deposit_events")
        .fetch_one(&pool)
        .await
        .expect("fetch real row");
    assert_eq!(row.0, "0xprotect", "real data should be intact");

    // Whether schema_result succeeded or failed, data safety is what matters
    let _ = schema_result;
}

#[tokio::test]
async fn sql_injection_via_custom_column_name_rejected_by_validator() {
    use pano::config::AppConfig;

    // Verify is_valid_sql_identifier rejects injection in column names
    assert!(!AppConfig::is_valid_sql_identifier(
        "tx_id; DELETE FROM deposit_events; --"
    ));
    assert!(!AppConfig::is_valid_sql_identifier("col\"\nDROP TABLE"));
    assert!(!AppConfig::is_valid_sql_identifier("col--comment"));
}
