use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use pano::config::{
    AppConfig, AssetConfig, ChainConfig, DetectorConfig, EgressConfig, HttpIngressConfig,
    IngressConfig, OverrideChains, OverrideConfig, ServerConfig,
};
use pano::detector::{DetectorHandle, resolve_watch_spec_to_watches};
use pano::model::Command;
use pano::server::router::router;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use tower::ServiceExt;

// ── Path constants (default config: prefix=v1, addresses=addresses) ──────

const WATCH_PATH: &str = "/v1/addresses";
const HEALTH_PATH: &str = "/healthz";
// EVM addresses are normalized to lowercase by normalize_address_key.
const RAW_ADDR: &str = "0xAbCdEf1234567890AbCdEf1234567890AbCdEf12";
const NORM_ADDR: &str = "0xabcdef1234567890abcdef1234567890abcdef12";

/// Build a minimal valid AppConfig for API ingress testing.
fn test_config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            enabled: true,
            bind: "0.0.0.0".into(),
            port: 3210,
            prefix: "v1".into(),
            dashboard: String::new(),
            dashboard_export: false,
            api_key: String::new(),
            shutdown_timeout_secs: 1,
        },
        detector: DetectorConfig::default(),
        chains: vec![ChainConfig {
            caip2: "eip155:1".into(),
            start_block: None,
            end_block: None,
            confirmed_blocks: 12,
            rpc: vec!["http://localhost:8545".into()],
            rpc_options: None,
            assets: vec![AssetConfig {
                symbol: "ETH".into(),
                contract: None,
                token_program: None,
                decimals: 18,
                min_amount: None,
            }],
        }],
        ingress: IngressConfig {
            http: HttpIngressConfig {
                enabled: true,
                addresses: "addresses".into(),
                max_body_bytes: 1_048_576,
            },
            ..Default::default()
        },
        egress: EgressConfig::default(),
        override_: OverrideConfig {
            chains: Some(OverrideChains { assets: true }),
            ..Default::default()
        },
    }
}

/// Config with file ingress enabled and authoritative.
fn test_config_authoritative_file() -> AppConfig {
    let mut cfg = test_config();
    cfg.ingress.file.enabled = true;
    cfg.ingress.file.authoritative = true;
    cfg.ingress.file.path = "/tmp/pano-test-ingress.json".into();
    cfg
}

/// Config with API key authentication.
fn test_config_with_api_key() -> AppConfig {
    let mut cfg = test_config();
    cfg.server.api_key = "test-secret-key".into();
    cfg
}

/// Build a DetectorHandle backed by a test command channel.
/// Returns the handle and the receiver for draining/verifying commands.
fn test_handle(config: AppConfig) -> (DetectorHandle, mpsc::Receiver<Command>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(256);
    let (events_tx, _) = tokio::sync::broadcast::channel::<pano::model::DepositEvent>(16);
    let watched = Arc::new(RwLock::new(hashbrown::HashMap::new()));
    let (address_change_tx, _) = tokio::sync::watch::channel(());
    let handle = DetectorHandle {
        cmd_tx,
        events_tx,
        watched,
        config: Arc::new(config),
        address_change_tx,
    };
    (handle, cmd_rx)
}

/// Spawn a background task that drains all commands from the channel.
/// Prevents channel backpressure from blocking the handler.
fn drain_commands(mut rx: mpsc::Receiver<Command>) {
    tokio::spawn(async move { while rx.recv().await.is_some() {} });
}

#[tokio::test]
async fn health_reports_detector_command_loop_liveness() {
    let (handle, command_rx) = test_handle(test_config());
    let app = router(handle);
    let response = app
        .oneshot(Request::get(HEALTH_PATH).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    drop(command_rx);
}

#[tokio::test]
async fn health_requires_internal_api_key_when_configured() {
    let (handle, _command_rx) = test_handle(test_config_with_api_key());
    let app = router(handle);
    let response = app
        .oneshot(Request::get(HEALTH_PATH).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ── Test 1: POST with valid WatchSpec returns 201 ────────────────────────

#[tokio::test]
async fn post_watch_valid_spec_returns_201() {
    let (handle, cmd_rx) = test_handle(test_config());
    drain_commands(cmd_rx);

    let app = router(handle);
    let body = serde_json::json!({"address": RAW_ADDR});

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(WATCH_PATH)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
}

// ── Test 2: Invalid JSON returns 400 ─────────────────────────────────────

#[tokio::test]
async fn post_watch_invalid_json_returns_400() {
    let (handle, cmd_rx) = test_handle(test_config());
    drain_commands(cmd_rx);

    let app = router(handle);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(WATCH_PATH)
                .header("content-type", "application/json")
                .body(Body::from("{not valid json"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ── Test 3: Unknown fields rejected (deny_unknown_fields) ────────────────

#[tokio::test]
async fn post_watch_unknown_fields_rejected_returns_422() {
    let (handle, cmd_rx) = test_handle(test_config());
    drain_commands(cmd_rx);

    let app = router(handle);
    let body = serde_json::json!({
        "address": RAW_ADDR,
        "unexpected_field": "should-not-be-here"
    });

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(WATCH_PATH)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // WatchSpec has #[serde(deny_unknown_fields)], so axum's Json extractor
    // returns 422 Unprocessable Entity for deserialization errors.
    assert_eq!(
        response.status(),
        StatusCode::UNPROCESSABLE_ENTITY,
        "unknown fields should be rejected by serde deny_unknown_fields"
    );
}

// ── Test 4: DELETE with watched address returns 204 ──────────────────────

#[tokio::test]
async fn delete_watch_address_returns_204() {
    let (handle, cmd_rx) = test_handle(test_config());
    drain_commands(cmd_rx);

    // Register a watch entry so the address is known to the watched map.
    // normalize_address_key lowercases EVM addresses; use the normalized form.
    {
        let mut w = handle.watched.write().await;
        w.insert(
            (NORM_ADDR.into(), "eip155:1".into(), "ETH".into()),
            pano::model::ResolvedWatch {
                address: NORM_ADDR.into(),
                caip2: "eip155:1".into(),
                symbol: "ETH".into(),
                contract: None,
                token_program: None,
                decimals: Some(18),
                start_block: None,
                end_block: None,
                confirmed_blocks: 12,
                min_amount: None,
                egress: None,
            },
        );
    }

    let app = router(handle);
    let delete_uri = format!("{WATCH_PATH}/{RAW_ADDR}");

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&delete_uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

// ── Test 5: GET /watch returns 405 (no GET handler registered) ───────────

#[tokio::test]
async fn get_watch_list_not_implemented_returns_405() {
    let (handle, cmd_rx) = test_handle(test_config());
    drain_commands(cmd_rx);

    let app = router(handle);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(WATCH_PATH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // The addresses path only has POST registered, so GET returns
    // 405 Method Not Allowed (not 404) because the path matches but
    // no handler accepts GET.
    assert_eq!(
        response.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "GET on /v1/addresses: current API behavior — path exists but has no GET handler (POST only)"
    );
}

// ── Test 6: Duplicate watch address returns 409 Conflict ─────────────────

#[tokio::test]
async fn duplicate_watch_address_returns_409_conflict() {
    let (handle, cmd_rx) = test_handle(test_config());
    drain_commands(cmd_rx);

    // Pre-populate watched map with the normalized triad that the WatchSpec
    // would resolve to (normalize_address_key lowercases EVM).
    {
        let mut w = handle.watched.write().await;
        w.insert(
            (NORM_ADDR.into(), "eip155:1".into(), "ETH".into()),
            pano::model::ResolvedWatch {
                address: NORM_ADDR.into(),
                caip2: "eip155:1".into(),
                symbol: "ETH".into(),
                contract: None,
                token_program: None,
                decimals: Some(18),
                start_block: None,
                end_block: None,
                confirmed_blocks: 12,
                min_amount: None,
                egress: None,
            },
        );
    }

    let app = router(handle);
    let body = serde_json::json!({"address": RAW_ADDR});

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(WATCH_PATH)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
}

// ── Test 7: Watch address enqueues command for detector processing ────────

#[tokio::test]
async fn watch_address_enqueues_command_and_appears_in_watched_state() {
    let (handle, mut cmd_rx) = test_handle(test_config());

    let app = router(handle.clone());
    let spec_body = serde_json::json!({"address": RAW_ADDR});

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(WATCH_PATH)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&spec_body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    // Verify the Command::Watch was enqueued on the detector channel.
    let cmd = cmd_rx.recv().await.expect("expected a Command on cmd_rx");
    let spec = match &cmd {
        Command::Watch(spec) => spec.clone(),
        other => panic!("expected Command::Watch, got: {other:?}"),
    };

    // Simulate detector loop: resolve and insert into watched state.
    let resolved = resolve_watch_spec_to_watches(&handle.config, &spec)
        .expect("valid watch spec should resolve");
    {
        let mut w = handle.watched.write().await;
        for rw in resolved {
            let key = (rw.address.clone(), rw.caip2.clone(), rw.symbol.clone());
            w.insert(key, rw);
        }
    }

    // Assert the address now appears in the watched map (ready for scan cycles).
    // The resolver normalizes the address (lowercase for EVM).
    let watched = handle.watched.read().await;
    let key = (NORM_ADDR.to_string(), "eip155:1".into(), "ETH".into());
    assert!(
        watched.contains_key(&key),
        "watched map should contain the address after command processing"
    );

    // Drain remaining commands.
    drain_commands(cmd_rx);
}

// ── Test 8: API key authentication enforcement ────────────────────────────

#[tokio::test]
async fn api_key_authentication_rejects_unauthenticated_returns_401() {
    let (handle, cmd_rx) = test_handle(test_config_with_api_key());
    drain_commands(cmd_rx);

    let app = router(handle);
    let body = serde_json::json!({"address": RAW_ADDR});

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(WATCH_PATH)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "request without API key should be rejected with 401"
    );
}

#[tokio::test]
async fn api_key_authentication_accepts_valid_bearer_token() {
    let (handle, cmd_rx) = test_handle(test_config_with_api_key());
    drain_commands(cmd_rx);

    let app = router(handle);
    let body = serde_json::json!({"address": RAW_ADDR});

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(WATCH_PATH)
                .header("content-type", "application/json")
                .header("authorization", "Bearer test-secret-key")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "request with valid Bearer token should succeed"
    );
}

#[tokio::test]
async fn api_key_authentication_accepts_valid_x_pano_api_key_header() {
    let (handle, cmd_rx) = test_handle(test_config_with_api_key());
    drain_commands(cmd_rx);

    let app = router(handle);
    let body = serde_json::json!({"address": RAW_ADDR});

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(WATCH_PATH)
                .header("content-type", "application/json")
                .header("x-pano-api-key", "test-secret-key")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "request with valid X-Pano-API-Key header should succeed"
    );
}

#[tokio::test]
async fn api_key_authentication_rejects_wrong_key_returns_401() {
    let (handle, cmd_rx) = test_handle(test_config_with_api_key());
    drain_commands(cmd_rx);

    let app = router(handle);
    let body = serde_json::json!({"address": RAW_ADDR});

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(WATCH_PATH)
                .header("content-type", "application/json")
                .header("authorization", "Bearer wrong-key")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "request with wrong API key should be rejected with 401"
    );
}

// ── Test 9: Reject mutations when file ingress is authoritative ───────────

#[tokio::test]
async fn reject_post_when_file_ingress_authoritative_returns_405() {
    let (handle, cmd_rx) = test_handle(test_config_authoritative_file());
    drain_commands(cmd_rx);

    let app = router(handle);
    let body = serde_json::json!({"address": RAW_ADDR});

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(WATCH_PATH)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "HTTP mutations must be rejected when file ingress is authoritative"
    );

    let body_bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8_lossy(&body_bytes);
    assert!(
        body_str.contains("authoritative"),
        "error message should mention authoritative file ingress, got: {body_str}"
    );
}

#[tokio::test]
async fn reject_delete_when_file_ingress_authoritative_returns_405() {
    let (handle, cmd_rx) = test_handle(test_config_authoritative_file());
    drain_commands(cmd_rx);

    let app = router(handle);
    let delete_uri = format!("{WATCH_PATH}/{RAW_ADDR}");

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&delete_uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::METHOD_NOT_ALLOWED,
        "HTTP DELETE must be rejected when file ingress is authoritative"
    );
}

// ── Additional: DELETE non-existent address returns 404 ───────────────────

#[tokio::test]
async fn delete_nonexistent_address_returns_404() {
    let (handle, cmd_rx) = test_handle(test_config());
    drain_commands(cmd_rx);

    let app = router(handle);
    let delete_uri = format!("{WATCH_PATH}/0xFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF");

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&delete_uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ── Additional: POST without content-type header still works (axum JSON extraction) ──

#[tokio::test]
async fn post_watch_missing_content_type_header_returns_415() {
    // axum 0.8 Json extractor requires Content-Type: application/json by default
    // (via JsonRejection::MissingJsonContentType → 415).
    let (handle, cmd_rx) = test_handle(test_config());
    drain_commands(cmd_rx);

    let app = router(handle);
    let body = serde_json::json!({"address": RAW_ADDR});

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(WATCH_PATH)
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    // axum 0.8 requires Content-Type: application/json for JSON body parsing.
    assert_eq!(
        response.status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "POST without content-type should be rejected with 415 Unsupported Media Type (axum 0.8)"
    );
}

// ── Additional: POST with empty address returns 400 (validation) ──────────

#[tokio::test]
async fn post_watch_empty_spec_returns_400() {
    let (handle, cmd_rx) = test_handle(test_config());
    drain_commands(cmd_rx);

    let app = router(handle);
    // Empty spec: no address, no chains → resolve fails with "at least one of..."
    let body = serde_json::json!({});

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(WATCH_PATH)
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "empty watch spec should be rejected with 400"
    );
}

// ── Additional: DELETE with address normalization (mixed-case EVM) ────────

#[tokio::test]
async fn delete_watch_with_lowercase_address_matches_normalized_uppercase() {
    let (handle, cmd_rx) = test_handle(test_config());
    drain_commands(cmd_rx);

    // Register with normalized (lowercase) address as the detector loop would.
    // The handler resolves and inserts with normalize_address_key, which
    // lowercases EVM addresses. We store the lowercase form.
    let upper_send = "0xABCDEF1234567890ABCDEF1234567890ABCDEF12";
    let lower = "0xabcdef1234567890abcdef1234567890abcdef12";
    {
        let mut w = handle.watched.write().await;
        w.insert(
            (lower.to_string(), "eip155:1".into(), "ETH".into()),
            pano::model::ResolvedWatch {
                address: NORM_ADDR.into(),
                caip2: "eip155:1".into(),
                symbol: "ETH".into(),
                contract: None,
                token_program: None,
                decimals: Some(18),
                start_block: None,
                end_block: None,
                confirmed_blocks: 12,
                min_amount: None,
                egress: None,
            },
        );
    }

    let app = router(handle);
    // DELETE with uppercase: the handler calls normalize_address_key which
    // lowercases EVM addresses, so the lookup will match the stored lowercase key.
    let delete_uri = format!("{WATCH_PATH}/{upper_send}");

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&delete_uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::NO_CONTENT,
        "DELETE with uppercase EVM address should match lowercase entry (handler normalizes to lowercase)"
    );
}
