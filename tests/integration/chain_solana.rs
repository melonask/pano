mod common;

use mockito::{Matcher, Server};
use pano::chain::ChainScanner;
use pano::chain::solana::SolanaScanner;
use pano::config::{AssetConfig, ChainConfig, RpcOptions, SolanaScanMode};
use pano::model::{ResolvedAsset, TargetMap};
use serde_json::json;

// ── Constants ─────────────────────────────────────────────────────────────

const SOL_WATCHED: &str = "DdZR5kXHVqMq1VEMhFeJMQdRwGF1vAfPqQqMq1VEMhFJ";
const SOL_SENDER: &str = "7SEPJ2DhAXpBDPRLuP1GkPYseB4w6V9YLjPbjY5FJBkE";
const SOL_OTHER: &str = "8Uo0gEzbCm7VxJ31L7CGC4Ko1JEBLFfH6U4b3qQmRqj3hgq";
const SPL_MINT_USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const SPL_MINT_USDT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";
const DEVNET_SOL_OWNER: &str = "3VN9g4VZanawKwVgXVDRe99G27yZmqh2Lbd62UpgXQu7";
const DEVNET_USDC_MINT: &str = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU";
const DEVNET_USDC_ATA: &str = "GyRjiZnwGLZKA3eGRuBk5LnrUqDmUmHthC7k6oyWuELg";
const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

// ── Helpers ───────────────────────────────────────────────────────────────

/// Default Solana chain config pointing at the supplied mockito URL.
fn solana_chain(rpc_url: String) -> ChainConfig {
    solana_chain_with_opts(rpc_url, None)
}

/// Solana chain config with optional RpcOptions override.
fn solana_chain_with_opts(rpc_url: String, opts: Option<RpcOptions>) -> ChainConfig {
    let rpc_options = opts.unwrap_or(RpcOptions {
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
        solana_scan_mode: SolanaScanMode::Signatures,
    });
    ChainConfig {
        caip2: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp".to_string(),
        start_block: None,
        end_block: Some(0),
        confirmed_blocks: 32,
        rpc: vec![rpc_url],
        rpc_options: Some(rpc_options),
        assets: sol_native_assets(),
    }
}

/// Native SOL asset plus common SPL assets.
fn sol_native_assets() -> Vec<AssetConfig> {
    vec![
        AssetConfig {
            symbol: "SOL".to_string(),
            contract: None,
            token_program: None,
            decimals: 9,
            min_amount: None,
        },
        AssetConfig {
            symbol: "USDC".to_string(),
            contract: Some(SPL_MINT_USDC.to_string()),
            token_program: None,
            decimals: 6,
            min_amount: None,
        },
        AssetConfig {
            symbol: "USDT".to_string(),
            contract: Some(SPL_MINT_USDT.to_string()),
            token_program: None,
            decimals: 6,
            min_amount: None,
        },
    ]
}

/// Chain config with max_retries=0 — for error propagation tests.
fn solana_chain_no_retry(rpc_url: String) -> ChainConfig {
    solana_chain_with_opts(
        rpc_url,
        Some(RpcOptions {
            max_concurrent: 1,
            delay_ms: 0,
            batch_size: 10,
            evm_log_address_batching: true,
            scan_lookback_blocks: 0,
            scan_interval_secs: 1,
            scan_timeout_secs: 5,
            max_native_scan_per_cycle: 10,
            request_timeout_secs: 5,
            max_retries: 0,
            retry_base_ms: 1,
            solana_max_supported_transaction_version: 0,
            solana_scan_mode: SolanaScanMode::Signatures,
        }),
    )
}

/// Register a mock JSON-RPC endpoint that matches on method name.
fn rpc_mock(server: &mut Server, method: &str, result: serde_json::Value) -> mockito::Mock {
    server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": method})))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc": "2.0", "id": 1, "result": result}).to_string())
        .create()
}

/// Register a mock that responds with a non-2xx HTTP status (no JSON body).
fn rpc_mock_error(server: &mut Server, method: &str, status: usize) -> mockito::Mock {
    server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": method})))
        .with_status(status)
        .create()
}

/// Build a TargetMap for one address watching native SOL only.
fn sol_targets(addr: &str) -> TargetMap {
    let mut targets = TargetMap::new();
    targets.insert(
        addr.to_string(),
        vec![ResolvedAsset {
            symbol: "SOL".to_string(),
            contract: None,
            token_program: None,
            decimals: Some(9),
        }],
    );
    targets
}

/// Build a TargetMap for one address watching a specific SPL token.
fn spl_targets(owner: &str, symbol: &str, mint: &str, decimals: u32) -> TargetMap {
    let mut targets = TargetMap::new();
    targets.insert(
        owner.to_string(),
        vec![ResolvedAsset {
            symbol: symbol.to_string(),
            contract: Some(mint.to_string()),
            token_program: None,
            decimals: Some(decimals),
        }],
    );
    targets
}

/// Build a TargetMap where one owner watches two different SPL tokens.
fn spl_multi_token_targets(owner: &str) -> TargetMap {
    let mut targets = TargetMap::new();
    targets.insert(
        owner.to_string(),
        vec![
            ResolvedAsset {
                symbol: "USDC".to_string(),
                contract: Some(SPL_MINT_USDC.to_string()),
                token_program: None,
                decimals: Some(6),
            },
            ResolvedAsset {
                symbol: "USDT".to_string(),
                contract: Some(SPL_MINT_USDT.to_string()),
                token_program: None,
                decimals: Some(6),
            },
        ],
    );
    targets
}

/// Helper: build a getSignaturesForAddress response with one signature.
fn one_sig_response(sig: &str, slot: u64) -> serde_json::Value {
    json!([{
        "signature": sig,
        "slot": slot,
        "err": null,
        "memo": null,
        "blockTime": 1717459200i64
    }])
}

/// Helper: build a getTransaction result for a native SOL transfer.
/// account_keys: [sender, watched, other]
/// pre:  [10_000_000_000, 1_000_000_000, 5_000_000_000]
/// post: [ 9_999_995_000, 1_500_000_000, 4_500_000_000]
///   -> sender lost 5_000 lamports fee; watched gained 500_000_000 lamports = 0.5 SOL
fn build_native_sol_tx(
    _sig: &str,
    sender: &str,
    watched: &str,
    other: &str,
    pre_watched: u64,
    post_watched: u64,
) -> serde_json::Value {
    let pre_sender: u64 = 10_000_000_000;
    let post_sender: u64 = pre_sender.saturating_sub(5_000); // fee
    let pre_other: u64 = 5_000_000_000;
    // Adjust other's post-balance to keep total consistent (sender fee = 5k)
    let post_other: u64 = pre_other;

    json!({
        "blockTime": 1717459200i64,
        "meta": {
            "err": null,
            "preBalances": [pre_sender, pre_watched, pre_other],
            "postBalances": [post_sender, post_watched, post_other],
            "preTokenBalances": [],
            "postTokenBalances": []
        },
        "transaction": {
            "message": {
                "accountKeys": [sender, watched, other]
            }
        }
    })
}

/// Helper: build a getTransaction result with SPL token balance changes.
/// account_keys: [sender_owner, watched_owner, mint_authority]
fn build_spl_tx(
    block_time: i64,
    account_keys: &[&str],
    pre_balances: &[u64],
    post_balances: &[u64],
    pre_tokens: serde_json::Value,
    post_tokens: serde_json::Value,
) -> serde_json::Value {
    json!({
        "blockTime": block_time,
        "meta": {
            "err": null,
            "preBalances": pre_balances,
            "postBalances": post_balances,
            "preTokenBalances": pre_tokens,
            "postTokenBalances": post_tokens
        },
        "transaction": {
            "message": {
                "accountKeys": account_keys
            }
        }
    })
}

// ── get_tip parses slot number ────────────────────────────────────────────

#[tokio::test]
async fn get_tip_parses_slot_number() {
    let mut server = Server::new_async().await;
    let mock = rpc_mock(&mut server, "getSlot", json!(250_000_000));
    let scanner = SolanaScanner::new(solana_chain(server.url())).unwrap();

    assert_eq!(scanner.get_tip().await.unwrap(), 250_000_000);
    mock.assert();
}

// ── get_tip errors on malformed response ──────────────────────────────────

#[tokio::test]
async fn get_tip_errors_on_malformed_response() {
    let mut server = Server::new_async().await;
    let _mock = rpc_mock(&mut server, "getSlot", json!("not_a_number"));
    let scanner = SolanaScanner::new(solana_chain(server.url())).unwrap();

    let err = scanner.get_tip().await.unwrap_err().to_string();
    assert!(
        err.contains("non-u64"),
        "expected 'non-u64' in error, got: {err}"
    );
}

// ── scan detects SPL token transfer ───────────────────────────────────────

#[tokio::test]
async fn scan_detects_spl_token_transfer() {
    let mut server = Server::new_async().await;

    // getSignaturesForAddress is called twice (initial + pagination follow-up).
    let _sigs_mock = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(
            json!({"method": "getSignaturesForAddress"}),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({"jsonrpc": "2.0", "id": 1, "result": one_sig_response("spl_sig_1", 123_000_000)})
                .to_string(),
        )
        .expect_at_least(2)
        .create();

    // accountKeys: [fee_payer, watched_owner, sender_token_acct, receiver_token_acct]
    // Sender token acct (index 2): pre 500000, post 0 (sent all)
    // Receiver token acct (index 3): pre 0, post 1000000 (received)
    let tx_mock = rpc_mock(
        &mut server,
        "getTransaction",
        build_spl_tx(
            1717459200,
            &[
                SOL_SENDER,
                SOL_WATCHED,
                "TAcctSender11111111111111111111111111",
                "TAcctReceiver1111111111111111111111",
            ],
            &[5_000_000_000, 1_000_000_000, 2_039_280, 2_039_280],
            &[4_995_000_000, 1_000_000_000, 2_039_280, 2_039_280],
            json!([
                {"accountIndex": 2, "mint": SPL_MINT_USDC, "owner": SOL_SENDER, "uiTokenAmount": {"amount": "500000", "decimals": 6}},
                {"accountIndex": 3, "mint": SPL_MINT_USDC, "owner": SOL_WATCHED, "uiTokenAmount": {"amount": "0", "decimals": 6}}
            ]),
            json!([
                {"accountIndex": 2, "mint": SPL_MINT_USDC, "owner": SOL_SENDER, "uiTokenAmount": {"amount": "0", "decimals": 6}},
                {"accountIndex": 3, "mint": SPL_MINT_USDC, "owner": SOL_WATCHED, "uiTokenAmount": {"amount": "1000000", "decimals": 6}}
            ]),
        ),
    );

    let scanner = SolanaScanner::new(solana_chain(server.url())).unwrap();
    let targets = spl_targets(SOL_WATCHED, "USDC", SPL_MINT_USDC, 6);

    let events = scanner
        .scan(123_000_000, 123_000_000, &targets)
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data.tx_id, "spl_sig_1");
    assert_eq!(events[0].data.symbol, "USDC");
    assert_eq!(events[0].data.address, SOL_WATCHED);
    assert_eq!(events[0].data.block_number, 123_000_000);
    // post 1_000_000 - pre 0 = 1_000_000 raw units
    assert_eq!(events[0].data.amount, "1000000");
    assert_eq!(events[0].data.sender, SOL_SENDER);
    assert_eq!(events[0].data.log_index, 0);
    assert_eq!(events[0].data.confirmations, 1);
    assert!(!events[0].data.timestamp.is_empty());
    tx_mock.assert();
}

#[tokio::test]
async fn scan_detects_spl_token_transfer_from_derived_ata_signature() {
    let mut server = Server::new_async().await;

    let _owner_sigs = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({
            "method": "getSignaturesForAddress",
            "params": [DEVNET_SOL_OWNER]
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc": "2.0", "id": 1, "result": []}).to_string())
        .expect_at_least(1)
        .create();

    let _ata_sigs = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({
            "method": "getSignaturesForAddress",
            "params": [DEVNET_USDC_ATA]
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({"jsonrpc": "2.0", "id": 1, "result": one_sig_response("ata_sig_1", 123_000_000)})
                .to_string(),
        )
        .expect_at_least(1)
        .create();

    let tx_mock = rpc_mock(
        &mut server,
        "getTransaction",
        build_spl_tx(
            1717459200,
            &[
                SOL_SENDER,
                DEVNET_USDC_ATA,
                "SenderTokenAcct111111111111111111111",
                DEVNET_SOL_OWNER,
            ],
            &[5_000_000_000, 2_039_280, 2_039_280, 1_000_000_000],
            &[4_995_000_000, 2_039_280, 2_039_280, 1_000_000_000],
            json!([
                {"accountIndex": 2, "mint": DEVNET_USDC_MINT, "owner": SOL_SENDER, "uiTokenAmount": {"amount": "500000", "decimals": 6}},
                {"accountIndex": 1, "mint": DEVNET_USDC_MINT, "owner": DEVNET_SOL_OWNER, "uiTokenAmount": {"amount": "0", "decimals": 6}}
            ]),
            json!([
                {"accountIndex": 2, "mint": DEVNET_USDC_MINT, "owner": SOL_SENDER, "uiTokenAmount": {"amount": "490000", "decimals": 6}},
                {"accountIndex": 1, "mint": DEVNET_USDC_MINT, "owner": DEVNET_SOL_OWNER, "uiTokenAmount": {"amount": "10000", "decimals": 6}}
            ]),
        ),
    );

    let scanner = SolanaScanner::new(solana_chain(server.url())).unwrap();
    let mut targets = spl_targets(DEVNET_SOL_OWNER, "USDC", DEVNET_USDC_MINT, 6);
    targets.get_mut(DEVNET_SOL_OWNER).unwrap()[0].token_program =
        Some(SPL_TOKEN_PROGRAM_ID.to_string());

    let events = scanner
        .scan(123_000_000, 123_000_000, &targets)
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data.tx_id, "ata_sig_1");
    assert_eq!(events[0].data.symbol, "USDC");
    assert_eq!(events[0].data.address, DEVNET_SOL_OWNER);
    assert_eq!(events[0].data.amount, "10000");
    tx_mock.assert();
}

// ── scan detects native SOL transfer ──────────────────────────────────────

#[tokio::test]
async fn scan_detects_native_sol_transfer() {
    let mut server = Server::new_async().await;

    let _sigs_mock = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(
            json!({"method": "getSignaturesForAddress"}),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({"jsonrpc": "2.0", "id": 1, "result": one_sig_response("sol_sig_1", 200_000_000)})
                .to_string(),
        )
        .expect_at_least(2)
        .create();

    let tx_mock = rpc_mock(
        &mut server,
        "getTransaction",
        build_native_sol_tx(
            "sol_sig_1",
            SOL_SENDER,
            SOL_WATCHED,
            SOL_OTHER,
            1_000_000_000, // pre watched = 1 SOL
            1_500_000_000, // post watched = 1.5 SOL -> 0.5 SOL gain
        ),
    );

    let scanner = SolanaScanner::new(solana_chain(server.url())).unwrap();
    let targets = sol_targets(SOL_WATCHED);

    let events = scanner
        .scan(200_000_000, 200_000_000, &targets)
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data.tx_id, "sol_sig_1");
    assert_eq!(events[0].data.symbol, "SOL");
    assert_eq!(events[0].data.address, SOL_WATCHED);
    assert_eq!(events[0].data.block_number, 200_000_000);
    // 1_500_000_000 - 1_000_000_000 = 500_000_000 lamports
    assert_eq!(events[0].data.amount, "500000000");
    assert_eq!(events[0].data.sender, SOL_SENDER);
    assert_eq!(events[0].data.confirmations, 1);
    tx_mock.assert();
}

// ── scan skips zero-amount transactions ───────────────────────────────────

#[tokio::test]
async fn scan_skips_zero_amount_transactions() {
    let mut server = Server::new_async().await;

    let _sigs_mock = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(
            json!({"method": "getSignaturesForAddress"}),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({"jsonrpc": "2.0", "id": 1, "result": one_sig_response("zero_sig", 100_000_000)})
                .to_string(),
        )
        .expect_at_least(2)
        .create();

    // pre == post for the watched address -> no net change.
    let _tx_mock = rpc_mock(
        &mut server,
        "getTransaction",
        build_native_sol_tx(
            "zero_sig",
            SOL_SENDER,
            SOL_WATCHED,
            SOL_OTHER,
            1_000_000_000, // pre
            1_000_000_000, // post (same)
        ),
    );

    let scanner = SolanaScanner::new(solana_chain(server.url())).unwrap();
    let targets = sol_targets(SOL_WATCHED);

    let events = scanner
        .scan(100_000_000, 100_000_000, &targets)
        .await
        .unwrap();

    assert!(events.is_empty(), "expected no events for zero net change");
}

// ── scan short-circuits empty targets ─────────────────────────────────────

#[tokio::test]
async fn scan_short_circuits_empty_targets() {
    let server = Server::new_async().await;
    let scanner = SolanaScanner::new(solana_chain(server.url())).unwrap();

    let events = scanner.scan(1, 100, &TargetMap::default()).await.unwrap();
    assert!(events.is_empty());
}

// ── scan handles failed RPC gracefully ────────────────────────────────────

#[tokio::test]
async fn scan_handles_failed_rpc_gracefully() {
    let mut server = Server::new_async().await;
    // Return HTTP 500 so the RPC call fails immediately. With max_retries=0
    // there is no retry and the error propagates to the scan result.
    let _mock = rpc_mock_error(&mut server, "getSignaturesForAddress", 500);

    let scanner = SolanaScanner::new(solana_chain_no_retry(server.url())).unwrap();
    let targets = sol_targets(SOL_WATCHED);

    let err = scanner
        .scan(100, 100, &targets)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("all RPC endpoints failed")
            || err.contains("HTTP status client error")
            || err.contains("500 Internal Server Error"),
        "expected RPC failure error, got: {err}"
    );
}

#[tokio::test]
async fn scan_drops_pruned_solana_before_cursor_and_rescans() {
    let mut server = Server::new_async().await;

    let _first_page = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({
            "method": "getSignaturesForAddress",
            "params": [SOL_WATCHED, {"limit": 10, "minContextSlot": 100, "commitment": "confirmed"}]
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({"jsonrpc": "2.0", "id": 1, "result": one_sig_response("old_cursor", 100)})
                .to_string(),
        )
        .expect(2)
        .create();

    let _pruned_cursor = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({
            "method": "getSignaturesForAddress",
            "params": [SOL_WATCHED, {"before": "old_cursor"}]
        })))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc": "2.0", "id": 2, "error": {"code": -32020, "message": "Transaction not found"}}).to_string())
        .expect(1)
        .create();

    let _tx_mock = rpc_mock(
        &mut server,
        "getTransaction",
        build_native_sol_tx(
            "old_cursor",
            SOL_SENDER,
            SOL_WATCHED,
            SOL_OTHER,
            0,
            1_000_000_000,
        ),
    );

    let scanner = SolanaScanner::new(solana_chain_no_retry(server.url())).unwrap();
    let events = scanner
        .scan(100, 100, &sol_targets(SOL_WATCHED))
        .await
        .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data.address, SOL_WATCHED);
}

// ── SPL token with missing uiTokenAmount ──────────────────────────────────

#[tokio::test]
async fn spl_token_with_missing_amount() {
    let mut server = Server::new_async().await;

    let _sigs_mock = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(
            json!({"method": "getSignaturesForAddress"}),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({"jsonrpc": "2.0", "id": 1, "result": one_sig_response("missing_amt", 150_000_000)})
                .to_string(),
        )
        .expect_at_least(2)
        .create();

    // post_token has no uiTokenAmount -> token_amount returns 0.
    let _tx_mock = rpc_mock(
        &mut server,
        "getTransaction",
        build_spl_tx(
            1717459200,
            &[SOL_SENDER, SOL_WATCHED, SPL_MINT_USDC],
            &[5_000_000_000, 1_000_000_000, 2_039_280],
            &[4_995_000_000, 1_000_000_000, 2_039_280],
            json!([]),
            json!([
                {"accountIndex": 2, "mint": SPL_MINT_USDC, "owner": SOL_WATCHED}
            ]),
        ),
    );

    let scanner = SolanaScanner::new(solana_chain(server.url())).unwrap();
    let targets = spl_targets(SOL_WATCHED, "USDC", SPL_MINT_USDC, 6);

    let events = scanner
        .scan(150_000_000, 150_000_000, &targets)
        .await
        .unwrap();

    // post_amount == 0, pre_amount == 0, post > pre is false -> no event.
    assert!(
        events.is_empty(),
        "expected no events when uiTokenAmount is missing"
    );
}

// ── Multiple token accounts same owner ────────────────────────────────────

#[tokio::test]
async fn multiple_token_accounts_same_owner() {
    let mut server = Server::new_async().await;

    let _sigs_mock = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(
            json!({"method": "getSignaturesForAddress"}),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({"jsonrpc": "2.0", "id": 1, "result": one_sig_response("multi_acct", 175_000_000)})
                .to_string(),
        )
        .expect_at_least(2)
        .create();

    // Two SPL token accounts owned by the same address, both receiving tokens
    // in the same transaction. accountIndex 2 = SPL_MINT_USDC, accountIndex 3 = SPL_MINT_USDT.
    let _tx_mock = rpc_mock(
        &mut server,
        "getTransaction",
        build_spl_tx(
            1717459200,
            &[SOL_SENDER, SOL_WATCHED, SPL_MINT_USDC, SPL_MINT_USDT],
            &[5_000_000_000, 1_000_000_000, 2_039_280, 2_039_280],
            &[4_995_000_000, 1_000_000_000, 2_039_280, 2_039_280],
            json!([
                {"accountIndex": 2, "mint": SPL_MINT_USDC, "owner": SOL_SENDER, "uiTokenAmount": {"amount": "200", "decimals": 6}},
                {"accountIndex": 3, "mint": SPL_MINT_USDT, "owner": SOL_SENDER, "uiTokenAmount": {"amount": "100", "decimals": 6}}
            ]),
            json!([
                {"accountIndex": 2, "mint": SPL_MINT_USDC, "owner": SOL_WATCHED, "uiTokenAmount": {"amount": "1200", "decimals": 6}},
                {"accountIndex": 3, "mint": SPL_MINT_USDT, "owner": SOL_WATCHED, "uiTokenAmount": {"amount": "1100", "decimals": 6}}
            ]),
        ),
    );

    let scanner = SolanaScanner::new(solana_chain(server.url())).unwrap();
    let targets = spl_multi_token_targets(SOL_WATCHED);

    let events = scanner
        .scan(175_000_000, 175_000_000, &targets)
        .await
        .unwrap();

    assert_eq!(events.len(), 2, "expected 2 SPL events, got {events:?}");

    // USDC event: post 1200 - pre 200 = 1000
    let usdc = events.iter().find(|e| e.data.symbol == "USDC").unwrap();
    assert_eq!(usdc.data.amount, "1000");
    assert_eq!(usdc.data.address, SOL_WATCHED);
    assert_eq!(usdc.data.log_index, 0);

    // USDT event: post 1100 - pre 100 = 1000
    let usdt = events.iter().find(|e| e.data.symbol == "USDT").unwrap();
    assert_eq!(usdt.data.amount, "1000");
    assert_eq!(usdt.data.address, SOL_WATCHED);
    assert_eq!(usdt.data.log_index, 1);
}

// ── Pre/post balance reconciliation ───────────────────────────────────────

#[tokio::test]
async fn pre_post_balance_reconciliation() {
    let mut server = Server::new_async().await;

    let _sigs_mock = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(
            json!({"method": "getSignaturesForAddress"}),
        ))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            json!({"jsonrpc": "2.0", "id": 1, "result": one_sig_response("recon_sig", 180_000_000)})
                .to_string(),
        )
        .expect_at_least(2)
        .create();

    // Native SOL: pre=0, post=7_000_000_000 -> amount = 7 SOL worth of lamports.
    // SPL USDC: pre=50, post=250 -> amount = 200 raw units.
    let _tx_mock = rpc_mock(
        &mut server,
        "getTransaction",
        build_spl_tx(
            1717459200,
            &[SOL_SENDER, SOL_WATCHED, SPL_MINT_USDC],
            &[10_000_000_000, 0, 2_039_280],
            &[9_995_000_000, 7_000_000_000, 2_039_280],
            json!([
                {"accountIndex": 2, "mint": SPL_MINT_USDC, "owner": SOL_SENDER, "uiTokenAmount": {"amount": "50", "decimals": 6}}
            ]),
            json!([
                {"accountIndex": 2, "mint": SPL_MINT_USDC, "owner": SOL_WATCHED, "uiTokenAmount": {"amount": "250", "decimals": 6}}
            ]),
        ),
    );

    let scanner = SolanaScanner::new(solana_chain(server.url())).unwrap();

    // Watch both SOL and USDC for the same address.
    let mut targets = TargetMap::new();
    targets.insert(
        SOL_WATCHED.to_string(),
        vec![
            ResolvedAsset {
                symbol: "SOL".to_string(),
                contract: None,
                token_program: None,
                decimals: Some(9),
            },
            ResolvedAsset {
                symbol: "USDC".to_string(),
                contract: Some(SPL_MINT_USDC.to_string()),
                token_program: None,
                decimals: Some(6),
            },
        ],
    );

    let events = scanner
        .scan(180_000_000, 180_000_000, &targets)
        .await
        .unwrap();

    // Native SOL event: 7_000_000_000 - 0 = 7_000_000_000
    let sol_ev = events.iter().find(|e| e.data.symbol == "SOL").unwrap();
    assert_eq!(sol_ev.data.amount, "7000000000");
    assert_eq!(sol_ev.data.address, SOL_WATCHED);

    // SPL USDC event: 250 - 50 = 200
    let usdc_ev = events.iter().find(|e| e.data.symbol == "USDC").unwrap();
    assert_eq!(usdc_ev.data.amount, "200");
    assert_eq!(usdc_ev.data.address, SOL_WATCHED);

    assert_eq!(events.len(), 2);
}
