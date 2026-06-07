mod common;

use common::{
    ERC20_CONTRACT, EVM_ADDR_LOWER, EVM_SENDER, erc20_targets, evm_chain, targets,
    topic_for_address,
};
use mockito::{Matcher, Server};
use pano::chain::ChainScanner;
use pano::chain::evm::{EvmScanner, extract_topic_address, parse_hex_u64, parse_hex_uint};
use pano::config::{AssetConfig, ChainConfig, RpcOptions};
use pano::model::{ResolvedAsset, TargetMap};
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
async fn get_tip_parses_eth_block_number() {
    let mut server = Server::new_async().await;
    let mock = rpc_mock(&mut server, "eth_blockNumber", json!("0x12a05f2"));
    let scanner = EvmScanner::new(evm_chain(server.url())).unwrap();

    assert_eq!(scanner.get_tip().await.unwrap(), 19_531_250);
    mock.assert();
}

#[tokio::test]
async fn get_tip_errors_on_non_string_result() {
    let mut server = Server::new_async().await;
    let _mock = rpc_mock(&mut server, "eth_blockNumber", json!(123));
    let scanner = EvmScanner::new(evm_chain(server.url())).unwrap();

    let err = scanner.get_tip().await.unwrap_err().to_string();
    assert!(err.contains("non-string"), "{err}");
}

#[test]
fn parse_hex_u64_cases() {
    assert_eq!(parse_hex_u64("0x0").unwrap(), 0);
    assert_eq!(parse_hex_u64("0xff").unwrap(), 255);
    assert_eq!(parse_hex_u64("0XFF").unwrap(), 255);
    assert_eq!(parse_hex_u64("ff").unwrap(), 255);
    assert!(parse_hex_u64("0x0x123").is_err());
    assert!(
        parse_hex_u64("")
            .unwrap_err()
            .to_string()
            .contains("empty hex string")
    );
    assert!(
        parse_hex_u64("0x")
            .unwrap_err()
            .to_string()
            .contains("empty hex string")
    );
    assert!(
        parse_hex_u64("0xGGGG")
            .unwrap_err()
            .to_string()
            .contains("invalid hex u64")
    );
    assert_eq!(parse_hex_u64("0xffffffffffffffff").unwrap(), u64::MAX);
    assert!(parse_hex_u64("0x10000000000000000").is_err());
}

#[test]
fn parse_hex_uint_cases() {
    assert_eq!(parse_hex_uint("0x0").as_deref(), Some("0"));
    assert_eq!(parse_hex_uint("0x").as_deref(), Some("0"));
    assert_eq!(parse_hex_uint("0").as_deref(), Some("0"));
    assert_eq!(parse_hex_uint("0x0x123"), None);
    assert_eq!(
        parse_hex_uint("0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
            .unwrap(),
        "115792089237316195423570985008687907853269984665640564039457584007913129639935"
    );
    assert_eq!(parse_hex_uint("0xinvalid"), None);
}

#[test]
fn extract_topic_address_cases() {
    let topic = topic_for_address("0xAbCdEf1234567890AbCdEf1234567890AbCdEf12");
    assert_eq!(
        extract_topic_address(&json!(topic)).as_deref(),
        Some(EVM_ADDR_LOWER)
    );
    assert_eq!(extract_topic_address(&json!("0x1234")), None);
    assert_eq!(
        extract_topic_address(&json!(
            "000000000000000000000000abcdef1234567890abcdef1234567890abcdef12"
        )),
        None
    );
    assert_eq!(
        extract_topic_address(&json!(
            "0xffffffffffffffffffffffffabcdef1234567890abcdef1234567890abcdef12"
        )),
        None
    );
    assert_eq!(
        extract_topic_address(&json!(
            "0x000000000000000000000000zzzzzz1234567890abcdef1234567890abcdef12"
        )),
        None
    );
}

#[tokio::test]
async fn scan_detects_erc20_transfer_to_watched_address() {
    let mut server = Server::new_async().await;
    let logs = rpc_mock(
        &mut server,
        "eth_getLogs",
        json!([{
            "address": ERC20_CONTRACT.to_uppercase(),
            "blockNumber": "0x64",
            "transactionHash": "0xerc20tx",
            "logIndex": "0x2",
            "removed": false,
            "topics": [
                "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                topic_for_address(EVM_SENDER),
                topic_for_address(EVM_ADDR_LOWER)
            ],
            "data": "0x0f4240"
        }]),
    );
    let timestamp = rpc_mock(
        &mut server,
        "eth_getBlockByNumber",
        json!({"timestamp":"0x665f9a80"}),
    );
    let scanner = EvmScanner::new(evm_chain(server.url())).unwrap();

    let events = scanner
        .scan(100, 100, &erc20_targets(EVM_ADDR_LOWER))
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data.address, EVM_ADDR_LOWER);
    assert_eq!(events[0].data.sender, EVM_SENDER);
    assert_eq!(events[0].data.amount, "1000000");
    assert_eq!(events[0].data.symbol, "USDC");
    assert_eq!(events[0].data.log_index, 2);
    assert_eq!(events[0].data.block_number, 100);
    logs.assert();
    timestamp.assert();
}

#[tokio::test]
async fn scan_skips_removed_short_topics_and_zero_amount_logs() {
    let mut server = Server::new_async().await;
    let _logs = rpc_mock(
        &mut server,
        "eth_getLogs",
        json!([
            {"address":ERC20_CONTRACT,"blockNumber":"0x64","transactionHash":"0x1","logIndex":"0x0","removed":true,"topics":["x", topic_for_address(EVM_SENDER), topic_for_address(EVM_ADDR_LOWER)],"data":"0x1"},
            {"address":ERC20_CONTRACT,"blockNumber":"0x64","transactionHash":"0x2","logIndex":"0x0","topics":["x", topic_for_address(EVM_SENDER)],"data":"0x1"},
            {"address":ERC20_CONTRACT,"blockNumber":"0x64","transactionHash":"0x3","logIndex":"0x0","topics":["x", topic_for_address(EVM_SENDER), topic_for_address(EVM_ADDR_LOWER)],"data":"0x0"}
        ]),
    );
    let _timestamp = rpc_mock(
        &mut server,
        "eth_getBlockByNumber",
        json!({"timestamp":"0x665f9a80"}),
    );
    let scanner = EvmScanner::new(evm_chain(server.url())).unwrap();

    assert!(
        scanner
            .scan(100, 100, &erc20_targets(EVM_ADDR_LOWER))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn scan_detects_native_eth_deposit_and_skips_zero_or_null_to() {
    let mut server = Server::new_async().await;
    let block = rpc_mock(
        &mut server,
        "eth_getBlockByNumber",
        json!({
            "timestamp":"0x665f9a80",
            "transactions":[
                {"hash":"0xnative","from":EVM_SENDER,"to":EVM_ADDR_LOWER,"value":"0xde0b6b3a7640000"},
                {"hash":"0xzero","from":EVM_SENDER,"to":EVM_ADDR_LOWER,"value":"0x0"},
                {"hash":"0xcontract","from":EVM_SENDER,"to":null,"value":"0x1"}
            ]
        }),
    );
    let scanner = EvmScanner::new(evm_chain(server.url())).unwrap();

    let events = scanner
        .scan(100, 100, &targets(EVM_ADDR_LOWER, "ETH"))
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data.tx_id, "0xnative");
    assert_eq!(events[0].data.amount, "1000000000000000000");
    assert_eq!(events[0].data.address, EVM_ADDR_LOWER);
    block.assert();
}

#[tokio::test]
async fn scan_empty_targets_or_invalid_range_short_circuits() {
    let server = Server::new_async().await;
    let scanner = EvmScanner::new(evm_chain(server.url())).unwrap();

    assert!(
        scanner
            .scan(2, 1, &targets(EVM_ADDR_LOWER, "ETH"))
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

// ── Edge-case tests ───────────────────────────────────────────────────────

#[tokio::test]
async fn eth_getlogs_null_returns_no_events() {
    // Some RPC providers return JSON null instead of an empty array for
    // eth_getLogs when there are no matching logs. The scanner must treat
    // this gracefully and produce zero events (not crash).
    let mut server = Server::new_async().await;
    let logs = rpc_mock(&mut server, "eth_getLogs", json!(null));
    // No native targets, so no eth_getBlockByNumber calls are expected.
    let scanner = EvmScanner::new(evm_chain(server.url())).unwrap();

    let events = scanner
        .scan(100, 100, &erc20_targets(EVM_ADDR_LOWER))
        .await
        .unwrap();

    assert!(
        events.is_empty(),
        "expected no events for null eth_getLogs, got {events:?}"
    );
    logs.assert();
}

#[tokio::test]
async fn custom_runtime_erc20_missing_decimals_errors() {
    // Runtime TargetMap assets whose contract is NOT in the static chain
    // config MUST provide decimals. Defaulting to 0 would cause downstream
    // formatting to produce wildly incorrect human-readable amounts.
    let mut server = Server::new_async().await;
    let _logs = rpc_mock(&mut server, "eth_getLogs", json!([]));
    let scanner = EvmScanner::new(evm_chain(server.url())).unwrap();

    let mut targets = TargetMap::new();
    targets.insert(
        EVM_ADDR_LOWER.to_string(),
        vec![ResolvedAsset {
            symbol: "USDT".to_string(),
            contract: Some("0x3333333333333333333333333333333333333333".to_string()),
            token_program: None,
            decimals: None, // missing — must trigger error
        }],
    );

    let err = scanner
        .scan(100, 100, &targets)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("missing decimals"),
        "expected 'missing decimals' error for custom runtime ERC-20 without decimals, got: {err}"
    );
}

#[tokio::test]
async fn malicious_timestamp_exceeds_i64_max_errors_native_scan() {
    // A faulty or malicious RPC that returns a block timestamp larger than
    // i64::MAX must produce a hard error instead of a silently-wrong
    // negative timestamp from the unchecked `as i64` cast.
    let mut server = Server::new_async().await;
    // 0x8000000000000000 == 9_223_372_036_854_775_808 > i64::MAX (9_223_372_036_854_775_807)
    let _block = rpc_mock(
        &mut server,
        "eth_getBlockByNumber",
        json!({
            "timestamp": "0x8000000000000000",
            "transactions": [
                {"hash":"0xbadtime","from":EVM_SENDER,"to":EVM_ADDR_LOWER,"value":"0xde0b6b3a7640000"}
            ]
        }),
    );
    let scanner = EvmScanner::new(evm_chain(server.url())).unwrap();

    let err = scanner
        .scan(100, 100, &targets(EVM_ADDR_LOWER, "ETH"))
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("exceeds i64 range"),
        "expected 'exceeds i64 range' error for malicious timestamp, got: {err}"
    );
}

#[tokio::test]
async fn multi_block_native_scan_detects_deposits_across_blocks() {
    // Scan a two-block range and verify that a deposit in each block is
    // detected, confirming the batch loop correctly iterates across block
    // boundaries.
    let mut server = Server::new_async().await;
    // The same mock responds to both blocks; use expect_at_least so the
    // Drop-guard assertion tolerates 2 calls instead of the default 1.
    let block = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_getBlockByNumber"})))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({"jsonrpc":"2.0","id":1,"result":{
                "timestamp": "0x665f9a80",
                "transactions": [
                    {"hash":"0xblock100","from":EVM_SENDER,"to":EVM_ADDR_LOWER,"value":"0xde0b6b3a7640000"},
                ]
            }})
            .to_string(),
        )
        .expect_at_least(2)
        .create();
    let scanner = EvmScanner::new(evm_chain(server.url())).unwrap();

    // batch_size=10 so both blocks 100 and 101 fit in one batch window.
    let events = scanner
        .scan(100, 101, &targets(EVM_ADDR_LOWER, "ETH"))
        .await
        .unwrap();

    assert_eq!(
        events.len(),
        2,
        "expected 2 deposits across blocks 100-101, got {events:?}"
    );
    assert_eq!(events[0].data.block_number, 100);
    assert_eq!(events[0].data.tx_id, "0xblock100");
    assert_eq!(events[1].data.block_number, 101);
    assert_eq!(events[1].data.tx_id, "0xblock100");
    block.assert();
}

// ── Edge-case tests (continued) ───────────────────────────────────────────

#[tokio::test]
async fn erc20_transfer_with_extra_topics_still_detected() {
    // Some tokens emit non-standard Transfer events with additional topics
    // beyond the standard 3 (indexed params). The scanner must still detect
    // the transfer using topics[1] (sender) and topics[2] (recipient).
    let mut server = Server::new_async().await;
    let logs = rpc_mock(
        &mut server,
        "eth_getLogs",
        json!([{
            "address": ERC20_CONTRACT.to_uppercase(),
            "blockNumber": "0x64",
            "transactionHash": "0xextratopics",
            "logIndex": "0x2",
            "removed": false,
            "topics": [
                "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                topic_for_address(EVM_SENDER),
                topic_for_address(EVM_ADDR_LOWER),
                "0x0000000000000000000000000000000000000000000000000000000000000001"
            ],
            "data": "0x0f4240"
        }]),
    );
    let timestamp = rpc_mock(
        &mut server,
        "eth_getBlockByNumber",
        json!({"timestamp":"0x665f9a80"}),
    );
    let scanner = EvmScanner::new(evm_chain(server.url())).unwrap();

    let events = scanner
        .scan(100, 100, &erc20_targets(EVM_ADDR_LOWER))
        .await
        .unwrap();

    assert_eq!(
        events.len(),
        1,
        "expected 1 event for 4-topic log, got {events:?}"
    );
    assert_eq!(events[0].data.address, EVM_ADDR_LOWER);
    assert_eq!(events[0].data.sender, EVM_SENDER);
    assert_eq!(events[0].data.amount, "1000000");
    assert_eq!(events[0].data.tx_id, "0xextratopics");
    logs.assert();
    timestamp.assert();
}

#[tokio::test]
async fn erc20_log_missing_data_field_skipped() {
    // A malformed log entry without a `data` field must be silently
    // skipped (data defaults to "0x0" → amount "0" → filtered) rather
    // than crashing the scanner.
    let mut server = Server::new_async().await;
    let _logs = rpc_mock(
        &mut server,
        "eth_getLogs",
        json!([{
            "address": ERC20_CONTRACT,
            "blockNumber": "0x64",
            "transactionHash": "0xnodata",
            "logIndex": "0x1",
            "removed": false,
            "topics": [
                "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                topic_for_address(EVM_SENDER),
                topic_for_address(EVM_ADDR_LOWER)
            ]
        }]),
    );
    // A log was returned, so a block timestamp fetch is triggered.
    let _timestamp = rpc_mock(
        &mut server,
        "eth_getBlockByNumber",
        json!({"timestamp":"0x665f9a80"}),
    );
    let scanner = EvmScanner::new(evm_chain(server.url())).unwrap();

    let events = scanner
        .scan(100, 100, &erc20_targets(EVM_ADDR_LOWER))
        .await
        .unwrap();

    assert!(
        events.is_empty(),
        "expected no events for log without data field, got {events:?}"
    );
}

#[tokio::test]
async fn native_eth_transfer_to_empty_string_skipped() {
    // Different RPC providers serialize missing `to` differently: some
    // use `null`, others use `""`. Both must be treated identically.
    let mut server = Server::new_async().await;
    let _block = rpc_mock(
        &mut server,
        "eth_getBlockByNumber",
        json!({
            "timestamp": "0x665f9a80",
            "transactions": [
                {"hash":"0xemptystr","from":EVM_SENDER,"to":"","value":"0xde0b6b3a7640000"}
            ]
        }),
    );
    let scanner = EvmScanner::new(evm_chain(server.url())).unwrap();

    let events = scanner
        .scan(100, 100, &targets(EVM_ADDR_LOWER, "ETH"))
        .await
        .unwrap();

    assert!(
        events.is_empty(),
        "expected no events for tx with to = empty string, got {events:?}"
    );
}

#[tokio::test]
async fn contract_address_mixed_case_normalization() {
    // The contract address in a runtime TargetMap may differ in casing
    // from the `address` field returned by eth_getLogs. Both must be
    // lowercased before comparison so matching is case-insensitive.
    //
    // Use a custom chain without any static ERC-20 assets so only the
    // single runtime asset triggers an eth_getLogs call.
    let mixed_contract = "0xAbCdEf1234567890AbCdEf1234567890AbCdEf12";
    let mixed_upper = mixed_contract.to_uppercase(); // "0XABCDEF..."

    let mut server = Server::new_async().await;
    let logs = rpc_mock(
        &mut server,
        "eth_getLogs",
        json!([{
            "address": mixed_upper,
            "blockNumber": "0x64",
            "transactionHash": "0xmixtx",
            "logIndex": "0x3",
            "removed": false,
            "topics": [
                "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                topic_for_address(EVM_SENDER),
                topic_for_address(EVM_ADDR_LOWER)
            ],
            "data": "0x0f4240"
        }]),
    );
    let timestamp = rpc_mock(
        &mut server,
        "eth_getBlockByNumber",
        json!({"timestamp":"0x665f9a80"}),
    );

    // Chain with only native ETH — no static ERC-20 so eth_getLogs is
    // driven exclusively by the runtime TargetMap below.
    let chain = ChainConfig {
        caip2: "eip155:1".to_string(),
        start_block: Some(0),
        end_block: Some(0),
        confirmed_blocks: 12,
        rpc: vec![server.url()],
        rpc_options: Some(RpcOptions {
            max_concurrent: 2,
            delay_ms: 0,
            batch_size: 10,
            evm_log_address_batching: true,
            scan_lookback_blocks: 0,
            scan_interval_secs: 1,
            scan_timeout_secs: 5,
            max_native_scan_per_cycle: 10,
            request_timeout_secs: 5,
            max_retries: 1,
            retry_base_ms: 1,
            solana_max_supported_transaction_version: 0,
            solana_scan_mode: Default::default(),
        }),
        assets: vec![AssetConfig {
            symbol: "ETH".to_string(),
            contract: None,
            token_program: None,
            decimals: 18,
            min_amount: None,
        }],
    };

    let scanner = EvmScanner::new(chain).unwrap();

    let mut targets = TargetMap::new();
    targets.insert(
        EVM_ADDR_LOWER.to_string(),
        vec![ResolvedAsset {
            symbol: "MIXED".to_string(),
            contract: Some(mixed_contract.to_string()),
            token_program: None,
            decimals: Some(6),
        }],
    );

    let events = scanner.scan(100, 100, &targets).await.unwrap();

    assert_eq!(
        events.len(),
        1,
        "expected 1 event for mixed-case contract, got {events:?}"
    );
    assert_eq!(events[0].data.address, EVM_ADDR_LOWER);
    assert_eq!(events[0].data.symbol, "MIXED");
    assert_eq!(events[0].data.amount, "1000000");
    logs.assert();
    timestamp.assert();
}

#[tokio::test]
async fn erc20_log_missing_transactionhash_defaults_to_empty() {
    // A log entry without a `transactionHash` field must not crash.
    // The tx_id should default to an empty string.
    let mut server = Server::new_async().await;
    let logs = rpc_mock(
        &mut server,
        "eth_getLogs",
        json!([{
            "address": ERC20_CONTRACT,
            "blockNumber": "0x64",
            "logIndex": "0x1",
            "removed": false,
            "topics": [
                "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                topic_for_address(EVM_SENDER),
                topic_for_address(EVM_ADDR_LOWER)
            ],
            "data": "0x0f4240"
        }]),
    );
    let timestamp = rpc_mock(
        &mut server,
        "eth_getBlockByNumber",
        json!({"timestamp":"0x665f9a80"}),
    );
    let scanner = EvmScanner::new(evm_chain(server.url())).unwrap();

    let events = scanner
        .scan(100, 100, &erc20_targets(EVM_ADDR_LOWER))
        .await
        .unwrap();

    assert_eq!(
        events.len(),
        1,
        "expected 1 event for log without transactionHash, got {events:?}"
    );
    assert_eq!(events[0].data.tx_id, "");
    assert_eq!(events[0].data.address, EVM_ADDR_LOWER);
    assert_eq!(events[0].data.amount, "1000000");
    logs.assert();
    timestamp.assert();
}

#[tokio::test]
async fn erc20_scan_batches_only_actively_watched_contracts() {
    let mut server = Server::new_async().await;
    let usdt_contract = "0x3333333333333333333333333333333333333333";

    let logs = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({
            "method": "eth_getLogs",
            "params": [{
                "address": [ERC20_CONTRACT, usdt_contract],
                "fromBlock": "0x64",
                "toBlock": "0x64"
            }]
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc":"2.0","id":1,"result":[]}).to_string())
        .create();

    let mut chain = evm_chain(server.url());
    chain.assets.push(AssetConfig {
        symbol: "USDT".to_string(),
        contract: Some(usdt_contract.to_string()),
        token_program: None,
        decimals: 6,
        min_amount: None,
    });

    let mut watched = erc20_targets(EVM_ADDR_LOWER);
    watched
        .get_mut(EVM_ADDR_LOWER)
        .unwrap()
        .push(ResolvedAsset {
            symbol: "USDT".to_string(),
            contract: Some(usdt_contract.to_string()),
            token_program: None,
            decimals: Some(6),
        });

    let scanner = EvmScanner::new(chain).unwrap();
    let events = scanner.scan(100, 100, &watched).await.unwrap();

    assert!(events.is_empty());
    logs.assert();
}

#[tokio::test]
async fn erc20_scan_skips_configured_contracts_without_watchers() {
    let mut server = Server::new_async().await;
    let unwatched_contract = "0x3333333333333333333333333333333333333333";

    let logs = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({
            "method": "eth_getLogs",
            "params": [{
                "address": ERC20_CONTRACT,
                "fromBlock": "0x64",
                "toBlock": "0x64"
            }]
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc":"2.0","id":1,"result":[]}).to_string())
        .create();

    let mut chain = evm_chain(server.url());
    chain.assets.push(AssetConfig {
        symbol: "USDT".to_string(),
        contract: Some(unwatched_contract.to_string()),
        token_program: None,
        decimals: 6,
        min_amount: None,
    });

    let scanner = EvmScanner::new(chain).unwrap();
    let events = scanner
        .scan(100, 100, &erc20_targets(EVM_ADDR_LOWER))
        .await
        .unwrap();

    assert!(events.is_empty());
    logs.assert();
}

// NOTE: The following EVM edge cases are not testable in chain_evm.rs
// because the behavior lives in the detector layer (detector/mod.rs and
// detector/util.rs), not in the scanner:
//
//   * scan_lookback_blocks configuration effect
//     The lookback is applied by the detector's `scan_start` closure
//     and `effective_scan_to_block` / `effective_evm_native_scan_to_block`
//     in detector/util.rs. The EvmScanner::scan() method simply accepts
//     from_block/to_block and has no knowledge of lookback.
//
//   * max_native_scan_per_cycle limiting
//     The native-scan cap is enforced by `effective_evm_native_scan_to_block`
//     in detector/util.rs, which adjusts the to_block passed to scan().
//     The scanner itself performs no cap enforcement.
//
// These should be tested in a dedicated tests/detector.rs suite.
