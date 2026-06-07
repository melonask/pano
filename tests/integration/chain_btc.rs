use super::common::{BTC_ADDR, btc_chain, targets};
use mockito::{Matcher, Server};
use pano::chain::ChainScanner;
use pano::chain::btc::{BtcScanner, btc_to_sats, first_input_address};
use pano::config::{AssetConfig, ChainConfig, RpcOptions};
use serde_json::json;

fn rpc_mock(server: &mut Server, method: &str, result: serde_json::Value) -> mockito::Mock {
    server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": method})))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc":"2.0","id":1,"result":result}).to_string())
        .create()
}

#[tokio::test]
async fn get_tip_returns_block_count() {
    let mut server = Server::new_async().await;
    let mock = rpc_mock(&mut server, "getblockcount", json!(800000));
    let scanner = BtcScanner::new(btc_chain(server.url())).unwrap();

    assert_eq!(scanner.get_tip().await.unwrap(), 800000);
    mock.assert();
}

#[tokio::test]
async fn get_tip_errors_on_non_u64_result() {
    let mut server = Server::new_async().await;
    let _mock = rpc_mock(&mut server, "getblockcount", json!("800000"));
    let scanner = BtcScanner::new(btc_chain(server.url())).unwrap();

    let err = scanner.get_tip().await.unwrap_err().to_string();
    assert!(err.contains("non-u64 result"), "{err}");
}

#[tokio::test]
async fn scan_detects_basic_btc_deposit() {
    let mut server = Server::new_async().await;
    let hash = rpc_mock(&mut server, "getblockhash", json!("abc123"));
    let block = rpc_mock(
        &mut server,
        "getblock",
        json!({
            "time": 1717459200i64,
            "tx": [{
                "txid": "btctx1",
                "vin": [{"prevout":{"scriptPubKey":{"address":"1SenderAddr1111111111111111111"}}}],
                "vout": [
                    {"n":0,"value":0.5,"scriptPubKey":{"address":BTC_ADDR}},
                    {"n":1,"value":1.0,"scriptPubKey":{"address":"1Unwatched11111111111111111111"}},
                    {"n":2,"value":0.0,"scriptPubKey":{"address":BTC_ADDR}}
                ]
            }]
        }),
    );
    let scanner = BtcScanner::new(btc_chain(server.url())).unwrap();

    let events = scanner
        .scan(100, 100, &targets(BTC_ADDR, "BTC"))
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data.amount, "50000000");
    assert_eq!(events[0].data.address, BTC_ADDR);
    assert_eq!(events[0].data.tx_id, "btctx1");
    assert_eq!(events[0].data.block_number, 100);
    assert_eq!(events[0].data.log_index, 0);
    assert_eq!(events[0].data.timestamp, "2024-06-04T00:00:00Z");
    hash.assert();
    block.assert();
}

#[tokio::test]
async fn scan_multiple_outputs_same_address_have_distinct_log_indexes() {
    let mut server = Server::new_async().await;
    let _hash = rpc_mock(&mut server, "getblockhash", json!("abc123"));
    let _block = rpc_mock(
        &mut server,
        "getblock",
        json!({"time":1717459200i64,"tx":[{"txid":"btctx2","vout":[
            {"n":0,"value":"0.00000001","scriptPubKey":{"addresses":[BTC_ADDR]}},
            {"n":3,"value":"0.00000002","scriptPubKey":{"address":BTC_ADDR}}
        ]}]}),
    );
    let scanner = BtcScanner::new(btc_chain(server.url())).unwrap();

    let events = scanner
        .scan(100, 100, &targets(BTC_ADDR, "BTC"))
        .await
        .unwrap();

    assert_eq!(events.len(), 2);
    let indexes: Vec<u64> = events.iter().map(|event| event.data.log_index).collect();
    assert_eq!(indexes, vec![0, 3]);
}

#[tokio::test]
async fn scan_short_circuits_empty_targets_and_invalid_range() {
    let server = Server::new_async().await;
    let scanner = BtcScanner::new(btc_chain(server.url())).unwrap();

    assert!(
        scanner
            .scan(2, 1, &targets(BTC_ADDR, "BTC"))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        scanner
            .scan(1, 1, &Default::default())
            .await
            .unwrap()
            .is_empty()
    );
}

#[test]
fn btc_to_sats_edge_cases() {
    assert_eq!(btc_to_sats("0.00000001").unwrap(), "1");
    assert_eq!(btc_to_sats("21000000").unwrap(), "2100000000000000");
    assert!(
        btc_to_sats("21000000.00000001")
            .unwrap_err()
            .to_string()
            .contains("exceeds maximum supply")
    );
    assert_eq!(btc_to_sats("0").unwrap(), "0");
    assert!(
        btc_to_sats("-0.5")
            .unwrap_err()
            .to_string()
            .contains("negative BTC amount")
    );
    assert_eq!(btc_to_sats("1e-8").unwrap(), "1");
    assert!(
        btc_to_sats("abc")
            .unwrap_err()
            .to_string()
            .contains("invalid BTC amount")
    );
    assert_eq!(btc_to_sats("0.000000004").unwrap(), "0");
    assert_eq!(btc_to_sats("0.000000005").unwrap(), "0");
    assert_eq!(btc_to_sats("0.000000006").unwrap(), "1");
}

#[test]
fn first_input_address_variants() {
    assert_eq!(
        first_input_address(&json!({"vin":[{"coinbase":"abcd"}]})),
        ""
    );
    assert_eq!(
        first_input_address(&json!({"vin":[{"prevout":{"scriptPubKey":{"address":"sender1"}}}]})),
        "sender1"
    );
    assert_eq!(
        first_input_address(
            &json!({"vin":[{"prevout":{"scriptPubKey":{"addresses":["sender2"]}}}]})
        ),
        "sender2"
    );
    assert_eq!(first_input_address(&json!({"vin":[]})), "");
}

// ── Edge-case tests ───────────────────────────────────────────────────────

#[tokio::test]
async fn getblockhash_null_result_errors() {
    // RPC returns JSON null for getblockhash — the scanner must error out
    // cleanly instead of panicking or producing garbage.
    let mut server = Server::new_async().await;
    let _hash = rpc_mock(&mut server, "getblockhash", json!(null));
    let scanner = BtcScanner::new(btc_chain(server.url())).unwrap();

    let err = scanner
        .scan(100, 100, &targets(BTC_ADDR, "BTC"))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("non-string result"),
        "expected non-string-result error, got: {err}"
    );
}

#[tokio::test]
async fn getblock_null_result_no_events() {
    // RPC returns JSON null for getblock. The scanner must handle this
    // gracefully — no crash, just zero deposit events.
    let mut server = Server::new_async().await;
    let _hash = rpc_mock(&mut server, "getblockhash", json!("abc123"));
    let _block = rpc_mock(&mut server, "getblock", json!(null));
    let scanner = BtcScanner::new(btc_chain(server.url())).unwrap();

    let events = scanner
        .scan(100, 100, &targets(BTC_ADDR, "BTC"))
        .await
        .unwrap();
    assert!(events.is_empty(), "expected no events for null getblock");
}

#[tokio::test]
async fn transaction_with_no_vout_skipped() {
    // A transaction without a vout field (e.g. malformed or all-inputs)
    // must be skipped, not crash.
    let mut server = Server::new_async().await;
    let _hash = rpc_mock(&mut server, "getblockhash", json!("hash1"));
    let _block = rpc_mock(
        &mut server,
        "getblock",
        json!({
            "time": 1717459200i64,
            "tx": [
                {"txid": "no_vout_tx", "vin": [{"prevout":{"scriptPubKey":{"address":"sender1"}}}]}
            ]
        }),
    );
    let scanner = BtcScanner::new(btc_chain(server.url())).unwrap();

    let events = scanner
        .scan(100, 100, &targets(BTC_ADDR, "BTC"))
        .await
        .unwrap();
    assert!(
        events.is_empty(),
        "expected no events from tx without vout, got {events:?}"
    );
}

#[tokio::test]
async fn coinbase_tx_watched_address_detected() {
    // A coinbase (mining reward) transaction sending to a watched address
    // must still be detected as a deposit. The sender is empty for coinbase.
    let mut server = Server::new_async().await;
    let _hash = rpc_mock(&mut server, "getblockhash", json!("cb_hash"));
    let _block = rpc_mock(
        &mut server,
        "getblock",
        json!({
            "time": 1717459200i64,
            "tx": [{
                "txid": "coinbase_tx_1",
                "vin": [{"coinbase": "03d92b0f"}],
                "vout": [
                    {"n": 0, "value": 6.25, "scriptPubKey": {"address": BTC_ADDR}},
                    {"n": 1, "value": 0.1, "scriptPubKey": {"address": "1Miner1111111111111111111111111"}}
                ]
            }]
        }),
    );
    let scanner = BtcScanner::new(btc_chain(server.url())).unwrap();

    let events = scanner
        .scan(100, 100, &targets(BTC_ADDR, "BTC"))
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data.tx_id, "coinbase_tx_1");
    assert_eq!(events[0].data.amount, "625000000"); // 6.25 BTC
    assert_eq!(events[0].data.address, BTC_ADDR);
    assert_eq!(events[0].data.sender, ""); // coinbase has no sender
    assert_eq!(events[0].data.log_index, 0);
}

#[tokio::test]
async fn multiple_txs_in_single_block() {
    // A block with multiple transactions: verify all matching deposits
    // are found, not just the first one.
    let mut server = Server::new_async().await;
    let _hash = rpc_mock(&mut server, "getblockhash", json!("multi_hash"));
    let _block = rpc_mock(
        &mut server,
        "getblock",
        json!({
            "time": 1717459200i64,
            "tx": [
                {
                    "txid": "tx_no_match",
                    "vin": [{"prevout": {"scriptPubKey": {"address": "1SenderA11111111111111111111"}}}],
                    "vout": [
                        {"n": 0, "value": 0.123, "scriptPubKey": {"address": "1Random1111111111111111111111"}}
                    ]
                },
                {
                    "txid": "tx_match_1",
                    "vin": [{"prevout": {"scriptPubKey": {"address": "1SenderB11111111111111111111"}}}],
                    "vout": [
                        {"n": 0, "value": 0.5, "scriptPubKey": {"address": BTC_ADDR}},
                        {"n": 1, "value": 2.0, "scriptPubKey": {"address": "1Other11111111111111111111111"}}
                    ]
                },
                {
                    "txid": "tx_match_2",
                    "vin": [{"prevout": {"scriptPubKey": {"address": "1SenderC11111111111111111111"}}}],
                    "vout": [
                        {"n": 0, "value": 0.003, "scriptPubKey": {"address": BTC_ADDR}}
                    ]
                },
                {
                    "txid": "tx_no_match_2",
                    "vin": [{"prevout": {"scriptPubKey": {"address": "1SenderD11111111111111111111"}}}],
                    "vout": [
                        {"n": 0, "value": 0.0, "scriptPubKey": {"address": BTC_ADDR}}
                    ]
                }
            ]
        }),
    );
    let scanner = BtcScanner::new(btc_chain(server.url())).unwrap();

    let events = scanner
        .scan(100, 100, &targets(BTC_ADDR, "BTC"))
        .await
        .unwrap();

    assert_eq!(events.len(), 2, "expected 2 deposits, got {events:?}");
    // tx_match_1 (n=0, 0.5 BTC) should appear first
    assert_eq!(events[0].data.tx_id, "tx_match_1");
    assert_eq!(events[0].data.amount, "50000000"); // 0.5 BTC
    assert_eq!(events[0].data.log_index, 0);
    // tx_match_2 (n=0, 0.003 BTC)
    assert_eq!(events[1].data.tx_id, "tx_match_2");
    assert_eq!(events[1].data.amount, "300000"); // 0.003 BTC
    assert_eq!(events[1].data.log_index, 0);
}

#[tokio::test]
async fn rpc_error_mid_scan_returns_error() {
    // When scanning a range of blocks, an RPC failure on an intermediate
    // block should propagate as an error (the current implementation
    // fails the whole scan rather than returning partial results).
    //
    // We use Matcher::Json (exact body match including id/params) so
    // each mock targets exactly one call. The RPC id counter starts at 1
    // for a fresh RpcClient.
    let mut server = Server::new_async().await;

    // Call 1: getblockhash[100] → success, id=1
    server
        .mock("POST", "/")
        .match_body(Matcher::Json(json!({
            "jsonrpc":"2.0","id":1,"method":"getblockhash","params":[100]
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc":"2.0","id":1,"result":"hash100"}).to_string())
        .create();

    // Call 2: getblock["hash100", 2] → success, id=2
    server
        .mock("POST", "/")
        .match_body(Matcher::Json(json!({
            "jsonrpc":"2.0","id":2,"method":"getblock","params":["hash100",2]
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({"jsonrpc":"2.0","id":2,"result":{
                "time": 1717459200i64,
                "tx": [{
                    "txid": "tx100",
                    "vin": [{"prevout":{"scriptPubKey":{"address":"sender1"}}}],
                    "vout": [{"n":0,"value":0.5,"scriptPubKey":{"address":BTC_ADDR}}]
                }]
            }})
            .to_string(),
        )
        .create();

    // Call 3: getblockhash[101] → JSON-RPC error, id=3
    // With max_retries=0 (see custom chain below) there is no retry,
    // so the error propagates immediately.
    server
        .mock("POST", "/")
        .match_body(Matcher::Json(json!({
            "jsonrpc":"2.0","id":3,"method":"getblockhash","params":[101]
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({"jsonrpc":"2.0","id":3,"error":{"code":-32603,"message":"Internal error"}})
                .to_string(),
        )
        .create();

    // Build a chain with max_retries=0 so the JSON-RPC error fails
    // immediately without retry.
    let chain = ChainConfig {
        caip2: "bip122:000000000019d6689c085ae165831e93".to_string(),
        start_block: Some(0),
        end_block: Some(0),
        confirmed_blocks: 6,
        rpc: vec![server.url()],
        rpc_options: Some(RpcOptions {
            max_concurrent: 1,
            delay_ms: 0,
            batch_size: 25,
            evm_log_address_batching: true,
            scan_lookback_blocks: 0,
            scan_interval_secs: 1,
            scan_timeout_secs: 5,
            max_native_scan_per_cycle: 10,
            request_timeout_secs: 5,
            max_retries: 0,
            retry_base_ms: 1,
            solana_max_supported_transaction_version: 0,
            solana_scan_mode: Default::default(),
        }),
        assets: vec![AssetConfig {
            symbol: "BTC".to_string(),
            contract: None,
            token_program: None,
            decimals: 8,
            min_amount: None,
        }],
    };
    let scanner = BtcScanner::new(chain).unwrap();

    let result = scanner.scan(100, 101, &targets(BTC_ADDR, "BTC")).await;

    assert!(
        result.is_err(),
        "expected error for RPC failure mid-scan, got success: {result:?}"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("Internal error") || err.contains("JSON-RPC error"),
        "expected error about RPC failure, got: {err}"
    );
}

#[test]
fn btc_to_sats_more_than_8_decimal_places() {
    // Strengthen precision edge-case coverage beyond the existing
    // btc_to_sats_edge_cases test.  rust_decimal uses round-half-to-even
    // (banker's rounding) by default — values at exactly 0.5 round to
    // the nearest even integer.
    // Exactly 8 decimal places (the standard)
    assert_eq!(btc_to_sats("0.00000001").unwrap(), "1");
    assert_eq!(btc_to_sats("1.00000001").unwrap(), "100000001");
    // 9 decimal places: round down (< 0.5)
    assert_eq!(btc_to_sats("0.000000001").unwrap(), "0");
    assert_eq!(btc_to_sats("0.000000004").unwrap(), "0");
    // 9 decimal places: 0.5 → round half to even: 0 is even → 0
    assert_eq!(btc_to_sats("0.000000005").unwrap(), "0");
    // 9 decimal places: round up (> 0.5)
    assert_eq!(btc_to_sats("0.000000009").unwrap(), "1");
    // 9 decimal places with whole-number part — 100000000.5 → 100000000 is even → 100000000
    assert_eq!(btc_to_sats("1.000000005").unwrap(), "100000000");
    assert_eq!(btc_to_sats("1.000000004").unwrap(), "100000000");
    // 10 decimal places
    assert_eq!(btc_to_sats("0.0000000001").unwrap(), "0");
    assert_eq!(btc_to_sats("0.0000000005").unwrap(), "0");
    assert_eq!(btc_to_sats("0.0000000009").unwrap(), "0");
    // Many decimal places — still parses correctly
    assert_eq!(btc_to_sats("0.00000000123456789").unwrap(), "0");
    assert_eq!(btc_to_sats("1.00000000999999999").unwrap(), "100000001");
}

#[tokio::test]
async fn block_with_zero_transactions_no_events() {
    // An empty block (tx: []) must produce zero events without errors.
    let mut server = Server::new_async().await;
    let _hash = rpc_mock(&mut server, "getblockhash", json!("empty_hash"));
    let _block = rpc_mock(
        &mut server,
        "getblock",
        json!({
            "time": 1717459200i64,
            "tx": []
        }),
    );
    let scanner = BtcScanner::new(btc_chain(server.url())).unwrap();

    let events = scanner
        .scan(100, 100, &targets(BTC_ADDR, "BTC"))
        .await
        .unwrap();
    assert!(
        events.is_empty(),
        "expected no events for empty block, got {events:?}"
    );
}
