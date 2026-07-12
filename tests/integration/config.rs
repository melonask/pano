use pano::config::*;
use serial_test::serial;
use std::env;
use std::io::Write;
use tempfile::NamedTempFile;

// ── Helpers for the new namespaced config format ────────────────────────

fn write_temp_toml(content: impl AsRef<str>) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(content.as_ref().as_bytes()).unwrap();
    f.flush().unwrap();
    f
}

/// Minimal valid universal config with one chain and asset.
const MINIMAL_PANO_CONFIG: &str = r#"
[pano]
chains = ["eth"]
assets = ["eth"]

[pano.server]
enabled = false
port = 3210

[pano.ingress.file]
enabled = false

[pano.egress.file]
enabled = false

[chains.eth]
caip2 = "eip155:1"
rpc_urls = ["http://127.0.0.1:8545"]
confirmations = 12

[assets.eth]
chain = "eth"
symbol = "ETH"
decimals = 18
"#;

// ── Basic TOML loading ──────────────────────────────────────────────────

#[test]
fn config_load_happy_path() {
    let f = write_temp_toml(MINIMAL_PANO_CONFIG);
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
fn config_ignore_unrelated_package_namespaces() {
    let toml = format!(
        r#"{}
[ladon]
enabled = true
store = "ladon"

[ladon.derive]
format = "json"

[bria]
enabled = true

[oracles]
enabled = true
"#,
        MINIMAL_PANO_CONFIG
    );
    let f = write_temp_toml(&toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert_eq!(cfg.chains.len(), 1);
}

#[test]
fn config_reject_unknown_pano_field() {
    let toml = format!(
        r#"{}
[pano.unknown_field]
foo = "bar"
"#,
        MINIMAL_PANO_CONFIG
    );
    let f = write_temp_toml(&toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("unknown") || msg.contains("unknown_field"),
        "expected unknown field error, got: {msg}"
    );
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

// ── Missing [pano] section ──────────────────────────────────────────────

#[test]
fn config_load_missing_pano_section() {
    let toml = r#"
[chains.eth]
caip2 = "eip155:1"
rpc_urls = ["http://127.0.0.1:8545"]
confirmations = 12

[assets.eth]
chain = "eth"
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("missing [pano]"), "got: {msg}");
}

// ── resolve_env_vars ────────────────────────────────────────────────────

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
fn resolve_env_vars_with_default_value() {
    let result = AppConfig::resolve_env_vars("value = ${UNDEFINED_VAR:-default123}").unwrap();
    assert_eq!(result, "value = default123");
}

#[test]
#[serial(env)]
fn resolve_env_vars_set_overrides_default() {
    unsafe { env::set_var("PANO_TEST_OVERRIDE", "override") };
    let result = AppConfig::resolve_env_vars("value = ${PANO_TEST_OVERRIDE:-default}").unwrap();
    assert_eq!(result, "value = override");
    unsafe { env::remove_var("PANO_TEST_OVERRIDE") };
}

#[test]
#[serial(env)]
fn resolve_env_vars_empty_default() {
    let result = AppConfig::resolve_env_vars("value = ${UNDEFINED_VAR:-}").unwrap();
    assert_eq!(result, "value = ");
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
    assert_eq!(result, "hello${OTHER}");
    unsafe { env::remove_var("PANO_TEST_NESTED_A") };
    unsafe { env::remove_var("PANO_TEST_NESTED_B") };
}

// ── Validation — chains ──────────────────────────────────────────────────

#[test]
fn validation_empty_chains() {
    let toml = r#"
[pano]
chains = []
assets = []

[pano.server]
enabled = false
port = 3210
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("at least one chain must be configured"),
        "got: {msg}"
    );
}

#[test]
fn validation_duplicate_caip2() {
    let toml = r#"
[pano]
chains = ["eth", "eth2"]
assets = ["eth", "eth2_asset"]

[pano.server]
enabled = false
port = 3210

[chains.eth]
caip2 = "eip155:1"
rpc_urls = ["http://127.0.0.1:8545"]
confirmations = 12

[chains.eth2]
caip2 = "eip155:1"
rpc_urls = ["http://127.0.0.1:8546"]
confirmations = 6

[assets.eth]
chain = "eth"
symbol = "ETH"
decimals = 18

[assets.eth2_asset]
chain = "eth2"
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("duplicate chain caip2") || msg.contains("duplicate"),
        "got: {msg}"
    );
}

#[test]
fn validation_empty_rpc_list() {
    let toml = r#"
[pano]
chains = ["eth"]
assets = ["eth"]

[pano.server]
enabled = false
port = 3210

[chains.eth]
caip2 = "eip155:1"
rpc_urls = []
confirmations = 12

[assets.eth]
chain = "eth"
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("has no RPC endpoints"), "got: {msg}");
}

#[test]
fn validation_confirmed_blocks_zero() {
    let toml = r#"
[pano]
chains = ["eth"]
assets = ["eth"]

[pano.server]
enabled = false
port = 3210

[chains.eth]
caip2 = "eip155:1"
rpc_urls = ["http://127.0.0.1:8545"]
confirmations = 0

[assets.eth]
chain = "eth"
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("confirmed_blocks must be greater than 0"),
        "got: {msg}"
    );
}

// ── Unknown chain/asset references ──────────────────────────────────────

#[test]
fn validation_unknown_chain_ref() {
    let toml = r#"
[pano]
chains = ["nonexistent"]
assets = ["eth"]

[pano.server]
enabled = false
port = 3210

[assets.eth]
chain = "nonexistent"
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("unknown chain"), "got: {msg}");
}

#[test]
fn validation_unknown_asset_ref() {
    let toml = r#"
[pano]
chains = ["eth"]
assets = ["nonexistent"]

[pano.server]
enabled = false
port = 3210

[chains.eth]
caip2 = "eip155:1"
rpc_urls = ["http://127.0.0.1:8545"]
confirmations = 12
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("unknown asset"), "got: {msg}");
}

#[test]
fn validation_rejects_asset_for_unselected_chain() {
    let toml = r#"
[pano]
chains = ["eth"]
assets = ["btc"]

[chains.eth]
caip2 = "eip155:1"
rpc_urls = ["http://127.0.0.1:8545"]
confirmations = 12

[chains.btc]
caip2 = "bip122:000000000019d6689c085ae165831e93"
rpc_urls = ["http://127.0.0.1:8332"]
confirmations = 6

[assets.btc]
chain = "btc"
symbol = "BTC"
decimals = 8
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    assert!(format!("{err:#}").contains("not listed in pano.chains"));
}

#[test]
fn validation_rejects_sqlite_store_with_wrong_driver() {
    let toml = format!(
        r#"{}
[pano.egress.sqlite]
enabled = true
store = "postgres"

[stores.postgres]
driver = "postgres"
url = "postgres://localhost/pano"
"#,
        MINIMAL_PANO_CONFIG
    );
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    assert!(format!("{err:#}").contains("\"sqlite\" is required"));
}

// ── Shared chain and asset resolution ────────────────────────────────────

#[test]
fn shared_chain_resolution_rpc_options() {
    let toml = r#"
[pano]
chains = ["eth"]
assets = ["eth"]

[pano.server]
enabled = false
port = 3210

[pano.rpc_defaults]
max_concurrent = 5
scan_interval_secs = 10

[chains.eth]
caip2 = "eip155:1"
rpc_urls = ["http://127.0.0.1:8545"]
confirmations = 12

[assets.eth]
chain = "eth"
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    let rpc = cfg.chains[0].rpc_options.as_ref().unwrap();
    assert_eq!(rpc.max_concurrent, 5);
    assert_eq!(rpc.scan_interval_secs, 10);
    assert_eq!(cfg.chains[0].confirmed_blocks, 12);
    assert_eq!(cfg.chains[0].rpc[0], "http://127.0.0.1:8545");
}

#[test]
fn shared_asset_resolution_maps_to_chain() {
    let toml = r#"
[pano]
chains = ["eth"]
assets = ["eth", "usdc"]

[pano.server]
enabled = false
port = 3210

[chains.eth]
caip2 = "eip155:1"
rpc_urls = ["http://127.0.0.1:8545"]
confirmations = 12

[assets.eth]
chain = "eth"
symbol = "ETH"
decimals = 18

[assets.usdc]
chain = "eth"
symbol = "USDC"
contract = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
decimals = 6
"#;
    let f = write_temp_toml(toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert_eq!(cfg.chains[0].assets.len(), 2);
    assert_eq!(cfg.chains[0].assets[0].symbol, "ETH");
    assert_eq!(cfg.chains[0].assets[1].symbol, "USDC");
    assert_eq!(
        cfg.chains[0].assets[1].contract.as_deref(),
        Some("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48")
    );
}

// ── Shared path resolution ──────────────────────────────────────────────

#[test]
fn shared_path_resolution_file_ingress() {
    let toml = r#"
[pano]
chains = ["eth"]
assets = ["eth"]

[pano.server]
enabled = false
port = 3210

[pano.ingress.file]
enabled = true
path_ref = "my_watches"

[pano.egress.file]
enabled = true
path_ref = "my_events"

[paths.my_watches]
kind = "file"
path = "data/test/watches.jsonl"

[paths.my_events]
kind = "file"
path = "data/test/events.jsonl"

[chains.eth]
caip2 = "eip155:1"
rpc_urls = ["http://127.0.0.1:8545"]
confirmations = 12

[assets.eth]
chain = "eth"
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert_eq!(cfg.ingress.file.path, "data/test/watches.jsonl");
    assert_eq!(cfg.egress.file.path, "data/test/events.jsonl");
    assert!(cfg.ingress.file.enabled);
    assert!(cfg.egress.file.enabled);
}

#[test]
fn config_rejects_direct_file_path_override() {
    let toml = r#"
[pano]
chains = ["eth"]
assets = ["eth"]

[pano.server]
enabled = false
port = 3210

[pano.ingress.file]
enabled = true
path_ref = "my_watches"
path = "/override/path.jsonl"

[pano.egress.file]
enabled = false

[paths.my_watches]
kind = "file"
path = "data/test/watches.jsonl"

[chains.eth]
caip2 = "eip155:1"
rpc_urls = ["http://127.0.0.1:8545"]
confirmations = 12

[assets.eth]
chain = "eth"
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    assert!(format!("{err:#}").contains("unknown field"));
}

#[test]
fn config_rejects_unsupported_file_format_and_template() {
    let configs = [
        format!(
            "{MINIMAL_PANO_CONFIG}\n[paths.my_watches]\nkind = \"file\"\npath = \"data/test/watches.jsonl\"\nformat = \"jsonl\""
        ),
        MINIMAL_PANO_CONFIG.replace(
            "[pano.egress.file]\nenabled = false",
            "[pano.egress.file]\nenabled = false\ntemplate = \"{event}\"",
        ),
    ];
    for toml in configs {
        let f = write_temp_toml(toml);
        let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
        assert!(format!("{err:#}").contains("unknown field"));
    }
}

#[test]
fn unknown_path_ref_fails() {
    let toml = r#"
[pano]
chains = ["eth"]
assets = ["eth"]

[pano.server]
enabled = false
port = 3210

[pano.ingress.file]
enabled = true
path_ref = "missing_path"

[pano.egress.file]
enabled = false

[chains.eth]
caip2 = "eip155:1"
rpc_urls = ["http://127.0.0.1:8545"]
confirmations = 12

[assets.eth]
chain = "eth"
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("unknown path_ref"), "got: {msg}");
}

// ── Transport resolution ─────────────────────────────────────────────────

#[cfg(feature = "amqp")]
#[test]
fn shared_amqp_transport_resolution() {
    let toml = r#"
[pano]
chains = ["eth"]
assets = ["eth"]

[pano.server]
enabled = false
port = 3210

[pano.ingress.file]
enabled = false

[pano.egress.file]
enabled = false

[pano.egress.amqp]
enabled = true
transport = "my_broker"
exchange = "test.exchange"
detected_routing_key = "test.detected"
confirmed_routing_key = "test.confirmed"

[transports.amqp.my_broker]
url = "amqp://test-broker:5672"
username = "testuser"
password = "testpass"
reconnect_secs = 10
qos_prefetch = 50

[chains.eth]
caip2 = "eip155:1"
rpc_urls = ["http://127.0.0.1:8545"]
confirmations = 12

[assets.eth]
chain = "eth"
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert!(cfg.egress.queue.enabled);
    assert_eq!(cfg.egress.queue.url, "amqp://test-broker:5672");
    assert_eq!(cfg.egress.queue.username, "testuser");
    assert_eq!(cfg.egress.queue.reconnect_secs, 10);
    assert_eq!(cfg.egress.queue.exchange, "test.exchange");
}

#[cfg(feature = "amqp")]
#[test]
fn unknown_amqp_transport_fails() {
    let toml = r#"
[pano]
chains = ["eth"]
assets = ["eth"]

[pano.server]
enabled = false
port = 3210

[pano.ingress.file]
enabled = false

[pano.egress.file]
enabled = false

[pano.egress.amqp]
enabled = true
transport = "nonexistent"
exchange = "test.exchange"

[chains.eth]
caip2 = "eip155:1"
rpc_urls = ["http://127.0.0.1:8545"]
confirmations = 12

[assets.eth]
chain = "eth"
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("unknown AMQP transport"), "got: {msg}");
}

// ── Feature-gate checks ─────────────────────────────────────────────────

#[cfg(feature = "postgres")]
#[test]
fn postgres_disabled_fails_with_clear_error() {
    // This test verifies that when postgres feature is not enabled,
    // referencing pg config fails. But since we compile tests with
    // --features full, this test checks the opposite: with postgres
    // enabled, pg config should load.
    let toml = r#"
[pano]
chains = ["eth"]
assets = ["eth"]

[pano.server]
enabled = false
port = 3210

[pano.ingress.file]
enabled = false

[pano.egress.file]
enabled = false

[pano.egress.pg]
enabled = true
store = "my_pg"

[stores.my_pg]
driver = "postgres"
url = "postgres://localhost/test"

[chains.eth]
caip2 = "eip155:1"
rpc_urls = ["http://127.0.0.1:8545"]
confirmations = 12

[assets.eth]
chain = "eth"
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    // With full features enabled, this should load (feature check passes,
    // though the URL won't be validated for connectivity)
    let result = AppConfig::load(f.path().to_str().unwrap());
    assert!(
        result.is_ok(),
        "pg config should load with postgres feature: {result:?}"
    );
}

#[test]
fn sqlite_default_enabled() {
    let toml = r#"
[pano]
chains = ["eth"]
assets = ["eth"]

[pano.server]
enabled = false
port = 3210

[pano.ingress.file]
enabled = false

[pano.egress.file]
enabled = false

[pano.egress.sqlite]
enabled = true
store = "my_sqlite"

[stores.my_sqlite]
driver = "sqlite"
url = "sqlite:///tmp/test.db"

[chains.eth]
caip2 = "eip155:1"
rpc_urls = ["http://127.0.0.1:8545"]
confirmations = 12

[assets.eth]
chain = "eth"
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert!(cfg.egress.sqlite.enabled);
    assert_eq!(cfg.egress.sqlite.path, "/tmp/test.db");
}

// ── chain_by_caip2 ──────────────────────────────────────────────────────

#[test]
fn chain_by_caip2_found() {
    let f = write_temp_toml(MINIMAL_PANO_CONFIG);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    let chain = cfg.chain_by_caip2("eip155:1");
    assert!(chain.is_some());
    assert_eq!(chain.unwrap().caip2, "eip155:1");
}

#[test]
fn chain_by_caip2_not_found() {
    let f = write_temp_toml(MINIMAL_PANO_CONFIG);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert!(cfg.chain_by_caip2("nonexistent:1").is_none());
}

// ── Solana effective default lookback ────────────────────────────────────

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

// ── SQL identifiers ─────────────────────────────────────────────────────

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

// ── Env expansion in config loading ─────────────────────────────────────

#[test]
#[serial(env)]
fn env_expansion_in_config_with_default() {
    // Test that ${VAR:-default} works inside actual config values
    let toml = r#"
[pano]
chains = ["eth"]
assets = ["eth"]

[pano.server]
enabled = false
port = 3210

[pano.egress.file]
enabled = false

[pano.ingress.file]
enabled = false

[chains.eth]
caip2 = "${TEST_CAIP2:-eip155:1}"
rpc_urls = ["http://127.0.0.1:8545"]
confirmations = 12

[assets.eth]
chain = "eth"
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert_eq!(cfg.chains[0].caip2, "eip155:1");
}

#[test]
#[serial(env)]
fn env_expansion_with_set_variable() {
    unsafe { env::set_var("PANO_TEST_CAIP2", "eip155:999") };
    let toml = r#"
[pano]
chains = ["eth"]
assets = ["eth"]

[pano.server]
enabled = false
port = 3210

[pano.egress.file]
enabled = false

[pano.ingress.file]
enabled = false

[chains.eth]
caip2 = "${PANO_TEST_CAIP2}"
rpc_urls = ["http://127.0.0.1:8545"]
confirmations = 12

[assets.eth]
chain = "eth"
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let cfg = AppConfig::load(f.path().to_str().unwrap()).unwrap();
    assert_eq!(cfg.chains[0].caip2, "eip155:999");
    unsafe { env::remove_var("PANO_TEST_CAIP2") };
}

#[test]
#[serial(env)]
fn env_expansion_missing_without_default_fails() {
    let toml = r#"
[pano]
chains = ["eth"]
assets = ["eth"]

[pano.server]
enabled = false
port = 3210

[pano.egress.file]
enabled = false

[pano.ingress.file]
enabled = false

[chains.eth]
caip2 = "${MISSING_VAR_XYZ}"
rpc_urls = ["http://127.0.0.1:8545"]
confirmations = 12

[assets.eth]
chain = "eth"
symbol = "ETH"
decimals = 18
"#;
    let f = write_temp_toml(toml);
    let err = AppConfig::load(f.path().to_str().unwrap()).unwrap_err();
    let msg = format!("{err:?}");
    assert!(
        msg.contains("MISSING_VAR_XYZ") && msg.contains("not set"),
        "expected missing env var error, got: {msg}"
    );
}
