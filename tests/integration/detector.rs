// ── Detector Loop Integration Tests ───────────────────────────────────────
// Covers all detector loop test cases.
// Uses mockito RPC servers and broadcast/channel observation.
use super::common::{EVM_ADDR, EVM_ADDR_LOWER, EVM_SENDER, sample_data};
use mockito::{Matcher, Server};
use pano::config::{
    AppConfig, AssetConfig, ChainConfig, DetectorConfig, EgressConfig, IngressConfig,
    OverrideConfig, RpcOptions, ServerConfig,
};
use pano::detector::remember_event_key;
use pano::detector::util::deposit_event_key;
use pano::egress::EgressHandle;
use pano::ingress::IngressHandle;
use pano::model::{Command, DepositData, DepositEvent, DepositStatus, WatchSpec};
use serde_json::json;
use std::collections::VecDeque;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

// ── Helpers ─────────────────────────────────────────────────────────

/// Create a minimal AppConfig for detector tests.
fn test_app_config(chains: Vec<ChainConfig>, dedup_window: usize) -> AppConfig {
    AppConfig {
        server: ServerConfig {
            enabled: false,
            bind: String::new(),
            port: 0,
            prefix: String::new(),
            dashboard: String::new(),
            dashboard_export: false,
            api_key: String::new(),
            shutdown_timeout_secs: 1,
        },
        detector: DetectorConfig {
            dedup_window_size: dedup_window.max(1),
            delivery_workers: 1,
            delivery_queue_capacity: 16,
            command_queue_capacity: 256,
            stale_event_eviction_multiplier: 10,
            stale_event_eviction_min_blocks: 1_000,
            max_decimals: 30,
        },
        chains,
        ingress: IngressConfig::default(),
        egress: EgressConfig::default(),
        override_: OverrideConfig::default(),
    }
}

/// Single EVM chain with `scan_interval_secs = 1`, ETH-only native asset,
/// and `max_native_scan_per_cycle = 1` so each cycle scans at most 1 block.
fn eth_chain(rpc_url: String) -> ChainConfig {
    ChainConfig {
        caip2: "eip155:1".to_string(),
        start_block: None,
        end_block: None,
        confirmed_blocks: 12,
        rpc: vec![rpc_url],
        rpc_options: Some(RpcOptions {
            scan_interval_secs: 1,
            scan_lookback_blocks: 0,
            scan_timeout_secs: 5,
            max_native_scan_per_cycle: 1,
            solana_scan_mode: Default::default(),
            max_concurrent: 1,
            delay_ms: 0,
            batch_size: 1,
            evm_log_address_batching: true,
            request_timeout_secs: 5,
            max_retries: 1,
            retry_base_ms: 1,
            solana_max_supported_transaction_version: 0,
        }),
        assets: vec![AssetConfig {
            symbol: "ETH".to_string(),
            contract: None,
            token_program: None,
            decimals: 18,
            min_amount: None,
        }],
    }
}

/// Mock for eth_blockNumber → tip.
fn mock_tip(server: &mut Server, tip_hex: &str) -> mockito::Mock {
    server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_blockNumber"})))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc":"2.0","id":1,"result":tip_hex}).to_string())
        .create()
}

/// Mock for eth_getBlockByNumber returning a block with one transfer to `to_addr`.
fn mock_native_block(
    server: &mut Server,
    timestamp_hex: &str,
    tx_hash: &str,
    from_addr: &str,
    to_addr: &str,
    value_hex: &str,
) -> mockito::Mock {
    server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(
            json!({"method": "eth_getBlockByNumber"}),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({"jsonrpc":"2.0","id":1,"result":{
                "timestamp": timestamp_hex,
                "transactions": [
                    {"hash": tx_hash, "from": from_addr, "to": to_addr, "value": value_hex}
                ]
            }})
            .to_string(),
        )
        .create()
}

/// Mock for eth_getBlockByNumber returning an empty block (no transactions).
fn mock_empty_block(server: &mut Server, timestamp_hex: &str) -> mockito::Mock {
    server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(
            json!({"method": "eth_getBlockByNumber"}),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({"jsonrpc":"2.0","id":1,"result":{
                "timestamp": timestamp_hex,
                "transactions": []
            }})
            .to_string(),
        )
        .create()
}

/// Build a minimal WatchSpec for a single address/chain/symbol.
fn watch_spec(address: &str, _caip2: &str, _symbol: &str) -> WatchSpec {
    WatchSpec {
        address: Some(address.to_string()),
        chains: vec![],
        egress: None,
    }
}

/// Wait for and collect events from the broadcast channel within the timeout.
async fn collect_events(
    rx: &mut broadcast::Receiver<DepositEvent>,
    timeout: Duration,
    max_count: usize,
) -> Vec<DepositEvent> {
    let mut events = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    while events.len() < max_count {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(event)) => events.push(event),
            Ok(Err(broadcast::error::RecvError::Lagged(n))) => {
                eprintln!("broadcast lagged by {n}, skipping");
                continue;
            }
            Ok(Err(broadcast::error::RecvError::Closed)) => break,
            Err(_timeout) => break,
        }
    }
    events
}

/// Helper: create ingress/egress handles, start detector, and subscribe to events.
fn start_detector(
    config: AppConfig,
) -> (
    pano::detector::DetectorHandle,
    tokio::task::JoinHandle<()>,
    broadcast::Receiver<DepositEvent>,
) {
    let (_ingress_tx, ingress_rx) = mpsc::channel::<Command>(256);
    let ingress = IngressHandle {
        command_rx: ingress_rx,
    };
    let (events_tx, _) = broadcast::channel::<DepositEvent>(4096);
    let egress = EgressHandle {
        event_tx: events_tx.clone(),
    };
    let (handle, task) =
        pano::detector::start_with_tasks(config, ingress, egress).expect("detector start");
    let events_rx = events_tx.subscribe();
    (handle, task, events_rx)
}

// ── Full scan cycle — detect → emit → deliver ────────────────────────────

#[tokio::test]
async fn full_scan_cycle_detect_emit_deliver() {
    let mut server = Server::new_async().await;
    let url = server.url();

    // Tip = block 100
    let _tip = mock_tip(&mut server, "0x64");
    // Block 100 contains 1 ETH deposit to our address
    let _block = mock_native_block(
        &mut server,
        "0x665f9a80",
        "0xdeposit1",
        EVM_SENDER,
        EVM_ADDR_LOWER,
        "0xde0b6b3a7640000", // 1 ETH
    );

    let chain = eth_chain(url);
    let config = test_app_config(vec![chain], 100_000);
    let (handle, task, mut events_rx) = start_detector(config);

    // Send a watch command
    let spec = watch_spec(EVM_ADDR, "eip155:1", "ETH");
    handle
        .cmd_tx
        .send(Command::Watch(Box::new(spec)))
        .await
        .expect("send watch");

    // Wait for scan cycle (scan_interval_secs = 1)
    let collected = collect_events(&mut events_rx, Duration::from_secs(3), 2).await;

    assert!(
        !collected.is_empty(),
        "expected at least 1 detected event within timeout"
    );
    let detected = collected
        .iter()
        .find(|e| e.event == DepositStatus::Detected.event_name())
        .expect("expected a detected event");
    assert_eq!(
        detected.data.address, EVM_ADDR_LOWER,
        "address should be normalized"
    );
    assert_eq!(detected.data.symbol, "ETH");
    assert_eq!(detected.data.amount, "1000000000000000000");
    assert_eq!(detected.data.block_number, 100);
    assert_eq!(detected.data.tx_id, "0xdeposit1");

    // Shutdown
    handle
        .cmd_tx
        .send(Command::Shutdown)
        .await
        .expect("send shutdown");
    let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
}

// ── Confirmation tracking — detected → confirmed ─────────────────────────

#[tokio::test]
async fn confirmation_tracking_detected_to_confirmed() {
    let mut server = Server::new_async().await;
    let url = server.url();

    // Cycle 1: tip = 100, deposit at block 100
    let _tip1 = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_blockNumber"})))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc":"2.0","id":1,"result":"0x64"}).to_string())
        .expect(1)
        .create();
    let _block1 = mock_native_block(
        &mut server,
        "0x665f9a80",
        "0xdeposit2",
        EVM_SENDER,
        EVM_ADDR_LOWER,
        "0xde0b6b3a7640000",
    );

    // Cycle 2: tip = 112 (≥ 12 confirmations). Block 101 is EMPTY — no new deposit.
    let _tip2 = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_blockNumber"})))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc":"2.0","id":1,"result":"0x70"}).to_string())
        .expect(1)
        .create();
    let _block2 = mock_empty_block(&mut server, "0x665f9a80");

    let chain = eth_chain(url);
    let config = test_app_config(vec![chain], 100_000);
    let (handle, task, mut events_rx) = start_detector(config);

    handle
        .cmd_tx
        .send(Command::Watch(Box::new(watch_spec(
            EVM_ADDR, "eip155:1", "ETH",
        ))))
        .await
        .expect("send watch");

    // Collect events across 2 scan cycles (2 × 1s + buffer)
    let collected = collect_events(&mut events_rx, Duration::from_secs(4), 3).await;

    let detected: Vec<_> = collected
        .iter()
        .filter(|e| e.event == DepositStatus::Detected.event_name())
        .collect();
    let confirmed: Vec<_> = collected
        .iter()
        .filter(|e| e.event == DepositStatus::Confirmed.event_name())
        .collect();

    assert_eq!(
        detected.len(),
        1,
        "expected exactly 1 detected event, got {collected:?}"
    );
    assert_eq!(
        confirmed.len(),
        1,
        "expected exactly 1 confirmed event, got {collected:?}"
    );

    assert_eq!(detected[0].data.block_number, 100);
    assert_eq!(confirmed[0].data.block_number, 100);
    assert_eq!(confirmed[0].data.confirmations, 13); // 112-100+1 = 13
    assert_eq!(confirmed[0].data.tx_id, detected[0].data.tx_id);

    handle
        .cmd_tx
        .send(Command::Shutdown)
        .await
        .expect("send shutdown");
    let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
}

// ── Deduplication across scan cycles ─────────────────────────────────────
// Unit test on the public remember_event_key + deposit_event_key API.
// The dedup mechanism itself is a pure function; testing it directly
// is faster and more precise than orchestrating multi-cycle detector
// loop scans that must contend with lookback/cap interactions.

#[test]
fn deduplication_across_scan_cycles() {
    let dedup_window: usize = 100;
    let mut keys = hashbrown::HashSet::new();
    let mut order = VecDeque::new();

    let data = sample_data();
    let event = DepositEvent::detected(data).expect("valid event");
    let key = deposit_event_key(&event);

    // First insert of a key → accepted (new event)
    assert!(
        remember_event_key(&mut keys, &mut order, dedup_window, key.clone()),
        "first occurrence of an event key must be accepted"
    );

    // Second insert of the same key → rejected (duplicate)
    assert!(
        !remember_event_key(&mut keys, &mut order, dedup_window, key.clone()),
        "duplicate event key must be rejected by dedup"
    );

    // A different event key should be accepted
    let data2 = DepositData {
        tx_id: "0xdifferent".into(),
        ..sample_data()
    };
    let event2 = DepositEvent::detected(data2).expect("valid event");
    let key2 = deposit_event_key(&event2);
    assert!(
        remember_event_key(&mut keys, &mut order, dedup_window, key2),
        "a different event key must be accepted"
    );

    // Original key still rejected
    assert!(
        !remember_event_key(&mut keys, &mut order, dedup_window, key),
        "original key must still be rejected after inserting a different key"
    );
}

// ── Block cursor advancement after successful scan ───────────────────────

#[tokio::test]
async fn cursor_advances_after_successful_scan() {
    let mut server = Server::new_async().await;
    let url = server.url();

    // Cycle 1: tip = 100, scan block 100 → deposit at block 100
    let _tip1 = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_blockNumber"})))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc":"2.0","id":1,"result":"0x64"}).to_string())
        .expect(1)
        .create();
    let _block1 = mock_native_block(
        &mut server,
        "0x665f9a80",
        "0xcursor_a",
        EVM_SENDER,
        EVM_ADDR_LOWER,
        "0xde0b6b3a7640000",
    );

    // Cycle 2: tip = 101, scan block 101 → NEW deposit at block 101
    let _tip2 = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_blockNumber"})))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc":"2.0","id":1,"result":"0x65"}).to_string())
        .expect(1)
        .create();
    let _block2 = mock_native_block(
        &mut server,
        "0x665f9a80",
        "0xcursor_b",
        EVM_SENDER,
        EVM_ADDR_LOWER,
        "0x8ac7230489e80000", // 10 ETH — different amount
    );

    let chain = eth_chain(url);
    let config = test_app_config(vec![chain], 100_000);
    let (handle, task, mut events_rx) = start_detector(config);

    handle
        .cmd_tx
        .send(Command::Watch(Box::new(watch_spec(
            EVM_ADDR, "eip155:1", "ETH",
        ))))
        .await
        .expect("send watch");

    let collected = collect_events(&mut events_rx, Duration::from_secs(4), 5).await;
    let detected: Vec<_> = collected
        .iter()
        .filter(|e| e.event == DepositStatus::Detected.event_name())
        .collect();

    // Two distinct deposits: block 100 and block 101 → cursor advanced
    assert_eq!(
        detected.len(),
        2,
        "cursor should advance; expected 2 deposits at blocks 100 and 101, got {detected:?}"
    );
    let blocks: Vec<u64> = detected.iter().map(|e| e.data.block_number).collect();
    assert!(blocks.contains(&100), "should contain block 100 deposit");
    assert!(blocks.contains(&101), "should contain block 101 deposit");

    let amounts: Vec<&str> = detected.iter().map(|e| e.data.amount.as_str()).collect();
    assert!(
        amounts.contains(&"1000000000000000000"),
        "should have 1 ETH deposit"
    );
    assert!(
        amounts.contains(&"10000000000000000000"),
        "should have 10 ETH deposit"
    );

    handle
        .cmd_tx
        .send(Command::Shutdown)
        .await
        .expect("send shutdown");
    let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
}

// ── Cursor does not advance on scan failure ──────────────────────────────

#[tokio::test]
async fn cursor_does_not_advance_on_scan_failure() {
    let mut server = Server::new_async().await;
    let url = server.url();

    // Use explicit start_block=100 and end_block=102 so scan range is
    // bounded independent of tip changes.  The tip advances but to_block is
    // capped by end_block, so the same block 100 is scanned after failure.

    // Cycle 1: get_tip succeeds (tip=100), but scan fails (block RPC 503).
    let _tip1 = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_blockNumber"})))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc":"2.0","id":1,"result":"0x64"}).to_string())
        .expect(1)
        .create();
    // Block scan fails — after max_retries=1: 2 total attempts (initial + retry)
    let _block_fail = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(
            json!({"method": "eth_getBlockByNumber"}),
        ))
        .with_status(503)
        .expect(2)
        .create();

    // Cycle 2: tip=105, but end_block=102 → to_block=102. Cursor did NOT
    // advance (scan failed), so scan_start = configured_start = 100.
    // Range [100, min(100, effective_cap)] = [100, 100].  Scans block 100.
    let _tip2 = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_blockNumber"})))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc":"2.0","id":1,"result":"0x69"}).to_string())
        .expect(1)
        .create();
    let _block2 = mock_native_block(
        &mut server,
        "0x665f9a80",
        "0xrecover_after_fail",
        EVM_SENDER,
        EVM_ADDR_LOWER,
        "0xde0b6b3a7640000",
    );

    let mut chain = eth_chain(url);
    chain.start_block = Some(100);
    chain.end_block = Some(102); // cap the scan so only block 100 is reachable
    let config = test_app_config(vec![chain], 100_000);
    let (handle, task, mut events_rx) = start_detector(config);

    handle
        .cmd_tx
        .send(Command::Watch(Box::new(watch_spec(
            EVM_ADDR, "eip155:1", "ETH",
        ))))
        .await
        .expect("send watch");

    let collected = collect_events(&mut events_rx, Duration::from_secs(5), 5).await;
    let detected: Vec<_> = collected
        .iter()
        .filter(|e| e.event == DepositStatus::Detected.event_name())
        .collect();

    // The first successful scan after the failure should find the deposit at
    // block 100 — the same block the failed scan would have scanned.  This
    // proves the cursor did not advance after the failure.
    assert!(
        !detected.is_empty(),
        "after scan failure recovery, should detect the deposit; got {detected:?}"
    );
    assert_eq!(
        detected[0].data.block_number, 100,
        "first recovered deposit should be at block 100 (cursor did not advance after failure); got block {}",
        detected[0].data.block_number
    );
    assert_eq!(detected[0].data.tx_id, "0xrecover_after_fail");

    handle
        .cmd_tx
        .send(Command::Shutdown)
        .await
        .expect("send shutdown");
    let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
}

// ── Error recovery — RPC failure mid-scan (get_tip fails) ───────────────

#[tokio::test]
async fn error_recovery_rpc_failure_midscan() {
    let mut server = Server::new_async().await;
    let url = server.url();

    // Cycle 1: get_tip fails entirely → chain is skipped
    let _tip_fail = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_blockNumber"})))
        .with_status(503)
        .expect(2) // initial + 1 retry
        .create();

    // Cycle 2: everything works, deposit found at block 100
    let _tip_ok = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_blockNumber"})))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc":"2.0","id":1,"result":"0x64"}).to_string())
        .expect(1)
        .create();
    let _block = mock_native_block(
        &mut server,
        "0x665f9a80",
        "0xrecovery",
        EVM_SENDER,
        EVM_ADDR_LOWER,
        "0xde0b6b3a7640000",
    );

    let chain = eth_chain(url);
    let config = test_app_config(vec![chain], 100_000);
    let (handle, task, mut events_rx) = start_detector(config);

    handle
        .cmd_tx
        .send(Command::Watch(Box::new(watch_spec(
            EVM_ADDR, "eip155:1", "ETH",
        ))))
        .await
        .expect("send watch");

    let collected = collect_events(&mut events_rx, Duration::from_secs(5), 5).await;

    // After recovery, we should still get the deposit
    assert!(
        !collected.is_empty(),
        "detector should recover after get_tip failure and find deposits; got {collected:?}"
    );
    let detected = collected
        .iter()
        .find(|e| e.event == DepositStatus::Detected.event_name())
        .expect("expected detected event after recovery");
    assert_eq!(detected.data.tx_id, "0xrecovery");

    handle
        .cmd_tx
        .send(Command::Shutdown)
        .await
        .expect("send shutdown");
    let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
}

// ── Graceful shutdown with in-flight scans ───────────────────────────────

#[tokio::test]
async fn graceful_shutdown_with_inflight_scans() {
    let mut server = Server::new_async().await;
    let url = server.url();

    let _tip = mock_tip(&mut server, "0x64");
    let _block = mock_native_block(
        &mut server,
        "0x665f9a80",
        "0xshutdown",
        EVM_SENDER,
        EVM_ADDR_LOWER,
        "0xde0b6b3a7640000",
    );

    let chain = eth_chain(url);
    let config = test_app_config(vec![chain], 100_000);
    let (handle, task, _events_rx) = start_detector(config);

    handle
        .cmd_tx
        .send(Command::Watch(Box::new(watch_spec(
            EVM_ADDR, "eip155:1", "ETH",
        ))))
        .await
        .expect("send watch");

    // Wait a bit for the first scan to start, then request shutdown
    tokio::time::sleep(Duration::from_millis(1100)).await;
    handle
        .cmd_tx
        .send(Command::Shutdown)
        .await
        .expect("send shutdown");

    // The task should finish within a reasonable time
    let result = tokio::time::timeout(Duration::from_secs(3), task).await;
    assert!(
        result.is_ok(),
        "detector task should shut down cleanly within timeout"
    );
}

// ── confirmed_blocks gating ──────────────────────────────────────────────

#[tokio::test]
async fn confirmed_blocks_gating() {
    let mut server = Server::new_async().await;
    let url = server.url();

    // Use start_block=100 so the first scan hits block 100 regardless of
    // the tip value.  The deposit is at block 100.
    // Cycle 1: tip = 105, scan block 100 → deposit found.
    //          confirmations = 105-100+1 = 6 (< 12) → NO confirmed event.
    let _tip1 = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_blockNumber"})))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc":"2.0","id":1,"result":"0x69"}).to_string())
        .expect(1)
        .create();
    let _block1 = mock_native_block(
        &mut server,
        "0x665f9a80",
        "0xgating",
        EVM_SENDER,
        EVM_ADDR_LOWER,
        "0xde0b6b3a7640000",
    );

    // Cycle 2: tip = 112, empty block 101 (cursor advanced to 101).
    //          confirmations = 112-100+1 = 13 (≥ 12) → confirmed emitted.
    let _tip2 = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_blockNumber"})))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc":"2.0","id":1,"result":"0x70"}).to_string())
        .expect(1)
        .create();
    let _block2 = mock_empty_block(&mut server, "0x665f9a80");

    let mut chain = eth_chain(url);
    chain.start_block = Some(100); // anchor the scan start
    let config = test_app_config(vec![chain], 100_000);
    let (handle, task, mut events_rx) = start_detector(config);

    handle
        .cmd_tx
        .send(Command::Watch(Box::new(watch_spec(
            EVM_ADDR, "eip155:1", "ETH",
        ))))
        .await
        .expect("send watch");

    let collected = collect_events(&mut events_rx, Duration::from_secs(4), 5).await;

    let detected: Vec<_> = collected
        .iter()
        .filter(|e| e.event == DepositStatus::Detected.event_name())
        .collect();
    let confirmed: Vec<_> = collected
        .iter()
        .filter(|e| e.event == DepositStatus::Confirmed.event_name())
        .collect();

    assert_eq!(
        detected.len(),
        1,
        "should have 1 detected event, got {collected:?}"
    );
    assert_eq!(
        confirmed.len(),
        1,
        "should have 1 confirmed event after reaching 12 confirmations; got {collected:?}"
    );
    // At tip=112 with deposit at block 100: 112-100+1 = 13 confirmations
    assert_eq!(confirmed[0].data.confirmations, 13);

    handle
        .cmd_tx
        .send(Command::Shutdown)
        .await
        .expect("send shutdown");
    let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
}

// ── Multi-chain coordination / concurrent scanning ───────────────────────

#[tokio::test]
async fn multichain_coordination_concurrent_scanning() {
    let mut server1 = Server::new_async().await;
    let mut server2 = Server::new_async().await;
    let url1 = server1.url();
    let url2 = server2.url();

    // Chain 1 (eip155:1) — has a 1 ETH deposit
    let _tip1 = mock_tip(&mut server1, "0x64");
    let _block1 = mock_native_block(
        &mut server1,
        "0x665f9a80",
        "0xmulti1",
        EVM_SENDER,
        EVM_ADDR_LOWER,
        "0xde0b6b3a7640000",
    );

    // Chain 2 (eip155:137) — has a 10 ETH deposit
    let _tip2 = mock_tip(&mut server2, "0x64");
    let _block2 = mock_native_block(
        &mut server2,
        "0x665f9a80",
        "0xmulti2",
        EVM_SENDER,
        EVM_ADDR_LOWER,
        "0x8ac7230489e80000", // 10 ETH
    );

    let mut chain1 = eth_chain(url1);
    chain1.caip2 = "eip155:1".to_string();
    let mut chain2 = eth_chain(url2);
    chain2.caip2 = "eip155:137".to_string();

    let config = test_app_config(vec![chain1, chain2], 100_000);
    let (handle, task, mut events_rx) = start_detector(config);

    // Watch on chain 1
    handle
        .cmd_tx
        .send(Command::Watch(Box::new(watch_spec(
            EVM_ADDR, "eip155:1", "ETH",
        ))))
        .await
        .expect("send watch1");
    // Watch on chain 2
    handle
        .cmd_tx
        .send(Command::Watch(Box::new(watch_spec(
            EVM_ADDR,
            "eip155:137",
            "ETH",
        ))))
        .await
        .expect("send watch2");

    let collected = collect_events(&mut events_rx, Duration::from_secs(4), 5).await;

    let caip2s: Vec<&str> = collected
        .iter()
        .filter(|e| e.event == DepositStatus::Detected.event_name())
        .map(|e| e.data.caip2.as_str())
        .collect();

    assert!(
        caip2s.contains(&"eip155:1"),
        "should contain deposit from eip155:1; got {caip2s:?}"
    );
    assert!(
        caip2s.contains(&"eip155:137"),
        "should contain deposit from eip155:137; got {caip2s:?}"
    );

    // Verify amounts are different per chain
    let amounts: Vec<&str> = collected
        .iter()
        .filter(|e| e.event == DepositStatus::Detected.event_name())
        .map(|e| e.data.amount.as_str())
        .collect();
    assert!(
        amounts.contains(&"1000000000000000000"),
        "should contain 1 ETH deposit from chain 1"
    );
    assert!(
        amounts.contains(&"10000000000000000000"),
        "should contain 10 ETH deposit from chain 2"
    );

    handle
        .cmd_tx
        .send(Command::Shutdown)
        .await
        .expect("send shutdown");
    let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
}

// ── Dedup window expiry re-emits event ───────────────────────────────────
// Direct unit test on remember_event_key — faster and more precise
// than orchestrating a multi-cycle detector loop scan.

#[test]
fn dedup_window_expiry_reemits_event() {
    let dedup_window_size: usize = 3;
    let mut keys = hashbrown::HashSet::new();
    let mut order = VecDeque::new();

    // Generate 4 distinct event keys
    let data = sample_data();
    let e1 = DepositEvent::detected(data.clone()).expect("valid event");
    let key1 = deposit_event_key(&e1);

    let data2 = DepositData {
        tx_id: "0xtx2".into(),
        ..data.clone()
    };
    let e2 = DepositEvent::detected(data2).expect("valid event");
    let key2 = deposit_event_key(&e2);

    let data3 = DepositData {
        tx_id: "0xtx3".into(),
        ..data.clone()
    };
    let e3 = DepositEvent::detected(data3).expect("valid event");
    let key3 = deposit_event_key(&e3);

    let data4 = DepositData {
        tx_id: "0xtx4".into(),
        ..data.clone()
    };
    let e4 = DepositEvent::detected(data4).expect("valid event");
    let key4 = deposit_event_key(&e4);

    // Insert key1 → accepted
    assert!(remember_event_key(
        &mut keys,
        &mut order,
        dedup_window_size,
        key1.clone()
    ));
    // Insert key2 → accepted
    assert!(remember_event_key(
        &mut keys,
        &mut order,
        dedup_window_size,
        key2.clone()
    ));
    // Insert key3 → accepted (window=3 now full: key1, key2, key3)
    assert!(remember_event_key(
        &mut keys,
        &mut order,
        dedup_window_size,
        key3.clone()
    ));

    // Insert key4 → accepted, but key1 should be evicted (window overflow)
    assert!(remember_event_key(
        &mut keys,
        &mut order,
        dedup_window_size,
        key4.clone()
    ));

    // key1 should be evicted from the dedup window
    assert!(
        !keys.contains(&key1),
        "key1 should be evicted after window overflow"
    );

    // Re-insert key1 → should be accepted again (re-emitted after expiry)
    assert!(
        remember_event_key(&mut keys, &mut order, dedup_window_size, key1.clone()),
        "key1 should be re-emitted after dedup window expiry"
    );
}

// ── Address normalization — watch uppercase, deposit lowercase matched ───

#[tokio::test]
async fn watch_uppercase_deposit_lowercase_matched() {
    let mut server = Server::new_async().await;
    let url = server.url();

    // Deposit goes to LOWERCASE address
    let _tip = mock_tip(&mut server, "0x64");
    let _block = mock_native_block(
        &mut server,
        "0x665f9a80",
        "0xcase",
        EVM_SENDER,
        EVM_ADDR_LOWER, // lowercase in block data
        "0xde0b6b3a7640000",
    );

    let chain = eth_chain(url);
    let config = test_app_config(vec![chain], 100_000);
    let (handle, task, mut events_rx) = start_detector(config);

    // Watch using UPPERCASE address — detector should normalize it to lowercase
    handle
        .cmd_tx
        .send(Command::Watch(Box::new(watch_spec(
            EVM_ADDR, // "0xAbCdEf..."
            "eip155:1", "ETH",
        ))))
        .await
        .expect("send watch");

    let collected = collect_events(&mut events_rx, Duration::from_secs(3), 5).await;

    assert!(
        !collected.is_empty(),
        "should detect deposit even when watched with uppercase address; got {collected:?}"
    );

    let detected = collected
        .iter()
        .find(|e| e.event == DepositStatus::Detected.event_name())
        .expect("expected detected event");
    // The deposited-to address is lowercase; the watched address was uppercase.
    // The detector normalizes before comparison, so they match.
    assert_eq!(detected.data.address, EVM_ADDR_LOWER);

    handle
        .cmd_tx
        .send(Command::Shutdown)
        .await
        .expect("send shutdown");
    let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
}
