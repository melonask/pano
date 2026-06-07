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
            if opts.solana_scan_mode == SolanaScanMode::Blocks {
                0 // block mode: scan full lookback range, no per-address batching needed
            } else {
                opts.batch_size
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
