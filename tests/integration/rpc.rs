mod common;

use mockito::{Matcher, Server};
use pano::config::{ChainConfig, RpcOptions};
use pano::rpc::RpcClient;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Thin helper: build a ChainConfig suitable for RpcClient tests.
fn rpc_chain(rpc_urls: Vec<String>, max_retries: u32, retry_base_ms: u64) -> ChainConfig {
    rpc_chain_with_options(rpc_urls, max_retries, retry_base_ms, 2, 5)
}

fn rpc_chain_with_options(
    rpc_urls: Vec<String>,
    max_retries: u32,
    retry_base_ms: u64,
    max_concurrent: usize,
    request_timeout_secs: u64,
) -> ChainConfig {
    ChainConfig {
        caip2: "eip155:1".to_string(),
        start_block: Some(0),
        end_block: Some(0),
        confirmed_blocks: 12,
        rpc: rpc_urls,
        rpc_options: Some(RpcOptions {
            max_concurrent,
            delay_ms: 0,
            batch_size: 10,
            evm_log_address_batching: true,
            scan_lookback_blocks: 0,
            scan_interval_secs: 1,
            scan_timeout_secs: 5,
            max_native_scan_per_cycle: 10,
            solana_scan_mode: Default::default(),
            request_timeout_secs,
            max_retries,
            retry_base_ms,
            solana_max_supported_transaction_version: 0,
        }),
        assets: vec![],
    }
}

async fn start_delayed_json_rpc_server(
    delay: Duration,
    max_seen: Arc<AtomicUsize>,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let active = Arc::new(AtomicUsize::new(0));
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let active = active.clone();
            let max_seen = max_seen.clone();
            tokio::spawn(async move {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                max_seen.fetch_max(current, Ordering::SeqCst);
                let mut buf = [0_u8; 2048];
                let _ = socket.read(&mut buf).await;
                tokio::time::sleep(delay).await;
                let body = json!({"jsonrpc":"2.0","id":1,"result":"0x2a"}).to_string();
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
                active.fetch_sub(1, Ordering::SeqCst);
            });
        }
    });
    (url, handle)
}

// ── Retry: max_retries rounds after the initial attempt ───────────────────

/// With max_retries=1 the client must perform 2 total rounds (initial + 1 retry)
/// and eventually succeed on the second JSON-RPC response.
#[tokio::test]
async fn retry_with_max_retries_1_makes_two_total_attempts_and_succeeds() {
    let mut server = Server::new_async().await;

    // First attempt: fail with 503
    let _fail = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_blockNumber"})))
        .with_status(503)
        .expect(1)
        .create();

    // Second attempt (retry): succeed
    let success = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_blockNumber"})))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc":"2.0","id":1,"result":"0x1"}).to_string())
        .expect(1)
        .create();

    let chain = rpc_chain(vec![server.url()], 1, 0);
    let client = RpcClient::new(chain);

    let result = client.send("eth_blockNumber", json!([])).await;

    assert!(result.is_ok(), "expected Ok on retry, got: {:?}", result);
    success.assert();
}

// ── Retry: endpoint failover within a single round ─────────────────────────

/// First endpoint returns 503; second endpoint succeeds — all within round 0
/// (no retry round needed).  Uses max_retries=0 to isolate failover behaviour.
#[tokio::test]
async fn endpoint_failover_first_fails_second_succeeds_in_same_round() {
    let mut server1 = Server::new_async().await;
    let mut server2 = Server::new_async().await;

    // Endpoint 0 always fails
    let _fail = server1
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_blockNumber"})))
        .with_status(503)
        .expect(1)
        .create();

    // Endpoint 1 succeeds
    let success = server2
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_blockNumber"})))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc":"2.0","id":1,"result":"0x2"}).to_string())
        .expect(1)
        .create();

    let chain = rpc_chain(vec![server1.url(), server2.url()], 0, 0);
    let client = RpcClient::new(chain);

    let result = client.send("eth_blockNumber", json!([])).await;

    assert!(
        result.is_ok(),
        "expected Ok from second endpoint, got: {:?}",
        result
    );
    assert_eq!(
        result.unwrap(),
        json!("0x2"),
        "response should come from the second endpoint"
    );
    success.assert();
}

// ── Retry: exhausted retries returns error ─────────────────────────────────

/// With max_retries=1 and a single persistently-failing endpoint, the client
/// must attempt 2 total rounds (initial + 1 retry) and then return an error.
#[tokio::test]
async fn exhausted_retries_returns_error_after_expected_attempts() {
    let mut server = Server::new_async().await;

    // Single mock expects exactly 2 calls (initial + one retry).
    let fail = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_blockNumber"})))
        .with_status(503)
        .expect(2)
        .create();

    let chain = rpc_chain(vec![server.url()], 1, 0);
    let client = RpcClient::new(chain);

    let result = client.send("eth_blockNumber", json!([])).await;

    assert!(
        result.is_err(),
        "expected error after exhausting retries, got: {:?}",
        result
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("all RPC endpoints failed"),
        "error message should mention endpoint failure, got: {err}"
    );
    fail.assert();
}

#[tokio::test]
async fn request_timeout_triggers_error_after_duration() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        if let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0_u8; 128];
            let _ = socket.read(&mut buf).await;
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });

    let chain = rpc_chain_with_options(vec![url], 0, 0, 1, 1);
    let client = RpcClient::new(chain);

    let result = client.send("eth_blockNumber", json!([])).await;

    assert!(result.is_err(), "expected timeout error, got {result:?}");
    server.abort();
}

#[test]
fn retry_backoff_is_exponential_and_saturating() {
    let client = RpcClient::new(rpc_chain(vec!["http://127.0.0.1:1".to_string()], 3, 100));

    assert_eq!(client.retry_backoff(0), Duration::from_millis(100));
    assert_eq!(client.retry_backoff(1), Duration::from_millis(200));
    assert_eq!(client.retry_backoff(2), Duration::from_millis(400));
}

#[tokio::test]
async fn connection_refused_returns_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);

    let client = RpcClient::new(rpc_chain_with_options(vec![url], 0, 0, 1, 1));
    let result = client.send("eth_blockNumber", json!([])).await;

    assert!(
        result.is_err(),
        "expected connection-refused error, got {result:?}"
    );
}

#[tokio::test]
async fn http_429_triggers_backoff_then_retry() {
    let mut server = Server::new_async().await;
    let rate_limited = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_blockNumber"})))
        .with_status(429)
        .expect(1)
        .create();
    let success = server
        .mock("POST", "/")
        .match_body(Matcher::PartialJson(json!({"method": "eth_blockNumber"})))
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(json!({"jsonrpc":"2.0","id":1,"result":"0x4"}).to_string())
        .expect(1)
        .create();

    let client = RpcClient::new(rpc_chain(vec![server.url()], 1, 0));
    let result = client.send("eth_blockNumber", json!([])).await;

    assert_eq!(result.unwrap(), json!("0x4"));
    rate_limited.assert();
    success.assert();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn max_concurrent_limits_simultaneous_requests() {
    let max_seen = Arc::new(AtomicUsize::new(0));
    let (url, server) =
        start_delayed_json_rpc_server(Duration::from_millis(100), max_seen.clone()).await;
    let client = RpcClient::new(rpc_chain_with_options(vec![url], 0, 0, 1, 5));

    let first = {
        let client = client.clone();
        tokio::spawn(async move { client.send("eth_blockNumber", json!([])).await })
    };
    let second = tokio::spawn(async move { client.send("eth_blockNumber", json!([])).await });

    assert_eq!(first.await.unwrap().unwrap(), json!("0x2a"));
    assert_eq!(second.await.unwrap().unwrap(), json!("0x2a"));
    assert_eq!(
        max_seen.load(Ordering::SeqCst),
        1,
        "semaphore should allow only one in-flight HTTP request"
    );
    server.abort();
}
