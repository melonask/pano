use crate::config::{ChainConfig, RpcOptions, SolanaScanMode};
use crate::model::{ChainKind, DepositEvent};
use std::time::Duration;

/// Generate a unique key for deposit event deduplication.
pub fn deposit_event_key(event: &DepositEvent) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        event.data.tx_id,
        event.data.caip2,
        event.data.symbol,
        crate::model::normalize_address_key(&event.data.address),
        event.data.block_number,
        event.data.amount,
        event.data.log_index,
        event.event,
    )
}

/// Return the scan interval for a chain, clamped to at least 1 second.
pub fn chain_scan_interval(chain_cfg: &ChainConfig) -> Duration {
    Duration::from_secs(chain_cfg.rpc_options_or_default().scan_interval_secs.max(1))
}

/// Return the minimum scan interval across all configured chains.
pub fn min_scan_interval(chains: &[ChainConfig]) -> Duration {
    chains
        .iter()
        .map(chain_scan_interval)
        .min()
        .unwrap_or_else(|| Duration::from_secs(RpcOptions::default().scan_interval_secs))
}

/// Compute the effective block to scan up to, respecting chain-specific caps.
///
/// Solana signature scans use indexed address-range filtering, so `batch_size`
/// limits signature pages rather than the slot range. Block scans make one RPC
/// request per slot and are therefore capped by `batch_size` slots per cycle.
pub fn effective_scan_to_block(
    chain_cfg: &ChainConfig,
    start: u64,
    requested_to: u64,
    lookback: u64,
) -> u64 {
    if start > requested_to {
        return requested_to;
    }
    let Some(kind) = ChainKind::from_caip2(&chain_cfg.caip2) else {
        return requested_to;
    };
    let cap = match kind {
        ChainKind::Evm => 0,
        ChainKind::Bitcoin => chain_cfg.rpc_options_or_default().batch_size,
        ChainKind::Solana => {
            let opts = chain_cfg.rpc_options_or_default();
            match opts.solana_scan_mode {
                SolanaScanMode::Signatures => return requested_to,
                SolanaScanMode::Blocks => {
                    return requested_to.min(start.saturating_add(opts.batch_size.max(1) - 1));
                }
            }
        }
    };
    if cap == 0 {
        return requested_to;
    }
    let effective_cap = cap.max(lookback.saturating_add(1)).max(1);
    requested_to.min(start.saturating_add(effective_cap - 1))
}

/// Compute the native EVM scan cap without limiting ERC-20 log scans.
pub fn effective_evm_native_scan_to_block(
    chain_cfg: &ChainConfig,
    start: u64,
    requested_to: u64,
    lookback: u64,
) -> u64 {
    if start > requested_to {
        return requested_to;
    }
    let cap = chain_cfg.rpc_options_or_default().max_native_scan_per_cycle;
    if cap == 0 {
        return requested_to;
    }
    let effective_cap = cap.max(lookback.saturating_add(1)).max(1);
    requested_to.min(start.saturating_add(effective_cap - 1))
}

#[cfg(test)]
mod tests {
    use super::effective_scan_to_block;
    use crate::config::{AssetConfig, ChainConfig, RpcOptions, SolanaScanMode};

    fn solana_chain(mode: SolanaScanMode, batch_size: u64) -> ChainConfig {
        ChainConfig {
            caip2: "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp".to_string(),
            start_block: None,
            end_block: None,
            confirmed_blocks: 32,
            rpc: vec!["http://localhost".to_string()],
            rpc_options: Some(RpcOptions {
                batch_size,
                solana_scan_mode: mode,
                ..RpcOptions::default()
            }),
            assets: Vec::<AssetConfig>::new(),
        }
    }

    #[test]
    fn solana_signature_scan_reaches_requested_tip_regardless_of_page_size() {
        let chain = solana_chain(SolanaScanMode::Signatures, 20);

        assert_eq!(effective_scan_to_block(&chain, 100, 1_040, 500), 1_040);
    }

    #[test]
    fn solana_block_scan_is_capped_to_batch_size_slots() {
        let chain = solana_chain(SolanaScanMode::Blocks, 20);

        assert_eq!(effective_scan_to_block(&chain, 100, 1_040, 500), 119);
    }
}
