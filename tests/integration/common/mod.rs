use pano::config::{AssetConfig, ChainConfig, RpcOptions};
use pano::model::{DepositData, DepositEvent, ResolvedAsset, TargetMap};

pub const EVM_ADDR: &str = "0xAbCdEf1234567890AbCdEf1234567890AbCdEf12";
pub const EVM_ADDR_LOWER: &str = "0xabcdef1234567890abcdef1234567890abcdef12";
pub const EVM_SENDER: &str = "0x1111111111111111111111111111111111111111";
pub const ERC20_CONTRACT: &str = "0x2222222222222222222222222222222222222222";
pub const BTC_ADDR: &str = "1A1zP1eP5QGefi2DMPTfTL5SLmv7Divf";

pub fn evm_chain(rpc_url: impl Into<String>) -> ChainConfig {
    ChainConfig {
        caip2: "eip155:1".to_string(),
        start_block: Some(0),
        end_block: Some(0),
        confirmed_blocks: 12,
        rpc: vec![rpc_url.into()],
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
        assets: vec![
            AssetConfig {
                symbol: "ETH".to_string(),
                contract: None,
                token_program: None,
                decimals: 18,
                min_amount: None,
            },
            AssetConfig {
                symbol: "USDC".to_string(),
                contract: Some(ERC20_CONTRACT.to_string()),
                token_program: None,
                decimals: 6,
                min_amount: None,
            },
        ],
    }
}

pub fn btc_chain(rpc_url: impl Into<String>) -> ChainConfig {
    ChainConfig {
        caip2: "bip122:000000000019d6689c085ae165831e93".to_string(),
        start_block: Some(0),
        end_block: Some(0),
        confirmed_blocks: 6,
        rpc: vec![rpc_url.into()],
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
            max_retries: 1,
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
    }
}

pub fn targets(address: &str, symbol: &str) -> TargetMap {
    let mut targets = TargetMap::new();
    targets.insert(
        address.to_string(),
        vec![ResolvedAsset {
            symbol: symbol.to_string(),
            contract: None,
            token_program: None,
            decimals: None,
        }],
    );
    targets
}

pub fn erc20_targets(address: &str) -> TargetMap {
    let mut targets = TargetMap::new();
    targets.insert(
        address.to_string(),
        vec![ResolvedAsset {
            symbol: "USDC".to_string(),
            contract: Some(ERC20_CONTRACT.to_string()),
            token_program: None,
            decimals: Some(6),
        }],
    );
    targets
}

pub fn sample_data() -> DepositData {
    DepositData {
        tx_id: "0xtx".to_string(),
        caip2: "eip155:1".to_string(),
        symbol: "ETH".to_string(),
        address: EVM_ADDR.to_string(),
        block_number: 123,
        log_index: 0,
        amount: "1000000000000000000".to_string(),
        sender: EVM_SENDER.to_string(),
        confirmations: 1,
        timestamp: "2026-06-04T00:00:00Z".to_string(),
        internal_egress: None,
    }
}

pub fn sample_event() -> DepositEvent {
    DepositEvent::detected(sample_data()).expect("valid sample event")
}

pub fn topic_for_address(address: &str) -> String {
    format!(
        "0x000000000000000000000000{}",
        address.trim_start_matches("0x").to_lowercase()
    )
}
