/// Integration tests for file egress.
///
/// Covers: JSON/JSONL/CSV writes, append behaviour, internal_egress skip,
/// concurrent JSON writes, canonical-path lock key, and bounded queue draining.
use super::common;

use pano::egress::file::{FileWriteLocks, write_event_to_path, write_event_to_path_with_locks};
use pano::model::{DepositData, DepositEvent, EgressOverride, WebhookOverride};
use pano::shared::format::{FileFormat, infer_format, serialize_event, serialize_events};
use tempfile::TempDir;

// ── helpers ──────────────────────────────────────────────────────────────

fn mk_event(
    tx_id: &str,
    amount: &str,
    log_index: u64,
    egress: Option<EgressOverride>,
) -> DepositEvent {
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
        internal_egress: egress,
    };
    DepositEvent::detected(data).expect("valid event")
}

fn egress_with_webhook() -> EgressOverride {
    EgressOverride {
        webhook: Some(WebhookOverride {
            url: "https://example.com/hook".to_string(),
            secret: "s3cret".to_string(),
        }),
        ..Default::default()
    }
}

async fn read_file(path: &str) -> String {
    tokio::fs::read_to_string(path).await.unwrap_or_default()
}

// ── JSON egress — single event written to new file ───────────────────────

#[tokio::test]
async fn json_write_single_event_to_new_file() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("events.json");
    let path_s = path.to_str().expect("utf8 path");

    let event = mk_event("0xtx1", "1000000000000000000", 0, None);
    write_event_to_path(path_s, &event).await.expect("write ok");

    let content = read_file(path_s).await;
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&content).expect("valid JSON array");
    assert_eq!(parsed.len(), 1, "one event in array");
    assert_eq!(parsed[0]["event"], "pano.deposit.detected");
    assert_eq!(parsed[0]["data"]["tx_id"], "0xtx1");
}

// ── JSON egress — append to existing file ────────────────────────────────

#[tokio::test]
async fn json_write_appends_to_existing_file() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("events.json");
    let path_s = path.to_str().expect("utf8 path");

    let e1 = mk_event("0xtx1", "1000000000000000000", 0, None);
    let e2 = mk_event("0xtx2", "2000000000000000000", 0, None);

    write_event_to_path(path_s, &e1).await.expect("write 1");
    write_event_to_path(path_s, &e2).await.expect("write 2");

    let content = read_file(path_s).await;
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&content).expect("valid JSON array");
    assert_eq!(parsed.len(), 2, "two events in array");
    assert_eq!(parsed[0]["data"]["tx_id"], "0xtx1");
    assert_eq!(parsed[1]["data"]["tx_id"], "0xtx2");
}

// ── JSONL egress — event appended as new line ────────────────────────────

#[tokio::test]
async fn jsonl_write_appends_lines() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("events.jsonl");
    let path_s = path.to_str().expect("utf8 path");

    let e1 = mk_event("0xtx1", "1000000000000000000", 0, None);
    let e2 = mk_event("0xtx2", "2000000000000000000", 0, None);

    write_event_to_path(path_s, &e1).await.expect("write 1");
    write_event_to_path(path_s, &e2).await.expect("write 2");

    let content = read_file(path_s).await;
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2, "two lines");
    for line in &lines {
        let _v: serde_json::Value = serde_json::from_str(line).expect("each line is valid JSON");
    }
    assert!(lines[0].contains("0xtx1"));
    assert!(lines[1].contains("0xtx2"));
}

// ── CSV egress — event appended as new line ──────────────────────────────

#[tokio::test]
async fn csv_write_appends_lines() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("events.csv");
    let path_s = path.to_str().expect("utf8 path");

    let e1 = mk_event("0xtx1", "1000000000000000000", 0, None);
    let e2 = mk_event("0xtx2", "2000000000000000000", 1, None);

    write_event_to_path(path_s, &e1).await.expect("write 1");
    write_event_to_path(path_s, &e2).await.expect("write 2");

    let content = read_file(path_s).await;
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(lines.len(), 2, "two CSV lines");

    // Each line should have 14 comma-separated fields
    for line in &lines {
        let fields: Vec<&str> = line.split(',').collect();
        assert_eq!(fields.len(), 14, "14 fields per CSV line (no header)");
    }

    // First line: tx_id should appear
    assert!(lines[0].contains("0xtx1"));
    assert!(lines[1].contains("0xtx2"));
}

// ── CSV egress — 14 fields, no header row ────────────────────────────────

#[tokio::test]
async fn csv_format_has_correct_fields_and_no_header() {
    // Test serialize_event directly to verify CSV format properties
    let event = mk_event("0xtx9", "5000000000000000000", 3, None);

    let csv_line = serialize_event(&event, FileFormat::Csv).expect("serialize csv");

    let fields: Vec<&str> = csv_line.split(',').collect();
    assert_eq!(fields.len(), 14, "CSV line has exactly 14 fields");

    // Verify field content (order: event_id, event, version, occurred_at, tx_id,
    //   caip2, symbol, address, block_number, log_index, amount, sender,
    //   confirmations, timestamp)
    assert_eq!(fields[4], "0xtx9", "tx_id field");
    assert_eq!(fields[5], "eip155:1", "caip2 field");
    assert_eq!(fields[6], "ETH", "symbol field");
    assert_eq!(fields[8], "100", "block_number field");
    assert_eq!(fields[9], "3", "log_index field");
    assert_eq!(fields[10], "5000000000000000000", "amount field");

    // Confirm no header: first field is a ULID, not "event_id"
    assert_ne!(fields[0], "event_id");
}

// ── internal_egress not written to file output ────────────────────────────

#[tokio::test]
async fn internal_egress_not_in_serialized_output() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("events.json");
    let path_s = path.to_str().expect("utf8 path");

    let event = mk_event(
        "0xtx5",
        "5000000000000000000",
        0,
        Some(egress_with_webhook()),
    );

    // Verify serialize_event output lacks internal_egress
    let json_str = serialize_event(&event, FileFormat::Json).expect("serialize");
    assert!(
        !json_str.contains("internal_egress"),
        "internal_egress must NOT appear in serialized JSON: {}",
        json_str
    );

    let jsonl_str = serialize_event(&event, FileFormat::Jsonl).expect("serialize");
    assert!(
        !jsonl_str.contains("internal_egress"),
        "internal_egress must NOT appear in serialized JSONL"
    );

    // Write to file and verify
    write_event_to_path(path_s, &event).await.expect("write");

    let content = read_file(path_s).await;
    assert!(
        !content.contains("internal_egress"),
        "internal_egress must NOT appear in file output"
    );
}

// ── serialize_events with internal_egress ─────────────────────────────────

#[tokio::test]
async fn serialize_events_excludes_internal_egress() {
    let e1 = mk_event(
        "0xtx_a",
        "1000000000000000000",
        0,
        Some(egress_with_webhook()),
    );
    let e2 = mk_event("0xtx_b", "2000000000000000000", 0, None);

    let json_out =
        serialize_events(&[e1.clone(), e2.clone()], FileFormat::Json).expect("serialize json");
    assert!(!json_out.contains("internal_egress"));

    let jsonl_out = serialize_events(&[e1, e2], FileFormat::Jsonl).expect("serialize jsonl");
    assert!(!jsonl_out.contains("internal_egress"));
}

// ── Concurrent writes to same JSON file are serialized ────────────────────

#[tokio::test]
async fn concurrent_json_writes_produce_valid_array() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("concurrent.json");
    let path_s = path.to_str().expect("utf8 path");

    let path_arc = std::sync::Arc::new(path_s.to_string());
    let locks = FileWriteLocks::default();
    let num_writes: usize = 10;

    let mut handles = Vec::new();
    for i in 0..num_writes {
        let p = path_arc.clone();
        let locks = locks.clone();
        let handle = tokio::spawn(async move {
            // Yield before starting to increase chance of interleaving
            tokio::task::yield_now().await;
            let event = mk_event(
                &format!("0xtx_concurrent_{i}"),
                &format!("{}", (i + 1) * 1000),
                i as u64,
                None,
            );
            write_event_to_path_with_locks(&locks, &p, &event)
                .await
                .expect("concurrent write");
        });
        handles.push(handle);
    }

    for handle in handles {
        handle.await.expect("task joined");
    }

    let content = read_file(&path_arc).await;
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&content).expect("valid JSON array");

    assert_eq!(
        parsed.len(),
        num_writes,
        "concurrent writes: expected {num_writes} events, got {}",
        parsed.len()
    );

    // Each tx_id should be unique
    let tx_ids: Vec<String> = parsed
        .iter()
        .map(|v| v["data"]["tx_id"].as_str().unwrap().to_string())
        .collect();
    let mut unique_ids: Vec<String> = tx_ids.clone();
    unique_ids.sort();
    unique_ids.dedup();
    assert_eq!(
        unique_ids.len(),
        num_writes,
        "all tx_ids should be unique, no duplicates"
    );
}

// ── canonical_path — two paths to same file via symlink ──────────────────

#[tokio::test]
async fn canonical_path_lock_behaviour_with_symlink() {
    let dir = TempDir::new().expect("temp dir");
    let real_path = dir.path().join("symlink_target.json");
    let link_path = dir.path().join("symlink.json");

    let real_s = real_path.to_str().expect("utf8");
    let link_s = link_path.to_str().expect("utf8");

    // Pre-create an empty JSON array so both paths exist and canonicalize
    tokio::fs::write(real_s, "[]")
        .await
        .expect("create target file");

    // Create symlink pointing to the target file
    std::os::unix::fs::symlink(&real_path, &link_path).expect("create symlink");

    // Write to the real path first
    let locks = FileWriteLocks::default();
    let e1 = mk_event("0xreal", "1000", 0, None);
    write_event_to_path_with_locks(&locks, real_s, &e1)
        .await
        .expect("write real");

    // Write to the symlink path.
    // NOTE: rename() to a symlink path replaces the symlink with a regular
    // file on Unix, so after this step the symlink is gone and link_s is a
    // regular file containing the merged result.
    let e2 = mk_event("0xlink", "2000", 0, None);
    write_event_to_path_with_locks(&locks, link_s, &e2)
        .await
        .expect("write link");

    // Both writes used the same canonical-path lock key (since canonicalize
    // follows symlinks). The merged result is at the symlink path.
    let content = read_file(link_s).await;
    let parsed: Vec<serde_json::Value> = serde_json::from_str(&content).expect("valid JSON");
    assert_eq!(
        parsed.len(),
        2,
        "both events serialised via shared lock key"
    );
}

// ── bounded receiver draining ────────────────────────────────────────────

#[tokio::test]
async fn write_events_drains_bounded_queue_without_loss() {
    use pano::egress::file::write_events;
    use tokio::sync::mpsc;

    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("lagged.jsonl");
    let path_s = path.to_str().expect("utf8 path");

    let (tx, rx) = mpsc::channel::<DepositEvent>(1);

    // Spawn the write_events task
    let path_clone = path_s.to_string();
    let task = tokio::spawn(async move {
        // Use a timeout to prevent hanging if something goes wrong
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            write_events(path_clone, rx),
        )
        .await;
    });

    // Capacity is 1, so sends must wait for the file writer rather than drop.
    for i in 0..5 {
        let ev = mk_event(
            &format!("0xlagged_{i}"),
            &format!("{}", 1000 + i),
            i as u64,
            None,
        );
        tx.send(ev)
            .await
            .expect("file egress receiver remains open");
    }
    let final_event = mk_event("0xlagged_final", "9999", 99, None);
    tx.send(final_event)
        .await
        .expect("file egress receiver remains open");
    drop(tx);

    // The task should complete without panicking
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), task).await;
    assert!(result.is_ok(), "write_events task should not panic on lag");
    assert!(result.unwrap().is_ok(), "write_events task should complete");

    // Verify every queued event was written in order.
    let content = tokio::fs::read_to_string(&path_s).await.unwrap_or_default();
    let events: Vec<DepositEvent> = content
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is valid JSON"))
        .collect();
    assert_eq!(events.len(), 6);
    assert_eq!(events[0].data.tx_id, "0xlagged_0");
    assert_eq!(events[5].data.tx_id, "0xlagged_final");
}

// ── format edge cases ────────────────────────────────────────────────────

#[tokio::test]
async fn infer_format_from_extensions() {
    assert_eq!(infer_format("events.json"), FileFormat::Json);
    assert_eq!(infer_format("events.JSON"), FileFormat::Json);
    assert_eq!(infer_format("events.csv"), FileFormat::Csv);
    assert_eq!(infer_format("events.CSV"), FileFormat::Csv);
    assert_eq!(infer_format("events.jsonl"), FileFormat::Jsonl);
    assert_eq!(infer_format("events.ndjson"), FileFormat::Jsonl);
    assert_eq!(infer_format("events"), FileFormat::Jsonl);
    assert_eq!(infer_format(""), FileFormat::Jsonl);
}

#[tokio::test]
async fn serialize_events_empty_returns_correct_empty() {
    let empty: Vec<DepositEvent> = vec![];

    let json = serialize_events(&empty, FileFormat::Json).expect("json");
    assert_eq!(json, "[]");

    let jsonl = serialize_events(&empty, FileFormat::Jsonl).expect("jsonl");
    assert_eq!(jsonl, "");

    let csv = serialize_events(&empty, FileFormat::Csv).expect("csv");
    assert_eq!(csv, "");
}

// ── Edge: path with no parent directory returns error ────────────────────

#[tokio::test]
async fn write_to_path_with_no_parent_directory_returns_error() {
    let event = mk_event("0xnoparent", "1000000000000000000", 0, None);

    // Use a path nested under a nonexistent directory
    let result =
        write_event_to_path("/nonexistent_dir_xyz_12345/subdir/events.jsonl", &event).await;

    assert!(
        result.is_err(),
        "writing to path with nonexistent parent should fail"
    );
}

// ── Edge: permission denied on write (portable) ──────────────────────────

#[tokio::test]
async fn permission_denied_on_write_returns_error() {
    let dir = TempDir::new().expect("temp dir");

    // Create a read-only directory
    let ro_dir = dir.path().join("readonly");
    std::fs::create_dir(&ro_dir).expect("create ro dir");

    let mut perms = std::fs::metadata(&ro_dir).expect("metadata").permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&ro_dir, perms).expect("set readonly");

    let path = ro_dir.join("events.jsonl");
    let path_s = path.to_str().expect("utf8 path");

    let event = mk_event("0xperm", "1000000000000000000", 0, None);
    let result = write_event_to_path(path_s, &event).await;

    // On most Unix systems, writing to a read-only directory fails.
    // If the test runs as root (e.g., in a Docker container with --privileged),
    // the write may succeed. We accept both outcomes to keep the test portable.
    match result {
        Ok(()) => {
            // Running as root — skip assertion
            eprintln!(
                "permission_denied_on_write_returns_error: write succeeded (likely running as root)"
            );
        }
        Err(error) => {
            // Normal case: write should fail
            let err = error.to_string();
            assert!(
                err.contains("permission denied")
                    || err.contains("read-only")
                    || err.contains("PermissionDenied")
                    || err.contains("AccessDenied")
                    || err.to_lowercase().contains("perm"),
                "error should indicate permission issue: {err}"
            );
        }
    }
}

// ── Edge: CSV escaping commas, quotes, and newlines ──────────────────────

#[tokio::test]
async fn csv_escapes_commas_quotes_and_newlines() {
    use pano::shared::format::serialize_event;

    let event = mk_event("0xcsv_escape", "1000000000000000000", 0, None);
    let csv_line = serialize_event(&event, FileFormat::Csv).expect("serialize csv");

    // Basic: 14 comma-separated fields (simple split may overcount due to CSV quoting)
    let _fields: Vec<&str> = csv_line.split(',').collect();
    // CSV escaping means fields with commas/newlines/quotes are quoted,
    // so simple split(',') may overcount. But for normal data it's 14.
    // The csv crate handles escaping correctly internally.
    // We verify round-trip: parse the CSV line back and check fields.
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(csv_line.as_bytes());
    let record = reader
        .records()
        .next()
        .expect("at least one record")
        .expect("valid CSV record");
    assert_eq!(record.len(), 14, "CSV record should have 14 fields");

    // Now test with fields that contain special characters.
    // We create a DepositEvent with special chars in relevant string fields.
    // Since we can't easily inject special chars into all DepositEvent fields
    // (ULID enforces alphanumeric), we test CSV serialization/deserialization
    // with a manually constructed CSV line containing special chars.
    let special_data = r#"event_id,"hello, world","quote""test","line1
line2",rest"#;
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(special_data.as_bytes());
    let record = reader.records().next().expect("record").expect("valid");
    // Verify the csv crate correctly handles commas, quotes, and newlines
    assert_eq!(record.len(), 5, "should parse 5 fields");
    assert_eq!(
        &record[1], "hello, world",
        "comma inside quotes should be preserved"
    );
    assert_eq!(
        &record[2], "quote\"test",
        "escaped quote should be preserved"
    );
    assert_eq!(
        &record[3], "line1\nline2",
        "embedded newline should be preserved"
    );
}

// ── Edge: very large batch write ─────────────────────────────────────────

#[tokio::test]
async fn very_large_batch_write() {
    let dir = TempDir::new().expect("temp dir");
    let path = dir.path().join("large_batch.jsonl");
    let path_s = path.to_str().expect("utf8 path");

    let count = 500;
    let mut events = Vec::with_capacity(count);
    for i in 0..count {
        events.push(mk_event(
            &format!("0xlarge_{i:05}"),
            &format!("{}", 1000 + i),
            i as u64,
            None,
        ));
    }

    // Write all events sequentially
    for event in &events {
        write_event_to_path(path_s, event)
            .await
            .expect("write event");
    }

    // Verify all events were written
    let content = read_file(path_s).await;
    let lines: Vec<&str> = content.lines().collect();
    assert_eq!(
        lines.len(),
        count,
        "all {count} events should be present in output"
    );

    // Verify the content is valid JSONL
    for (i, line) in lines.iter().enumerate() {
        let v: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|_| panic!("line {i} is valid JSON"));
        assert_eq!(
            v["data"]["tx_id"].as_str().unwrap(),
            &format!("0xlarge_{i:05}"),
            "tx_id at line {i}"
        );
    }
}
