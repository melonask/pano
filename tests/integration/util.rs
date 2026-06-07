use pano::config::*;
use pano::detector::util::*;
use pano::model::*;
use pano::shared::amqp::build_amqp_url;
use pano::shared::util::*;
use std::time::Duration;

mod common;

// ── deposit_event_key ─────────────────────────────────────────────────────

fn make_event() -> DepositEvent {
    common::sample_event()
}

#[test]
fn deposit_event_key_format() {
    let event = make_event();
    let key = deposit_event_key(&event);
    let expected = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        event.data.tx_id,
        event.data.caip2,
        event.data.symbol,
        normalize_address_key(&event.data.address),
        event.data.block_number,
        event.data.amount,
        event.data.log_index,
        event.event,
    );
    assert_eq!(key, expected);
}

#[test]
fn deposit_event_key_changes_with_any_component() {
    let e1 = make_event();
    let mut e2 = e1.clone();
    e2.data.tx_id = "0xdifferent".to_string();
    assert_ne!(deposit_event_key(&e1), deposit_event_key(&e2));

    let mut e3 = e1.clone();
    e3.data.block_number = 999;
    assert_ne!(deposit_event_key(&e1), deposit_event_key(&e3));
}

#[test]
fn deposit_event_key_different_event_type() {
    let detected = make_event();
    let confirmed = DepositEvent::confirmed_from(&detected, 12).unwrap();
    assert_ne!(deposit_event_key(&detected), deposit_event_key(&confirmed));
}

// ── deposit_event_key address normalization ───────────────────────────────

#[test]
fn deposit_event_key_evm_lowercased() {
    let event = make_event(); // EVM address is mixed-case in sample_data
    let key = deposit_event_key(&event);
    // The key should contain the lowercased address
    assert!(key.contains("0xabcdef1234567890abcdef1234567890abcdef12"));
}

#[test]
fn deposit_event_key_solana_case_preserved() {
    let mut data = common::sample_data();
    data.caip2 = "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp".to_string();
    data.symbol = "SOL".to_string();
    data.address = "7EcDhSYGxXyscszYEp35KHN8vvw3svAuLKTzXwCFLtV".to_string();
    let event = DepositEvent::detected(data).unwrap();
    let key = deposit_event_key(&event);
    // Solana addresses should NOT be lowercased
    assert!(key.contains("7EcDhSYGxXyscszYEp35KHN8vvw3svAuLKTzXwCFLtV"));
}

// ── mask_rpc_url credentials ──────────────────────────────────────────────

#[test]
fn mask_rpc_url_credentials() {
    let masked = mask_rpc_url("https://user:pass@rpc.example.com/path");
    assert_eq!(masked, "https://***:***@rpc.example.com/path");
}

#[test]
fn mask_rpc_url_no_creds() {
    let masked = mask_rpc_url("https://rpc.example.com/path");
    assert_eq!(masked, "https://rpc.example.com/path");
}

// ── mask_rpc_url secret path segment ──────────────────────────────────────

#[test]
fn mask_rpc_url_long_path_segment() {
    let masked = mask_rpc_url("https://mainnet.infura.io/v3/abcdef1234567890abcdef1234567890");
    assert!(
        masked.contains("***"),
        "expected path segment masked, got: {masked}"
    );
    assert!(
        masked.starts_with("https://mainnet.infura.io/"),
        "got: {masked}"
    );
}

#[test]
fn mask_rpc_url_short_segment_not_masked() {
    let masked = mask_rpc_url("https://rpc.example.com/short");
    assert_eq!(masked, "https://rpc.example.com/short");
}

// ── mask_rpc_url query string ─────────────────────────────────────────────

#[test]
fn mask_rpc_url_query_masked() {
    let masked = mask_rpc_url("https://rpc.example.com/path?apikey=secret");
    assert!(masked.contains("***"), "got: {masked}");
}

// ── mask_rpc_url non-URL ──────────────────────────────────────────────────

#[test]
fn mask_rpc_url_non_url() {
    let masked = mask_rpc_url("not-a-url");
    assert_eq!(masked, "not-a-url");
}

// ── mask_secret_value ─────────────────────────────────────────────────────

#[test]
fn mask_secret_value_nested_object() {
    let mut val = serde_json::json!({
        "config": {
            "api_key": "secret123",
            "nested": {
                "token": "abc"
            }
        }
    });
    mask_secret_value(&mut val);
    assert_eq!(val["config"]["api_key"], "***");
    assert_eq!(val["config"]["nested"]["token"], "***");
}

#[test]
fn mask_secret_value_array() {
    let mut val = serde_json::json!([
        {"token": "abc"},
        {"token": "xyz"}
    ]);
    mask_secret_value(&mut val);
    assert_eq!(val[0]["token"], "***");
    assert_eq!(val[1]["token"], "***");
}

#[test]
fn mask_secret_value_empty_string_not_masked() {
    // Only non-empty strings are masked
    let mut val = serde_json::json!({
        "api_key": ""
    });
    mask_secret_value(&mut val);
    assert_eq!(val["api_key"], "");
}

#[test]
fn mask_secret_value_url_with_credentials() {
    // url::Url adds trailing slash for URLs without a path
    let mut val = serde_json::json!({
        "url": "https://user:pass@rpc.example.com/path"
    });
    mask_secret_value(&mut val);
    assert_eq!(val["url"], "https://***:***@rpc.example.com/path");
}

// ── is_sensitive_key ──────────────────────────────────────────────────────

#[test]
fn is_sensitive_key_true_cases() {
    for key in [
        "api_key",
        "apiKey",
        "API_KEY",
        "token",
        "access_token",
        "TOKEN",
        "secret",
        "client_secret",
        "password",
        "PASSWORD",
        "passphrase",
    ] {
        assert!(is_sensitive_key(key), "expected '{key}' to be sensitive");
    }
}

#[test]
fn is_sensitive_key_false_cases() {
    for key in ["username", "host", "port", "url", "address"] {
        assert!(
            !is_sensitive_key(key),
            "expected '{key}' NOT to be sensitive"
        );
    }
}

// ── build_amqp_url ────────────────────────────────────────────────────────

#[test]
fn build_amqp_url_credentials_in_url_take_precedence() {
    let result = build_amqp_url("amqp://user:pass@host", "other", "p").unwrap();
    assert_eq!(result, "amqp://user:pass@host");
}

#[test]
fn build_amqp_url_separate_credentials() {
    let result = build_amqp_url("amqp://host", "u", "p").unwrap();
    assert_eq!(result, "amqp://u:p@host");
}

#[test]
fn build_amqp_url_password_without_username() {
    let err = build_amqp_url("amqp://host", "", "p").unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("password requires queue username"),
        "got: {msg}"
    );
}

#[test]
fn build_amqp_url_both_empty_unchanged() {
    let result = build_amqp_url("amqp://host", "", "").unwrap();
    assert_eq!(result, "amqp://host");
}

// ── mask_config ───────────────────────────────────────────────────────────

#[test]
fn mask_config_rpc_urls_masked() {
    // Build a minimal config with credentials in the RPC URL
    let toml = r#"
[server]
port = 3210
enabled = true

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["https://user:pass@rpc.example.com/secret"]

[[chains.assets]]
symbol = "ETH"
decimals = 18
"#;
    let mut f = tempfile::NamedTempFile::new().unwrap();
    use std::io::Write;
    f.write_all(toml.as_bytes()).unwrap();
    f.flush().unwrap();
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    let masked = mask_config(&cfg);
    let rpc_url = masked["chains"][0]["rpc"][0].as_str().unwrap();
    assert!(rpc_url.contains("***"), "RPC URL not masked: {rpc_url}");
}

// ── chain_scan_interval ───────────────────────────────────────────────────

#[test]
fn chain_scan_interval_with_value() {
    let chain = common::evm_chain("http://127.0.0.1:8545");
    // The helper sets scan_interval_secs = 1
    let dur = chain_scan_interval(&chain);
    assert_eq!(dur, Duration::from_secs(1));
}

#[test]
fn chain_scan_interval_clamped_to_one() {
    let mut chain = common::evm_chain("http://127.0.0.1:8545");
    chain.rpc_options.as_mut().unwrap().scan_interval_secs = 0;
    let dur = chain_scan_interval(&chain);
    assert_eq!(dur, Duration::from_secs(1));
}

#[test]
fn chain_scan_interval_no_options_default() {
    let mut chain = common::evm_chain("http://127.0.0.1:8545");
    chain.rpc_options = None;
    let dur = chain_scan_interval(&chain);
    assert_eq!(dur, Duration::from_secs(5)); // default is 5
}

// ── min_scan_interval ─────────────────────────────────────────────────────

#[test]
fn min_scan_interval_across_chains() {
    let chain_a = {
        let mut c = common::evm_chain("http://127.0.0.1:8545");
        c.rpc_options.as_mut().unwrap().scan_interval_secs = 2;
        c
    };
    let chain_b = {
        let mut c = common::evm_chain("http://127.0.0.1:8546");
        c.caip2 = "eip155:137".to_string();
        c.rpc_options.as_mut().unwrap().scan_interval_secs = 10;
        c
    };
    let dur = min_scan_interval(&[chain_a, chain_b]);
    assert_eq!(dur, Duration::from_secs(2));
}

#[test]
fn min_scan_interval_empty_returns_default() {
    let dur = min_scan_interval(&[]);
    assert_eq!(dur, Duration::from_secs(5));
}
