use pano::config::*;
use serial_test::serial;
use std::env;
use std::io::Write;
use tempfile::NamedTempFile;

// ── Helpers ──────────────────────────────────────────────────────────────

fn write_temp_toml(content: impl AsRef<str>) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(content.as_ref().as_bytes()).unwrap();
    f.flush().unwrap();
    f
}

/// TOML snippet for a minimal valid chain with one asset.
const MINIMAL_CHAIN: &str = r#"
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18
"#;

/// Server config block needed for every valid config.
const SERVER_BLOCK: &str = r#"
[server]
port = 3210
enabled = true
"#;

fn full_config(chain_section: &str) -> String {
    format!("{SERVER_BLOCK}\n{chain_section}")
}

// ── Basic TOML loading ────────────────────────────────────────────────────

#[test]
fn config_load_happy_path() {
    let toml = full_config(MINIMAL_CHAIN);
    let f = write_temp_toml(&toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert_eq!(cfg.chains.len(), 1);
    assert_eq!(cfg.chains[0].caip2, "eip155:1");
    assert_eq!(cfg.chains[0].confirmed_blocks, 12);
    assert_eq!(cfg.server.port, 3210);
    // Defaults
    assert_eq!(cfg.detector.delivery_workers, 8);
    assert_eq!(cfg.detector.dedup_window_size, 100_000);
    assert_eq!(cfg.chains[0].start_block, None);
    assert_eq!(cfg.chains[0].end_block, None);
    assert_eq!(cfg.chains[0].assets.len(), 1);
}

#[test]
fn config_load_unknown_top_level_keys_accepted() {
    let toml = format!("{}\n[hello]\nworld = true\n", full_config(MINIMAL_CHAIN));
    let f = write_temp_toml(&toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert_eq!(cfg.chains.len(), 1);
}

#[test]
fn config_load_file_not_found() {
    let err = AppConfig::load("nonexistent_file_12345.toml").unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("failed to read config"),
        "expected 'failed to read config', got: {msg}"
    );
}

#[test]
fn config_load_malformed_toml() {
    let f = write_temp_toml("this is not valid toml {{{");
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("failed to parse config"),
        "expected 'failed to parse config', got: {msg}"
    );
}

// ── resolve_env_vars ──────────────────────────────────────────────────────

#[test]
#[serial(env)]
fn resolve_env_vars_single_substitution() {
    unsafe { env::set_var("PANO_TEST_SINGLE", "hello") };
    let result = AppConfig::resolve_env_vars("value = ${PANO_TEST_SINGLE}").unwrap();
    assert_eq!(result, "value = hello");
    unsafe { env::remove_var("PANO_TEST_SINGLE") };
}

#[test]
#[serial(env)]
fn resolve_env_vars_multiple_substitutions() {
    unsafe { env::set_var("PANO_TEST_A1", "one") };
    unsafe { env::set_var("PANO_TEST_A2", "two") };
    let result = AppConfig::resolve_env_vars("${PANO_TEST_A1} and ${PANO_TEST_A2}").unwrap();
    assert_eq!(result, "one and two");
    unsafe { env::remove_var("PANO_TEST_A1") };
    unsafe { env::remove_var("PANO_TEST_A2") };
}

#[test]
#[serial(env)]
fn resolve_env_vars_adjacent() {
    unsafe { env::set_var("PANO_TEST_ADJ1", "hello") };
    unsafe { env::set_var("PANO_TEST_ADJ2", "world") };
    let result = AppConfig::resolve_env_vars("${PANO_TEST_ADJ1}${PANO_TEST_ADJ2}").unwrap();
    assert_eq!(result, "helloworld");
    unsafe { env::remove_var("PANO_TEST_ADJ1") };
    unsafe { env::remove_var("PANO_TEST_ADJ2") };
}

#[test]
#[serial(env)]
fn resolve_env_vars_at_start_and_end() {
    unsafe { env::set_var("PANO_TEST_START", "start") };
    unsafe { env::set_var("PANO_TEST_END", "end") };
    let result = AppConfig::resolve_env_vars("${PANO_TEST_START} middle ${PANO_TEST_END}").unwrap();
    assert_eq!(result, "start middle end");
    unsafe { env::remove_var("PANO_TEST_START") };
    unsafe { env::remove_var("PANO_TEST_END") };
}

#[test]
#[serial(env)]
fn resolve_env_vars_missing_variable() {
    let err = AppConfig::resolve_env_vars("value = ${UNDEFINED_VAR_XYZ}").unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("UNDEFINED_VAR_XYZ") && msg.contains("not set"),
        "expected error about UNDEFINED_VAR_XYZ not set, got: {msg}"
    );
}

#[test]
#[serial(env)]
fn resolve_env_vars_no_substitution_markers() {
    let input = "plain text without any vars";
    let result = AppConfig::resolve_env_vars(input).unwrap();
    assert_eq!(result, input);
}

#[test]
#[serial(env)]
fn resolve_env_vars_invalid_identifier_patterns() {
    // ${0bad} does not match [A-Za-z_][A-Za-z0-9_]*
    let input = "val = ${0bad} and ${a-b}";
    let result = AppConfig::resolve_env_vars(input).unwrap();
    // Should pass through literally
    assert_eq!(result, "val = ${0bad} and ${a-b}");
}

#[test]
#[serial(env)]
fn resolve_env_vars_nested_expansion_not_re_scanned() {
    unsafe { env::set_var("PANO_TEST_NESTED_A", "hello${OTHER}") };
    unsafe { env::set_var("PANO_TEST_NESTED_B", "world") };
    let result = AppConfig::resolve_env_vars("${PANO_TEST_NESTED_A}").unwrap();
    // replacement values are never re-scanned
    assert_eq!(result, "hello${OTHER}");
    unsafe { env::remove_var("PANO_TEST_NESTED_A") };
    unsafe { env::remove_var("PANO_TEST_NESTED_B") };
}

// ── Validation — chains ───────────────────────────────────────────────────

#[test]
fn validation_empty_chains() {
    // Use chains = [] (empty array) at root level so serde parses
    // successfully, then validate() rejects with the expected message.
    // NOTE: chains = [] must appear before [server] in TOML, otherwise
    // it would be interpreted as server.chains = [].
    let toml = format!(
        r#"chains = []
{SERVER_BLOCK}"#
    );
    let f = write_temp_toml(&toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("at least one chain must be configured"),
        "got: {msg}"
    );
}

#[test]
fn validation_duplicate_caip2() {
    let toml = format!(
        r#"{SERVER_BLOCK}
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 6
rpc = ["http://127.0.0.1:8546"]

[[chains.assets]]
symbol = "ETH"
decimals = 18
"#
    );
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("duplicate chain caip2"), "got: {msg}");
}

#[test]
fn validation_empty_rpc_list() {
    let toml = format!(
        r#"{SERVER_BLOCK}
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = []

[[chains.assets]]
symbol = "ETH"
decimals = 18
"#
    );
    let f = write_temp_toml(&toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("has no RPC endpoints"), "got: {msg}");
}

#[test]
fn validation_confirmed_blocks_zero() {
    let toml = format!(
        r#"{SERVER_BLOCK}
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 0
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18
"#
    );
    let f = write_temp_toml(&toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("confirmed_blocks must be greater than 0"),
        "got: {msg}"
    );
}

#[test]
fn validation_start_gt_end_when_end_gt_0() {
    let toml = format!(
        r#"{SERVER_BLOCK}
[[chains]]
caip2 = "eip155:1"
start_block = 200
end_block = 100
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18
"#
    );
    let f = write_temp_toml(&toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("start_block is greater than end_block"),
        "got: {msg}"
    );
}

#[test]
fn validation_start_gt_end_when_end_0_is_valid() {
    let toml = format!(
        r#"{SERVER_BLOCK}
[[chains]]
caip2 = "eip155:1"
start_block = 200
end_block = 0
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18
"#
    );
    let f = write_temp_toml(&toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert_eq!(cfg.chains[0].start_block, Some(200));
    assert_eq!(cfg.chains[0].end_block, Some(0));
}

#[test]
fn validation_non_http_rpc_url() {
    let toml = format!(
        r#"{SERVER_BLOCK}
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["wss://ws.example.com"]

[[chains.assets]]
symbol = "ETH"
decimals = 18
"#
    );
    let f = write_temp_toml(&toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("must use http(s)"), "got: {msg}");
}

#[test]
fn validation_invalid_rpc_url() {
    let toml = format!(
        r#"{SERVER_BLOCK}
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["not a valid url at all"]

[[chains.assets]]
symbol = "ETH"
decimals = 18
"#
    );
    let f = write_temp_toml(&toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("invalid RPC URL"), "got: {msg}");
}

// ── Validation — rpc_options ──────────────────────────────────────────────

/// Build a TOML chain with rpc_options, replacing a specific field with zero.
fn rpc_option_chain_zero(field_name: &str) -> String {
    // Default values for all rpc_options fields that must be > 0
    let defaults: Vec<(&str, &str)> = vec![
        ("max_concurrent", "2"),
        ("batch_size", "10"),
        ("scan_interval_secs", "1"),
        ("scan_timeout_secs", "5"),
        ("max_native_scan_per_cycle", "10"),
        ("request_timeout_secs", "5"),
        ("max_retries", "3"),
        ("retry_base_ms", "100"),
    ];
    let options_lines: String = defaults
        .iter()
        .map(|(name, val)| {
            if *name == field_name {
                format!("{name} = 0\n")
            } else {
                format!("{name} = {val}\n")
            }
        })
        .collect();

    format!(
        r#"{SERVER_BLOCK}
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[chains.rpc_options]
{options_lines}
[[chains.assets]]
symbol = "ETH"
decimals = 18
"#
    )
}

fn assert_rpc_option_error(field_name: &str, expected_error: &str) {
    let toml = rpc_option_chain_zero(field_name);
    let f = write_temp_toml(&toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains(expected_error),
        "expected '{expected_error}' in error, got: {msg}"
    );
}

#[test]
fn validation_max_concurrent_zero() {
    assert_rpc_option_error("max_concurrent", "max_concurrent must be greater than 0");
}

#[test]
fn validation_batch_size_zero() {
    assert_rpc_option_error("batch_size", "batch_size must be greater than 0");
}

#[test]
fn validation_retry_base_ms_zero() {
    assert_rpc_option_error("retry_base_ms", "retry_base_ms must be greater than 0");
}

#[test]
fn validation_request_timeout_secs_zero() {
    assert_rpc_option_error(
        "request_timeout_secs",
        "request_timeout_secs must be greater than 0",
    );
}

#[test]
fn validation_scan_timeout_secs_zero() {
    assert_rpc_option_error(
        "scan_timeout_secs",
        "scan_timeout_secs must be greater than 0",
    );
}

#[test]
fn validation_max_retries_zero() {
    let toml = rpc_option_chain_zero("max_retries");
    let f = write_temp_toml(&toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).expect("max_retries=0 is valid");
    assert_eq!(
        cfg.chains[0]
            .rpc_options
            .as_ref()
            .expect("rpc options")
            .max_retries,
        0
    );
}

// ── Validation — assets ───────────────────────────────────────────────────

#[test]
fn validation_empty_asset_symbol() {
    let toml = full_config(
        r#"
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = ""
decimals = 18
"#,
    );
    let f = write_temp_toml(&toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("empty symbol"), "got: {msg}");
}

#[test]
fn validation_whitespace_asset_symbol() {
    let toml = full_config(
        r#"
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "  "
decimals = 18
"#,
    );
    let f = write_temp_toml(&toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("empty symbol"), "got: {msg}");
}

#[test]
fn validation_decimals_above_max() {
    let toml = full_config(
        r#"
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 31
"#,
    );
    let f = write_temp_toml(&toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("decimals above maximum"), "got: {msg}");
}

#[test]
fn validation_min_amount_zero() {
    let toml = full_config(
        r#"
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18
min_amount = "0"
"#,
    );
    let f = write_temp_toml(&toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("invalid min_amount"), "got: {msg}");
}

#[test]
fn validation_min_amount_leading_zero() {
    let toml = full_config(
        r#"
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18
min_amount = "00"
"#,
    );
    let f = write_temp_toml(&toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("invalid min_amount"), "got: {msg}");
}

#[test]
fn validation_min_amount_negative() {
    let toml = full_config(
        r#"
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18
min_amount = "-5"
"#,
    );
    let f = write_temp_toml(&toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("invalid min_amount"), "got: {msg}");
}

#[test]
fn validation_min_amount_empty() {
    let toml = full_config(
        r#"
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18
min_amount = ""
"#,
    );
    let f = write_temp_toml(&toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("invalid min_amount"), "got: {msg}");
}

#[test]
fn validation_min_amount_one_valid() {
    let toml = full_config(
        r#"
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18
min_amount = "1"
"#,
    );
    let f = write_temp_toml(&toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert_eq!(cfg.chains[0].assets[0].min_amount.as_deref(), Some("1"));
}

// ── Validation — SQL identifiers ──────────────────────────────────────────

#[test]
fn is_valid_sql_identifier_regular() {
    assert!(AppConfig::is_valid_sql_identifier("deposit_events"));
}

#[test]
fn is_valid_sql_identifier_sql_injection() {
    assert!(!AppConfig::is_valid_sql_identifier("deposit; DROP TABLE"));
}

#[test]
fn is_valid_sql_identifier_starts_with_digit() {
    assert!(!AppConfig::is_valid_sql_identifier("2fast"));
}

#[test]
fn is_valid_sql_identifier_underscore_prefix() {
    assert!(AppConfig::is_valid_sql_identifier("_ok"));
}

#[test]
fn is_valid_sql_identifier_64_chars() {
    let s = "a".repeat(64);
    assert!(!AppConfig::is_valid_sql_identifier(&s));
}

#[test]
fn is_valid_sql_identifier_empty() {
    assert!(!AppConfig::is_valid_sql_identifier(""));
}

#[test]
fn is_valid_sql_identifier_mixed_case() {
    assert!(AppConfig::is_valid_sql_identifier("MyTable"));
}

#[test]
fn is_valid_sql_identifier_all_underscores() {
    assert!(AppConfig::is_valid_sql_identifier("___"));
}

#[test]
fn is_valid_sql_identifier_hyphen() {
    assert!(!AppConfig::is_valid_sql_identifier("col-name"));
}

#[test]
fn is_valid_sql_identifier_63_chars() {
    let s = "a".repeat(63);
    assert!(AppConfig::is_valid_sql_identifier(&s));
}

#[test]
fn sql_identifier_leading_underscore() {
    assert!(AppConfig::is_valid_sql_identifier("_col"));
}

#[test]
fn sql_identifier_digit_start() {
    assert!(!AppConfig::is_valid_sql_identifier("1col"));
}

// ── Validation — server ───────────────────────────────────────────────────

#[test]
fn validation_server_disabled_with_http_ingress() {
    let toml = r#"
[server]
enabled = false
port = 0

[ingress.http]
enabled = true
addresses = "watch"

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("server.enabled=false"), "got: {msg}");
    assert!(msg.contains("disable [ingress.http]"), "got: {msg}");
}

#[test]
fn validation_server_port_zero() {
    let toml = r#"
[server]
port = 0
enabled = true

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("server.port must be greater than 0"),
        "got: {msg}"
    );
}

// ── Validation — webhook egress ───────────────────────────────────────────

#[test]
fn validation_webhook_enabled_with_empty_url() {
    let toml = r#"
[server]
port = 3210
enabled = true

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18

[egress.webhook]
enabled = true
url = ""
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("webhook.url is required"), "got: {msg}");
}

#[test]
fn validation_webhook_non_http_url() {
    let toml = r#"
[server]
port = 3210
enabled = true

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18

[egress.webhook]
enabled = true
url = "ftp://example.com/hook"
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("must use http(s)"), "got: {msg}");
}

// ── Validation — file ingress/egress path collision ───────────────────────

#[test]
fn validation_file_path_collision() {
    let toml = r#"
[server]
port = 3210
enabled = true

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18

[ingress.file]
enabled = true
path = "/tmp/same.json"

[egress.file]
enabled = true
path = "/tmp/same.json"
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("must be different"), "got: {msg}");
}

#[test]
fn validation_file_different_paths_ok() {
    let toml = r#"
[server]
port = 3210
enabled = true

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18

[ingress.file]
enabled = true
path = "/tmp/in.json"

[egress.file]
enabled = true
path = "/tmp/out.json"
"#;
    let f = write_temp_toml(toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert_eq!(cfg.ingress.file.path, "/tmp/in.json");
    assert_eq!(cfg.egress.file.path, "/tmp/out.json");
}

#[test]
fn validation_file_same_path_ingress_disabled_ok() {
    let toml = r#"
[server]
port = 3210
enabled = true

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18

[ingress.file]
enabled = false
path = "/tmp/same.json"

[egress.file]
enabled = true
path = "/tmp/same.json"
"#;
    let f = write_temp_toml(toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert!(!cfg.ingress.file.enabled);
}

// ── Validation — detector config ──────────────────────────────────────────

#[test]
fn validation_delivery_workers_zero() {
    let toml = r#"
[server]
port = 3210
enabled = true

[detector]
delivery_workers = 0

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("delivery_workers must be greater than 0"),
        "got: {msg}"
    );
}

#[test]
fn validation_delivery_queue_capacity_zero() {
    let toml = r#"
[server]
port = 3210
enabled = true

[detector]
delivery_queue_capacity = 0

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("delivery_queue_capacity must be greater than 0"),
        "got: {msg}"
    );
}

// ── chain_by_caip2 ────────────────────────────────────────────────────────

#[test]
fn chain_by_caip2_found() {
    let toml = full_config(MINIMAL_CHAIN);
    let f = write_temp_toml(&toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    let chain = cfg.chain_by_caip2("eip155:1");
    assert!(chain.is_some());
    assert_eq!(chain.unwrap().caip2, "eip155:1");
}

#[test]
fn chain_by_caip2_not_found() {
    let toml = full_config(MINIMAL_CHAIN);
    let f = write_temp_toml(&toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert!(cfg.chain_by_caip2("nonexistent:1").is_none());
}

// ── scan_lookback_blocks / scan_interval_secs / max_native_scan_per_cycle ─

/// These rpc_options fields intentionally allow zero — the caller is expected
/// to clamp or default at use sites. The config validator does not reject them.

#[test]
fn scan_lookback_blocks_zero_allowed() {
    let toml = format!(
        r#"{SERVER_BLOCK}
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[chains.rpc_options]
scan_lookback_blocks = 0
max_concurrent = 2
batch_size = 10
scan_interval_secs = 1
max_native_scan_per_cycle = 10
request_timeout_secs = 5
max_retries = 3
retry_base_ms = 100

[[chains.assets]]
symbol = "ETH"
decimals = 18
"#
    );
    let f = write_temp_toml(&toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert_eq!(
        cfg.chains[0]
            .rpc_options
            .as_ref()
            .unwrap()
            .scan_lookback_blocks,
        0
    );
}

#[test]
fn rpc_options_defaults_are_rate_limit_friendly() {
    let defaults = RpcOptions::default();
    assert_eq!(defaults.scan_lookback_blocks, 50);
    assert_eq!(defaults.scan_timeout_secs, 60);
    assert!(defaults.evm_log_address_batching);
}

#[test]
fn solana_effective_default_lookback_uses_slot_safe_value() {
    let chain = ChainConfig {
        caip2: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp".to_string(),
        start_block: None,
        end_block: None,
        confirmed_blocks: 32,
        rpc: vec!["http://127.0.0.1:8899".to_string()],
        rpc_options: None,
        assets: vec![AssetConfig {
            symbol: "SOL".to_string(),
            contract: None,
            token_program: None,
            decimals: 9,
            min_amount: None,
        }],
    };

    assert_eq!(chain.rpc_options_or_default().scan_lookback_blocks, 50);
    assert_eq!(chain.effective_scan_lookback_blocks(), 500);
}

#[test]
fn max_native_scan_per_cycle_zero_allowed() {
    let toml = format!(
        r#"{SERVER_BLOCK}
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[chains.rpc_options]
max_native_scan_per_cycle = 0
max_concurrent = 2
batch_size = 10
scan_lookback_blocks = 0
scan_interval_secs = 1
request_timeout_secs = 5
max_retries = 3
retry_base_ms = 100

[[chains.assets]]
symbol = "ETH"
decimals = 18
"#
    );
    let f = write_temp_toml(&toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert_eq!(
        cfg.chains[0]
            .rpc_options
            .as_ref()
            .unwrap()
            .max_native_scan_per_cycle,
        0
    );
}

#[test]
fn scan_interval_secs_zero_allowed() {
    let toml = format!(
        r#"{SERVER_BLOCK}
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[chains.rpc_options]
scan_interval_secs = 0
max_concurrent = 2
batch_size = 10
scan_lookback_blocks = 0
max_native_scan_per_cycle = 10
request_timeout_secs = 5
max_retries = 3
retry_base_ms = 100

[[chains.assets]]
symbol = "ETH"
decimals = 18
"#
    );
    let f = write_temp_toml(&toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert_eq!(
        cfg.chains[0]
            .rpc_options
            .as_ref()
            .unwrap()
            .scan_interval_secs,
        0
    );
}

// ── Duplicate asset symbol within same chain ──────────────────────────────

#[test]
fn duplicate_asset_symbol_within_same_chain_allowed() {
    // The validator does not check for duplicate asset symbols.
    // Two assets with the same symbol load successfully.
    let toml = format!(
        r#"{SERVER_BLOCK}
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "USDC"
decimals = 6
contract = "0x1111111111111111111111111111111111111111"

[[chains.assets]]
symbol = "USDC"
decimals = 6
contract = "0x2222222222222222222222222222222222222222"
"#
    );
    let f = write_temp_toml(&toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert_eq!(cfg.chains[0].assets.len(), 2);
    assert_eq!(cfg.chains[0].assets[0].symbol, "USDC");
    assert_eq!(cfg.chains[0].assets[1].symbol, "USDC");
}

// ── Missing [server] section ──────────────────────────────────────────────

#[test]
fn missing_server_section_defaults_to_disabled() {
    let toml = r#"
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert!(!cfg.server.enabled, "server should default to disabled");
}

// ── egress.pg configuration validation ────────────────────────────────────

#[test]
fn egress_pg_invalid_url() {
    let toml = r#"
[server]
port = 3210
enabled = true

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18

[egress.pg]
enabled = true
url = "mysql://localhost/db"
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("must start with postgres:// or postgresql://"),
        "got: {msg}"
    );
}

#[test]
fn egress_pg_invalid_table_name() {
    let toml = r#"
[server]
port = 3210
enabled = true

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18

[egress.pg]
enabled = true
url = "postgres://localhost/db"

[egress.pg.table]
name = "events; DROP TABLE"
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("egress.pg.table.name is not a valid SQL identifier"),
        "got: {msg}"
    );
}

#[test]
fn egress_pg_disabled_with_bad_url_loads_fine() {
    // Validation skips egress.pg entirely when enabled = false.
    let toml = r#"
[server]
port = 3210
enabled = true

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18

[egress.pg]
enabled = false
url = "garbage://not-even-a-url"
"#;
    let f = write_temp_toml(toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert!(!cfg.egress.pg.enabled);
}

#[test]
fn egress_pg_valid_url_loads_fine() {
    let toml = r#"
[server]
port = 3210
enabled = true

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18

[egress.pg]
enabled = true
url = "postgres://user:pass@localhost:5432/dbname"
"#;
    let f = write_temp_toml(toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert!(cfg.egress.pg.enabled);
    assert_eq!(
        cfg.egress.pg.url,
        "postgres://user:pass@localhost:5432/dbname"
    );
    // Default table/column names should be populated
    assert_eq!(cfg.egress.pg.table.name, "deposit_events");
    assert_eq!(cfg.egress.pg.table.columns.event_id, "event_id");
}

// ── ingress.* configuration validation ────────────────────────────────────

#[test]
fn ingress_pg_invalid_url() {
    let toml = r#"
[server]
port = 3210
enabled = true

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18

[ingress.pg]
enabled = true
url = "mysql://localhost/db"
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("must start with postgres:// or postgresql://"),
        "got: {msg}"
    );
}

#[test]
fn ingress_pg_invalid_table_name() {
    let toml = r#"
[server]
port = 3210
enabled = true

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18

[ingress.pg]
enabled = true
url = "postgres://localhost/db"

[ingress.pg.table]
name = "1badtable"
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("ingress.pg.table.name is not a valid SQL identifier"),
        "got: {msg}"
    );
}

#[test]
fn ingress_sqlite_invalid_table_name() {
    let toml = r#"
[server]
port = 3210
enabled = true

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18

[ingress.sqlite]
enabled = true
path = "/tmp/watched.db"

[ingress.sqlite.table]
name = "bad name with spaces"
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("ingress.sqlite.table.name is not a valid SQL identifier"),
        "got: {msg}"
    );
}

// ── egress.file.path with nonexistent directory ───────────────────────────

#[test]
fn egress_file_nonexistent_directory_allowed() {
    // The config validator does not check whether the output directory exists.
    // Validation only checks the path string — directory existence is handled
    // at runtime by the file egress writer.
    let toml = r#"
[server]
port = 3210
enabled = true

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["http://127.0.0.1:8545"]

[[chains.assets]]
symbol = "ETH"
decimals = 18

[egress.file]
enabled = true
path = "/nonexistent/directory/out.json"
"#;
    let f = write_temp_toml(toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert_eq!(cfg.egress.file.path, "/nonexistent/directory/out.json");
}
