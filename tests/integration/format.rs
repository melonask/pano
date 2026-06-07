use pano::model::*;
use pano::shared::format::*;

mod common;

// ── infer_format ──────────────────────────────────────────────────────────

#[test]
fn infer_json() {
    assert_eq!(infer_format("events.json"), FileFormat::Json);
}

#[test]
fn infer_json_uppercase() {
    assert_eq!(infer_format("events.JSON"), FileFormat::Json);
}

#[test]
fn infer_csv() {
    assert_eq!(infer_format("events.csv"), FileFormat::Csv);
}

#[test]
fn infer_csv_uppercase() {
    assert_eq!(infer_format("events.CSV"), FileFormat::Csv);
}

#[test]
fn infer_jsonl() {
    assert_eq!(infer_format("events.jsonl"), FileFormat::Jsonl);
}

#[test]
fn infer_ndjson() {
    assert_eq!(infer_format("events.ndjson"), FileFormat::Jsonl);
}

#[test]
fn infer_no_extension() {
    assert_eq!(infer_format("events"), FileFormat::Jsonl);
}

#[test]
fn infer_empty() {
    assert_eq!(infer_format(""), FileFormat::Jsonl);
}

// ── serialize_event / serialize_events ────────────────────────────────────

fn make_event() -> DepositEvent {
    common::sample_event()
}

#[test]
fn serialize_event_json() {
    let event = make_event();
    let out = serialize_event(&event, FileFormat::Json).unwrap();
    // Must be valid JSON
    let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(parsed.get("event_id").is_some());
    assert!(parsed.get("event").is_some());
    assert!(parsed.get("version").is_some());
    assert!(parsed.get("occurred_at").is_some());
    let data = parsed.get("data").unwrap();
    assert!(data.get("tx_id").is_some());
    assert!(data.get("caip2").is_some());
    assert!(data.get("symbol").is_some());
    assert!(data.get("address").is_some());
    assert!(data.get("block_number").is_some());
    assert!(data.get("log_index").is_some());
    assert!(data.get("amount").is_some());
    assert!(data.get("sender").is_some());
    assert!(data.get("confirmations").is_some());
    assert!(data.get("timestamp").is_some());
    // internal_egress must NOT appear (serde(skip))
    assert!(data.get("internal_egress").is_none());
}

#[test]
fn serialize_event_internal_egress_absent() {
    let mut data = common::sample_data();
    data.internal_egress = Some(EgressOverride {
        file: Some(FileOverride {
            path: "/tmp/secret.json".to_string(),
        }),
        ..Default::default()
    });
    let event = DepositEvent::detected(data).unwrap();
    let out = serialize_event(&event, FileFormat::Json).unwrap();
    assert!(!out.contains("internal_egress"));
    assert!(!out.contains("secret.json"));
}

#[test]
fn serialize_event_jsonl() {
    let event = make_event();
    let out = serialize_event(&event, FileFormat::Jsonl).unwrap();
    // Single line, no trailing newline
    assert!(!out.contains('\n'));
    assert!(!out.ends_with('\n'));
    // Must be valid JSON
    serde_json::from_str::<serde_json::Value>(&out).unwrap();
}

#[test]
fn serialize_events_jsonl_multiple() {
    let e1 = make_event();
    let e2 = make_event();
    let out = serialize_events(&[e1, e2], FileFormat::Jsonl).unwrap();
    assert_eq!(out.matches('\n').count(), 1);
    let lines: Vec<&str> = out.split('\n').collect();
    assert_eq!(lines.len(), 2);
    for line in lines {
        serde_json::from_str::<serde_json::Value>(line).unwrap();
    }
}

#[test]
fn serialize_event_csv() {
    let event = make_event();
    let out = serialize_event(&event, FileFormat::Csv).unwrap();
    // Single line, no trailing newline
    assert!(!out.contains('\n'));
    let parts: Vec<&str> = out.split(',').collect();
    // CSV writer produces 14 fields
    assert_eq!(parts.len(), 14);
}

#[test]
fn serialize_events_csv_multiple() {
    let e1 = make_event();
    let e2 = make_event();
    let out = serialize_events(&[e1, e2], FileFormat::Csv).unwrap();
    assert_eq!(out.matches('\n').count(), 2);
    assert!(out.ends_with('\n'));
    let lines: Vec<&str> = out.trim_end_matches('\n').split('\n').collect();
    assert_eq!(lines.len(), 2);
}

#[test]
fn serialize_events_empty() {
    let empty: Vec<DepositEvent> = vec![];
    assert_eq!(serialize_events(&empty, FileFormat::Json).unwrap(), "[]");
    assert_eq!(serialize_events(&empty, FileFormat::Jsonl).unwrap(), "");
    assert_eq!(serialize_events(&empty, FileFormat::Csv).unwrap(), "");
}
