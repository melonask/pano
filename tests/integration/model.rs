use pano::model::*;

mod common;

// ── ChainKind::from_caip2 ─────────────────────────────────────────────────

#[test]
fn chain_kind_from_caip2_evm() {
    assert_eq!(ChainKind::from_caip2("eip155:1"), Some(ChainKind::Evm));
    assert_eq!(ChainKind::from_caip2("eip155:137"), Some(ChainKind::Evm));
}

#[test]
fn chain_kind_from_caip2_solana() {
    assert_eq!(
        ChainKind::from_caip2("solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp"),
        Some(ChainKind::Solana)
    );
}

#[test]
fn chain_kind_from_caip2_bitcoin() {
    assert_eq!(
        ChainKind::from_caip2("bip122:000000000019d6689c085ae165831e93"),
        Some(ChainKind::Bitcoin)
    );
}

#[test]
fn chain_kind_from_caip2_unknown() {
    assert_eq!(ChainKind::from_caip2("cosmos:cosmoshub-4"), None);
}

#[test]
fn chain_kind_from_caip2_empty() {
    assert_eq!(ChainKind::from_caip2(""), None);
}

#[test]
fn chain_kind_from_caip2_no_colon() {
    // The code uses split(':').next() so a string without colon matches
    assert_eq!(ChainKind::from_caip2("eip155"), Some(ChainKind::Evm));
}

#[test]
fn chain_kind_from_caip2_case_sensitive() {
    // Namespace match is case-sensitive; "EIP155" != "eip155".
    // Callers supplying uppercase namespace will have the chain silently dropped.
    assert_eq!(ChainKind::from_caip2("EIP155:1"), None);
}

// ── validate_address_for_chain ────────────────────────────────────────────

#[test]
fn validate_evm_valid() {
    assert!(validate_address_for_chain(
        ChainKind::Evm,
        "0xAbCdEf1234567890AbCdEf1234567890AbCdEf12"
    ));
}

#[test]
fn validate_evm_uppercase_hex() {
    // EVM validator does NOT to_lowercase before checking is_ascii_hexdigit(),
    // but is_ascii_hexdigit accepts both cases so this is fine.
    assert!(validate_address_for_chain(
        ChainKind::Evm,
        "0xABCDEF1234567890ABCDEF1234567890ABCDEF12"
    ));
}

#[test]
fn validate_evm_too_short() {
    assert!(!validate_address_for_chain(
        ChainKind::Evm,
        "0xAbCdEf1234567890AbCdEf1234567890AbCdEf1"
    ));
}

#[test]
fn validate_evm_too_long() {
    assert!(!validate_address_for_chain(
        ChainKind::Evm,
        "0xAbCdEf1234567890AbCdEf1234567890AbCdEf123"
    ));
}

#[test]
fn validate_evm_no_prefix() {
    assert!(!validate_address_for_chain(
        ChainKind::Evm,
        "AbCdEf1234567890AbCdEf1234567890AbCdEf12"
    ));
}

#[test]
fn validate_evm_non_hex() {
    assert!(!validate_address_for_chain(
        ChainKind::Evm,
        "0xGHIJKL1234567890GHIJKL1234567890GHIJKL12"
    ));
}

// Solana tests

#[test]
fn validate_solana_valid() {
    // Valid base58 44-char key
    assert!(validate_address_for_chain(
        ChainKind::Solana,
        "7EcDhSYGxXyscszYEp35KHN8vvw3svAuLKTzXwCFLtV"
    ));
}

#[test]
fn validate_solana_invalid_chars_zero() {
    // '0' (zero) is not in base58 alphabet
    assert!(!validate_address_for_chain(
        ChainKind::Solana,
        "7EcDhSYGxXyscszYEp35KHN8vvw3svAuLKTzXwCfLt0"
    ));
}

#[test]
fn validate_solana_invalid_chars_uppercase_o() {
    // 'O' (uppercase O) is not in base58 alphabet
    assert!(!validate_address_for_chain(
        ChainKind::Solana,
        "7EcDhSYGxXyscszYEp35KHN8vvw3svAuLKTzXwCFLtO"
    ));
}

#[test]
fn validate_solana_invalid_chars_uppercase_i() {
    // 'I' (uppercase I) is not in base58 alphabet
    assert!(!validate_address_for_chain(
        ChainKind::Solana,
        "7EcDhSYGxXyscszYEp35KHN8vvw3svAuLKTzXwCfLtI"
    ));
}

#[test]
fn validate_solana_invalid_chars_lowercase_l() {
    // 'l' (lowercase l) is not in base58 alphabet
    assert!(!validate_address_for_chain(
        ChainKind::Solana,
        "7EcDhSYGxXyscszYEp35KHN8vvw3svAuLKTzXwCfLtl"
    ));
}

#[test]
fn validate_solana_too_short() {
    assert!(!validate_address_for_chain(
        ChainKind::Solana,
        "7EcDhSYGxXyscszYEp35KHN8vvw3"
    ));
}

#[test]
fn validate_solana_too_long() {
    // Valid base58 address is 44 chars max; append two chars to reach 45
    assert!(!validate_address_for_chain(
        ChainKind::Solana,
        "7EcDhSYGxXyscszYEp35KHN8vvw3svAuLKTzXwCFLtVxx"
    ));
}

// Bitcoin tests

#[test]
fn validate_bitcoin_p2pkh() {
    assert!(validate_address_for_chain(
        ChainKind::Bitcoin,
        "1A1zP1eP5QGefi2DMPTfTL5SLmv7Divf"
    ));
}

#[test]
fn validate_bitcoin_p2sh() {
    assert!(validate_address_for_chain(
        ChainKind::Bitcoin,
        "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy"
    ));
}

#[test]
fn validate_bitcoin_bech32_mainnet() {
    assert!(validate_address_for_chain(
        ChainKind::Bitcoin,
        "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
    ));
}

#[test]
fn validate_bitcoin_bech32_testnet() {
    assert!(validate_address_for_chain(
        ChainKind::Bitcoin,
        "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"
    ));
}

#[test]
fn validate_bitcoin_regtest() {
    assert!(validate_address_for_chain(
        ChainKind::Bitcoin,
        "bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"
    ));
}

#[test]
fn validate_bitcoin_legacy_too_short() {
    assert!(!validate_address_for_chain(
        ChainKind::Bitcoin,
        "1A1zP1eP5QGefi2DMPTf"
    ));
}

#[test]
fn validate_bitcoin_legacy_too_long() {
    assert!(!validate_address_for_chain(
        ChainKind::Bitcoin,
        "1A1zP1eP5QGefi2DMPTfTL5SLmv7Divf1234"
    ));
}

#[test]
fn validate_bitcoin_bech32_no_payload() {
    assert!(!validate_address_for_chain(ChainKind::Bitcoin, "bc1"));
}

#[test]
fn validate_bitcoin_bech32_invalid_chars() {
    assert!(!validate_address_for_chain(
        ChainKind::Bitcoin,
        "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdb"
    )); // 'b' is not in bech32 charset
}

// ── normalize_address_key ─────────────────────────────────────────────────

#[test]
fn normalize_evm_mixed_case() {
    assert_eq!(
        normalize_address_key("0xAbCdEf1234567890AbCdEf1234567890AbCdEf12"),
        "0xabcdef1234567890abcdef1234567890abcdef12"
    );
}

#[test]
fn normalize_evm_with_whitespace() {
    assert_eq!(
        normalize_address_key("  0xAbCdEf1234567890AbCdEf1234567890AbCdEf12  "),
        "0xabcdef1234567890abcdef1234567890abcdef12"
    );
}

#[test]
fn normalize_bech32_mainnet() {
    assert_eq!(
        normalize_address_key("bc1QAR0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"),
        "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
    );
}

#[test]
fn normalize_bech32_testnet() {
    assert_eq!(
        normalize_address_key("tb1Qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"),
        "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"
    );
}

#[test]
fn normalize_bech32_regtest() {
    assert_eq!(
        normalize_address_key("bcrt1Qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"),
        "bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"
    );
}

#[test]
fn normalize_solana_case_preserved() {
    assert_eq!(
        normalize_address_key("7EcDhSYGxXyscszYEp35KHN8vvw3svAuLKTzXwCFLtV"),
        "7EcDhSYGxXyscszYEp35KHN8vvw3svAuLKTzXwCFLtV"
    );
}

#[test]
fn normalize_bitcoin_legacy_case_preserved() {
    assert_eq!(
        normalize_address_key("1A1zP1eP5QGefi2DMPTfTL5SLmv7Divf"),
        "1A1zP1eP5QGefi2DMPTfTL5SLmv7Divf"
    );
}

#[test]
fn normalize_bitcoin_p2sh_case_preserved() {
    // P2SH addresses starting with "3" are not lowercased
    assert_eq!(
        normalize_address_key("3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy"),
        "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy"
    );
}

#[test]
fn normalize_empty_string() {
    assert_eq!(normalize_address_key(""), "");
}

#[test]
fn normalize_whitespace_only() {
    assert_eq!(normalize_address_key("   "), "");
}

// ── DepositEvent creation ─────────────────────────────────────────────────

#[test]
fn deposit_event_detected() {
    let data = common::sample_data();
    let event = DepositEvent::detected(data).unwrap();
    assert_eq!(event.event, "pano.deposit.detected");
    assert_eq!(event.version, 1);
    assert!(!event.event_id.is_empty());
    // ULID is 26 chars
    assert_eq!(event.event_id.len(), 26);
    // occurred_at is RFC3339
    assert!(event.occurred_at.contains('T'));
    assert!(event.occurred_at.ends_with('Z'));
}

#[test]
fn deposit_event_confirmed_from() {
    let detected = common::sample_event();
    let confirmed = DepositEvent::confirmed_from(&detected, 12).unwrap();
    assert_eq!(confirmed.event, "pano.deposit.confirmed");
    assert_eq!(confirmed.data.confirmations, 12);
    assert_eq!(confirmed.data.tx_id, detected.data.tx_id);
    assert_eq!(confirmed.data.amount, detected.data.amount);
}

#[test]
fn deposit_event_status_detected() {
    let event = common::sample_event();
    assert_eq!(event.status(), DepositStatus::Detected);
}

#[test]
fn deposit_event_status_confirmed() {
    let detected = common::sample_event();
    let confirmed = DepositEvent::confirmed_from(&detected, 6).unwrap();
    assert_eq!(confirmed.status(), DepositStatus::Confirmed);
}

#[test]
fn deposit_event_status_unknown_falls_back_to_detected() {
    // Create event with unknown event string via serde deserialization
    let json = r#"{
        "event_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
        "event": "pano.deposit.unknown",
        "version": 1,
        "occurred_at": "2026-06-04T00:00:00Z",
        "data": {
            "tx_id": "0xtx",
            "caip2": "eip155:1",
            "symbol": "ETH",
            "address": "0xabcdef1234567890abcdef1234567890abcdef12",
            "block_number": 123,
            "log_index": 0,
            "amount": "1000000000000000000",
            "sender": "0x1111111111111111111111111111111111111111",
            "confirmations": 1,
            "timestamp": "2026-06-04T00:00:00Z"
        }
    }"#;
    let event: DepositEvent = serde_json::from_str(json).unwrap();
    assert_eq!(event.status(), DepositStatus::Detected);
}

// ── Deposit amount validation (via DepositEvent::detected) ────────────────

#[test]
fn deposit_amount_zero_err() {
    let mut data = common::sample_data();
    data.amount = "0".to_string();
    let err = DepositEvent::detected(data).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("positive integer"), "got: {msg}");
}

#[test]
fn deposit_amount_leading_zeros_err() {
    let mut data = common::sample_data();
    data.amount = "00".to_string();
    let err = DepositEvent::detected(data).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("leading zeros"), "got: {msg}");
}

#[test]
fn deposit_amount_leading_zero_01_err() {
    let mut data = common::sample_data();
    data.amount = "01".to_string();
    let err = DepositEvent::detected(data).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("leading zeros"), "got: {msg}");
}

#[test]
fn deposit_amount_negative_err() {
    let mut data = common::sample_data();
    data.amount = "-1".to_string();
    let err = DepositEvent::detected(data).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("non-empty digit string"), "got: {msg}");
}

#[test]
fn deposit_amount_empty_err() {
    let mut data = common::sample_data();
    data.amount = "".to_string();
    let err = DepositEvent::detected(data).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("non-empty"), "got: {msg}");
}

#[test]
fn deposit_amount_one_ok() {
    let mut data = common::sample_data();
    data.amount = "1".to_string();
    let event = DepositEvent::detected(data).unwrap();
    assert_eq!(event.data.amount, "1");
}

#[test]
fn deposit_amount_large_ok() {
    let mut data = common::sample_data();
    data.amount = "999999999999999999999999999999".to_string();
    let event = DepositEvent::detected(data).unwrap();
    assert_eq!(event.data.amount, "999999999999999999999999999999");
}

#[test]
fn deposit_amount_dot_err() {
    let mut data = common::sample_data();
    data.amount = "1.5".to_string();
    let err = DepositEvent::detected(data).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("non-empty digit string"), "got: {msg}");
}

#[test]
fn deposit_amount_hex_err() {
    let mut data = common::sample_data();
    data.amount = "0x1f".to_string();
    let err = DepositEvent::detected(data).unwrap_err();
    let msg = format!("{err:?}");
    assert!(msg.contains("non-empty digit string"), "got: {msg}");
}

// ── WatchSpec serialization round-trip ────────────────────────────────────

#[test]
fn watch_spec_minimal_round_trip() {
    let json = r#"{"address":"0xabcdef1234567890abcdef1234567890abcdef12"}"#;
    let spec: WatchSpec = serde_json::from_str(json).unwrap();
    let serialized = serde_json::to_string(&spec).unwrap();
    let round_trip: WatchSpec = serde_json::from_str(&serialized).unwrap();
    assert_eq!(spec, round_trip);
}

#[test]
fn watch_spec_full_round_trip() {
    let json = r#"{
        "address": "0xabcdef1234567890abcdef1234567890abcdef12",
        "chains": [{
            "caip2": "eip155:1",
            "address": "0x2222222222222222222222222222222222222222",
            "start_block": 100,
            "end_block": 200,
            "confirmed_blocks": 24,
            "assets": [{
                "symbol": "USDC",
                "contract": "0x3333333333333333333333333333333333333333",
                "decimals": 6,
                "min_amount": "1000"
            }]
        }],
        "egress": {
            "webhook": {
                "url": "https://example.com/hook",
                "secret": "my-secret"
            },
            "file": {
                "path": "/tmp/out.json"
            }
        }
    }"#;
    let spec: WatchSpec = serde_json::from_str(json).unwrap();
    let serialized = serde_json::to_string(&spec).unwrap();
    let round_trip: WatchSpec = serde_json::from_str(&serialized).unwrap();
    assert_eq!(spec, round_trip);
}

#[test]
fn watch_spec_unknown_field() {
    let json = r#"{"address":"0xabcdef1234567890abcdef1234567890abcdef12","xyz":true}"#;
    let err = serde_json::from_str::<WatchSpec>(json).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("unknown field") || msg.contains("xyz"),
        "got: {msg}"
    );
}

#[test]
fn chain_entry_unknown_field() {
    let json = r#"{
        "address": "0xabcdef1234567890abcdef1234567890abcdef12",
        "chains": [{"caip2": "eip155:1", "rpc": []}]
    }"#;
    let err = serde_json::from_str::<WatchSpec>(json).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("unknown field") || msg.contains("rpc"),
        "got: {msg}"
    );
}

#[test]
fn asset_entry_unknown_field() {
    let json = r#"{
        "chains": [{"caip2": "eip155:1", "assets": [{"symbol":"X","xyz":true}]}]
    }"#;
    let err = serde_json::from_str::<WatchSpec>(json).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("unknown field") || msg.contains("xyz"),
        "got: {msg}"
    );
}

#[test]
fn egress_override_unknown_field() {
    let json = r#"{
        "address": "0xabcdef1234567890abcdef1234567890abcdef12",
        "egress": {"webhook": {"url": "https://x.com", "xyz": true}}
    }"#;
    let err = serde_json::from_str::<WatchSpec>(json).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("unknown field") || msg.contains("xyz"),
        "got: {msg}"
    );
}

// ── DepositEvent serialization/deserialization round-trip ─────────────────

#[test]
fn deposit_event_round_trip() {
    let event = common::sample_event();
    let json = serde_json::to_string_pretty(&event).unwrap();
    let round_trip: DepositEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(event.event_id, round_trip.event_id);
    assert_eq!(event.event, round_trip.event);
    assert_eq!(event.version, round_trip.version);
    assert_eq!(event.occurred_at, round_trip.occurred_at);
    assert_eq!(event.data.tx_id, round_trip.data.tx_id);
    assert_eq!(event.data.caip2, round_trip.data.caip2);
    assert_eq!(event.data.symbol, round_trip.data.symbol);
    assert_eq!(event.data.address, round_trip.data.address);
    assert_eq!(event.data.block_number, round_trip.data.block_number);
    assert_eq!(event.data.log_index, round_trip.data.log_index);
    assert_eq!(event.data.amount, round_trip.data.amount);
    assert_eq!(event.data.sender, round_trip.data.sender);
    assert_eq!(event.data.confirmations, round_trip.data.confirmations);
    assert_eq!(event.data.timestamp, round_trip.data.timestamp);
    // internal_egress is #[serde(skip)] so it is always None after deserialization
    assert_eq!(round_trip.data.internal_egress, None);
}

#[test]
fn deposit_event_internal_egress_skipped_in_json() {
    // internal_egress is #[serde(skip)] — it never appears in JSON output.
    // Verify that a populated internal_egress is preserved in memory but
    // excluded from serialization.
    let mut data = common::sample_data();
    data.internal_egress = Some(EgressOverride {
        webhook: Some(WebhookOverride {
            url: "https://hook.example.com".to_string(),
            secret: "s3cret".to_string(),
        }),
        ..Default::default()
    });
    let event = DepositEvent::detected(data).unwrap();
    // Verify internal_egress is populated in memory
    assert!(event.data.internal_egress.is_some());
    // Serialize — internal_egress should NOT appear in JSON
    let json = serde_json::to_string_pretty(&event).unwrap();
    assert!(!json.contains("internal_egress"));
    assert!(!json.contains("hook.example.com"));
    // Deserialize — internal_egress is None (data was not serialized)
    let round_trip: DepositEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(round_trip.data.internal_egress, None);
}

// ── Empty address in DepositData ──────────────────────────────────────────

#[test]
fn empty_address_in_deposit_data_accepted() {
    // DepositEvent::new only validates the amount field.
    // An empty address is not validated and passes through.
    let mut data = common::sample_data();
    data.address = String::new();
    let event = DepositEvent::detected(data).unwrap();
    assert_eq!(event.data.address, "");
    assert!(!event.event_id.is_empty());
}

// ── EgressOverride with all fields populated ──────────────────────────────

#[test]
fn egress_override_all_fields_round_trip() {
    let egress = EgressOverride {
        webhook: Some(WebhookOverride {
            url: "https://hooks.example.com/deposits".to_string(),
            secret: "wh-secret-12345".to_string(),
        }),
        file: Some(FileOverride {
            path: "/tmp/custom-out.json".to_string(),
        }),
        pg: Some(PgOverride {
            url: "postgres://user:pass@pg.example.com/db".to_string(),
            table: Some(TableOverride {
                name: "custom_deposits".to_string(),
            }),
        }),
        sqlite: Some(SqliteOverride {
            path: "/data/deposits.db".to_string(),
            table: Some(TableOverride {
                name: "custom_events".to_string(),
            }),
        }),
        queue: Some(QueueOverride {
            url: "amqp://broker.example.com".to_string(),
            exchange: Some("pano.custom".to_string()),
            username: Some("user".to_string()),
            password: Some("pass".to_string()),
        }),
        http: Some(HttpOverride {
            sse: Some("/events".to_string()),
            websocket: Some("/ws".to_string()),
        }),
    };
    let json = serde_json::to_string_pretty(&egress).unwrap();
    let round_trip: EgressOverride = serde_json::from_str(&json).unwrap();
    assert_eq!(egress, round_trip);
    // Spot-check a few nested fields
    assert_eq!(
        round_trip.webhook.as_ref().unwrap().url,
        "https://hooks.example.com/deposits"
    );
    assert_eq!(
        round_trip.pg.as_ref().unwrap().url,
        "postgres://user:pass@pg.example.com/db"
    );
    assert_eq!(
        round_trip.pg.as_ref().unwrap().table.as_ref().unwrap().name,
        "custom_deposits"
    );
    assert_eq!(
        round_trip.queue.as_ref().unwrap().exchange.as_deref(),
        Some("pano.custom")
    );
}

// ── WatchSpec with multiple chains for the same address ───────────────────

#[test]
fn watch_spec_multiple_chains_same_address() {
    let json = r#"{
        "address": "0xabcdef1234567890abcdef1234567890abcdef12",
        "chains": [
            {
                "caip2": "eip155:1",
                "address": "0xabcdef1234567890abcdef1234567890abcdef12",
                "start_block": 100
            },
            {
                "caip2": "eip155:137",
                "address": "0xabcdef1234567890abcdef1234567890abcdef12",
                "start_block": 200
            }
        ]
    }"#;
    let spec: WatchSpec = serde_json::from_str(json).unwrap();
    assert_eq!(
        spec.address.as_deref(),
        Some("0xabcdef1234567890abcdef1234567890abcdef12")
    );
    assert_eq!(spec.chains.len(), 2);
    assert_eq!(spec.chains[0].caip2, "eip155:1");
    assert_eq!(
        spec.chains[0].address.as_deref(),
        Some("0xabcdef1234567890abcdef1234567890abcdef12")
    );
    assert_eq!(spec.chains[1].caip2, "eip155:137");
    assert_eq!(
        spec.chains[1].address.as_deref(),
        Some("0xabcdef1234567890abcdef1234567890abcdef12")
    );
}

#[test]
fn watch_spec_multiple_chains_same_address_fallback() {
    // When chain.address is omitted, it falls back to root address
    let json = r#"{
        "address": "0xabcdef1234567890abcdef1234567890abcdef12",
        "chains": [
            {"caip2": "eip155:1", "start_block": 100},
            {"caip2": "eip155:137", "start_block": 200}
        ]
    }"#;
    let spec: WatchSpec = serde_json::from_str(json).unwrap();
    assert_eq!(spec.chains.len(), 2);
    // Chain-level address is None (uses root fallback in resolution, not here)
    assert_eq!(spec.chains[0].address, None);
    assert_eq!(spec.chains[1].address, None);
}
