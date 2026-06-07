/// Integration tests for the delivery router.
///
/// Covers: basic routing, per-event override precedence, fan-out,
/// orphaned event handling, concurrent load, channel isolation, and
/// URL normalization for connection pool deduplication.
mod common;

use pano::delivery::{EgressRouter, normalize_pool_key};
use pano::model::{
    DepositData, DepositEvent, EgressOverride, FileOverride, SqliteOverride, TableOverride,
};
use tempfile::TempDir;

// ── helpers ──────────────────────────────────────────────────────────────

fn mk_event_with_egress(egress: EgressOverride) -> DepositEvent {
    let data = DepositData {
        tx_id: "0xdeadbeef".to_string(),
        caip2: "eip155:1".to_string(),
        symbol: "ETH".to_string(),
        address: common::EVM_ADDR.to_string(),
        block_number: 100,
        log_index: 0,
        amount: "1000000000000000000".to_string(),
        sender: common::EVM_SENDER.to_string(),
        confirmations: 1,
        timestamp: "2026-06-04T00:00:00Z".to_string(),
        internal_egress: Some(egress),
    };
    DepositEvent::detected(data).expect("valid detected event")
}

fn mk_event_with_no_egress() -> DepositEvent {
    let data = DepositData {
        tx_id: "0xdeadbeef".to_string(),
        caip2: "eip155:1".to_string(),
        symbol: "ETH".to_string(),
        address: common::EVM_ADDR.to_string(),
        block_number: 100,
        log_index: 0,
        amount: "1000000000000000000".to_string(),
        sender: common::EVM_SENDER.to_string(),
        confirmations: 1,
        timestamp: "2026-06-04T00:00:00Z".to_string(),
        internal_egress: None,
    };
    DepositEvent::detected(data).expect("valid detected event")
}

fn read_file_content(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// Open a SQLite pool for verification, creating the database if missing.
async fn open_verification_pool(path: &str) -> sqlx::SqlitePool {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("open sqlite pool for verification")
}

// ── normalize_pool_key deduplicates equivalent URLs ───────────────────────

#[tokio::test]
async fn normalize_pool_key_deduplicates_equivalent_urls() {
    // Trailing slash is stripped for valid URLs.
    let a = normalize_pool_key("postgres://host/db");
    let b = normalize_pool_key("postgres://host/db/");
    assert_eq!(a, b, "trailing slash normalized");

    // Host is lowercased.
    let c = normalize_pool_key("postgres://HOST.COM/db");
    let d = normalize_pool_key("postgres://host.com/db");
    assert_eq!(c, d, "host case normalized");

    // Query parameter ordering is normalized.
    let e = normalize_pool_key("postgres://host/db?sslmode=require&foo=bar");
    let f = normalize_pool_key("postgres://host/db?foo=bar&sslmode=require");
    assert_eq!(e, f, "query param order normalized");

    // Different credentials produce different keys (identity-sensitive).
    let g = normalize_pool_key("postgres://user1:pass@host/db");
    let h = normalize_pool_key("postgres://user2:pass@host/db");
    assert_ne!(g, h, "different users produce different keys");

    // Invalid URL is returned as-is (fallback).
    let i = normalize_pool_key("not a valid url!!!");
    assert_eq!(i, "not a valid url!!!", "invalid URL returned as-is");

    // Port is preserved as part of the URL.
    let j = normalize_pool_key("postgres://host:5432/db");
    let k = normalize_pool_key("postgres://host:5433/db");
    assert_ne!(j, k, "different ports produce different keys");

    // Non-URL strings are returned as-is (used for bare filesystem paths).
    // Trailing slashes are NOT normalized on bare paths since they don't
    // parse as valid URLs (no scheme) — this is by design.
    let l = normalize_pool_key("/tmp/mydb.sqlite");
    assert_eq!(l, "/tmp/mydb.sqlite", "bare path returned as-is");

    // Query parameter values are preserved.
    let m = normalize_pool_key("postgres://host/db?a=1&b=2");
    let n = normalize_pool_key("postgres://host/db?b=2&a=1");
    assert_eq!(m, n, "query param values preserved after normalization");
}

// ── Per-event EgressOverride takes precedence ──────────────────────────────
// No internal_egress causes no delivery. The router uses only the per-event
// override — it does not consult any config-level egress settings.

#[tokio::test]
async fn no_internal_egress_causes_no_delivery() {
    let dir = TempDir::new().expect("temp dir");
    let file_path = dir.path().join("should_not_exist.json");

    let router = EgressRouter::new();
    let event = mk_event_with_no_egress(); // internal_egress = None

    // Must not panic.
    router.route(&event).await;

    // No file should have been created since the router returned early.
    assert!(
        !file_path.exists(),
        "no file should be created when internal_egress is None"
    );
}

// ── Event routed to correct egress based on override ──────────────────────

#[tokio::test]
async fn event_routed_to_file_egress() {
    let dir = TempDir::new().expect("temp dir");
    let file_path = dir.path().join("events.json");
    let file_path_s = file_path.to_str().expect("utf8");

    let router = EgressRouter::new();
    let event = mk_event_with_egress(EgressOverride {
        file: Some(FileOverride {
            path: file_path_s.to_string(),
        }),
        ..Default::default()
    });

    router.route(&event).await;

    // File should exist and contain the event.
    let content = read_file_content(file_path_s);
    assert!(!content.is_empty(), "file should contain event data");
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&content).expect("valid JSON array");
    assert_eq!(parsed.len(), 1, "one event written");
    assert_eq!(parsed[0]["event"], "pano.deposit.detected");
    assert_eq!(parsed[0]["data"]["tx_id"], "0xdeadbeef");
}

#[tokio::test]
async fn event_routed_to_sqlite_egress() {
    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("test.db");
    let db_path_s = db_path.to_str().expect("utf8");

    let router = EgressRouter::new();
    let event = mk_event_with_egress(EgressOverride {
        sqlite: Some(SqliteOverride {
            path: db_path_s.to_string(),
            table: None,
        }),
        ..Default::default()
    });

    router.route(&event).await;

    // Verify the event was inserted into the SQLite database.
    let pool = open_verification_pool(db_path_s).await;

    let row_count: (i32,) = sqlx::query_as("SELECT COUNT(*) FROM deposit_events")
        .fetch_one(&pool)
        .await
        .expect("count rows");
    assert_eq!(row_count.0, 1, "one event inserted");

    let tx_id_row: (String,) = sqlx::query_as("SELECT tx_id FROM deposit_events")
        .fetch_one(&pool)
        .await
        .expect("fetch row");
    assert_eq!(tx_id_row.0, "0xdeadbeef", "correct tx_id stored");
}

#[tokio::test]
async fn sqlite_override_creates_custom_table_schema() {
    let dir = TempDir::new().expect("temp dir");
    let db_path = dir.path().join("custom.db");
    let db_path_s = db_path.to_str().expect("utf8");

    let router = EgressRouter::new();
    let event = mk_event_with_egress(EgressOverride {
        sqlite: Some(SqliteOverride {
            path: db_path_s.to_string(),
            table: Some(TableOverride {
                name: "custom_events".to_string(),
            }),
        }),
        ..Default::default()
    });

    router.route(&event).await;

    let pool = open_verification_pool(db_path_s).await;
    let row_count: (i32,) = sqlx::query_as("SELECT COUNT(*) FROM custom_events")
        .fetch_one(&pool)
        .await
        .expect("count rows in custom table");
    assert_eq!(row_count.0, 1, "one event inserted into custom table");
}

// ── Fan-out — single event to multiple channels ───────────────────────────

#[tokio::test]
async fn fan_out_to_multiple_channels() {
    let dir = TempDir::new().expect("temp dir");
    let file_path = dir.path().join("events.json");
    let file_path_s = file_path.to_str().expect("utf8");
    let db_path = dir.path().join("test.db");
    let db_path_s = db_path.to_str().expect("utf8");

    let router = EgressRouter::new();
    let event = mk_event_with_egress(EgressOverride {
        file: Some(FileOverride {
            path: file_path_s.to_string(),
        }),
        sqlite: Some(SqliteOverride {
            path: db_path_s.to_string(),
            table: None,
        }),
        ..Default::default()
    });

    router.route(&event).await;

    // Both file and sqlite should have received the event.
    let content = read_file_content(file_path_s);
    assert!(!content.is_empty(), "file should contain event");
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&content).expect("valid JSON array");
    assert_eq!(parsed.len(), 1, "one event in file");

    let pool = open_verification_pool(db_path_s).await;
    let row_count: (i32,) = sqlx::query_as("SELECT COUNT(*) FROM deposit_events")
        .fetch_one(&pool)
        .await
        .expect("count rows");
    assert_eq!(row_count.0, 1, "one event in sqlite");
}

// ── Event with no matching egress handled gracefully ──────────────────────
// When internal_egress is Some but all fields are None, no channel matches.
// The event is consumed without error — no panic, no file written.

#[tokio::test]
async fn event_with_empty_egress_override_no_panic() {
    let dir = TempDir::new().expect("temp dir");
    let some_path = dir.path().join("nothing.json");

    let router = EgressRouter::new();
    // internal_egress is Some but all fields are None (default).
    let event = mk_event_with_egress(EgressOverride::default());

    // Must not panic.
    router.route(&event).await;

    // No file should have been written — no channel matched.
    assert!(
        !some_path.exists(),
        "no file should be created when no egress channel matches"
    );
}

// ── Failed delivery to one channel doesn't block another ──────────────────
// File egress succeeds even when sqlite egress fails (bad path).
// Each channel handler catches its own errors; the router does not
// short-circuit on error.

#[tokio::test]
async fn failed_delivery_to_one_channel_does_not_block_another() {
    let dir = TempDir::new().expect("temp dir");
    let file_path = dir.path().join("events.json");
    let file_path_s = file_path.to_str().expect("utf8");

    // sqlite path in a non-existent subdirectory — will fail to create pool.
    let bad_sqlite_path = dir.path().join("non_existent_subdir").join("test.db");
    let bad_sqlite_path_s = bad_sqlite_path.to_str().expect("utf8");

    let router = EgressRouter::new();
    let event = mk_event_with_egress(EgressOverride {
        file: Some(FileOverride {
            path: file_path_s.to_string(),
        }),
        sqlite: Some(SqliteOverride {
            path: bad_sqlite_path_s.to_string(),
            table: None,
        }),
        ..Default::default()
    });

    // This should not panic, and file delivery should succeed.
    router.route(&event).await;

    // File should contain the event despite sqlite failure.
    let content = read_file_content(file_path_s);
    assert!(!content.is_empty(), "file delivery succeeded");
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&content).expect("valid JSON array");
    assert_eq!(parsed.len(), 1, "one event written to file");
    assert_eq!(parsed[0]["data"]["tx_id"], "0xdeadbeef");

    // Verify the bad sqlite path still doesn't exist (no crash, no partial db).
    assert!(
        !bad_sqlite_path.exists(),
        "failed sqlite path should not have created a file"
    );
}

// ── Route completes under concurrent load ─────────────────────────────────
// Multiple concurrent route() calls all finish without deadlock or panic.
// The router is tested with file-only delivery to avoid SQLite contention.

#[tokio::test(flavor = "multi_thread")]
async fn route_completes_under_concurrent_load() {
    let dir = TempDir::new().expect("temp dir");
    let file_path = dir.path().join("events.json");
    let file_path_s = file_path.to_str().expect("utf8");
    let db_path = dir.path().join("test.db");
    let db_path_s = db_path.to_str().expect("utf8");

    let router = EgressRouter::new();
    let egress = EgressOverride {
        file: Some(FileOverride {
            path: file_path_s.to_string(),
        }),
        sqlite: Some(SqliteOverride {
            path: db_path_s.to_string(),
            table: None,
        }),
        ..Default::default()
    };

    // Build 20 unique events.
    let events: Vec<DepositEvent> = (0..20)
        .map(|i| {
            let data = DepositData {
                tx_id: format!("0xconc{:02x}", i),
                caip2: "eip155:1".to_string(),
                symbol: "ETH".to_string(),
                address: common::EVM_ADDR.to_string(),
                block_number: 100 + i as u64,
                log_index: i as u64,
                amount: "1000000000000000000".to_string(),
                sender: common::EVM_SENDER.to_string(),
                confirmations: 1,
                timestamp: "2026-06-04T00:00:00Z".to_string(),
                internal_egress: Some(egress.clone()),
            };
            DepositEvent::detected(data).expect("valid event")
        })
        .collect();

    // Spawn all routes concurrently.
    let handles: Vec<tokio::task::JoinHandle<()>> = events
        .iter()
        .map(|ev| {
            let r = router.clone();
            let ev_clone = ev.clone();
            tokio::spawn(async move {
                r.route(&ev_clone).await;
            })
        })
        .collect();

    for h in handles {
        h.await.expect("task should complete without panic");
    }

    // Verify file received all events.
    let content = read_file_content(file_path_s);
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&content).expect("valid JSON array");
    assert_eq!(parsed.len(), 20, "all 20 events written to file");

    // Verify sqlite received all events.
    let pool = open_verification_pool(db_path_s).await;
    let row_count: (i32,) = sqlx::query_as("SELECT COUNT(*) FROM deposit_events")
        .fetch_one(&pool)
        .await
        .expect("count rows");
    assert_eq!(row_count.0, 20, "all 20 events in sqlite");
}
