pub mod util;

use crate::config::{AppConfig, ChainConfig};
use crate::delivery::EgressRouter;
use crate::egress::EgressHandle;
use crate::ingress::IngressHandle;
use crate::model::{
    ChainKind, Command, DepositEvent, EgressOverride, ResolvedAsset, ResolvedWatch, TargetMap,
    WatchSpec, normalize_address_key, validate_address_for_chain,
};
use anyhow::Result;
use futures::stream::{FuturesUnordered, StreamExt};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock, broadcast, mpsc};
use tokio::task::JoinHandle;

use util::*;

type WatchKey = (String, String, String);
type WatchedMap = hashbrown::HashMap<WatchKey, ResolvedWatch>;
type SharedWatched = Arc<RwLock<WatchedMap>>;

/// Shared detector state accessible from API handlers and chain scanners.
#[derive(Debug, Clone)]
pub struct DetectorHandle {
    pub cmd_tx: mpsc::Sender<Command>,
    pub events_tx: broadcast::Sender<DepositEvent>,
    /// Keyed by the symmetric triad: (address, caip2, symbol).
    pub watched: SharedWatched,
    pub config: Arc<AppConfig>,
    pub address_change_tx: tokio::sync::watch::Sender<()>,
}

/// Start the detector loop and return its task for graceful shutdown.
pub fn start_with_tasks(
    config: AppConfig,
    ingress: IngressHandle,
    egress: EgressHandle,
) -> Result<(DetectorHandle, JoinHandle<()>)> {
    let (cmd_tx, mut cmd_rx) =
        mpsc::channel::<Command>(config.detector.command_queue_capacity.max(1));
    let (delivery_tx, delivery_rx) =
        mpsc::channel::<DepositEvent>(config.detector.delivery_queue_capacity.max(1));
    let events_tx = egress.event_tx.clone();
    let watched: SharedWatched = Arc::new(RwLock::new(hashbrown::HashMap::new()));
    let config = Arc::new(config);
    let (address_change_tx, _) = tokio::sync::watch::channel(());

    let handle = DetectorHandle {
        cmd_tx: cmd_tx.clone(),
        events_tx: events_tx.clone(),
        watched: watched.clone(),
        config: config.clone(),
        address_change_tx: address_change_tx.clone(),
    };

    let scanners: Vec<_> = config
        .chains
        .iter()
        .map(|chain_cfg| match crate::chain::create_scanner(chain_cfg) {
            Ok(scanner) => {
                let chain_caip2 = chain_cfg.caip2.clone();
                Ok((chain_caip2, scanner))
            }
            Err(e) => Err(anyhow::anyhow!(
                "failed to create scanner for {}: {e}",
                chain_cfg.caip2
            )),
        })
        .collect::<Result<_>>()?;

    let mut ingress_rx = ingress.command_rx;
    let ingress_fwd_tx = handle.cmd_tx.clone();
    tokio::spawn(async move {
        while let Some(cmd) = ingress_rx.recv().await {
            if ingress_fwd_tx.send(cmd).await.is_err() {
                break;
            }
        }
    });

    spawn_delivery_workers(
        delivery_rx,
        config.detector.delivery_workers.max(1),
        EgressRouter::with_http_timeout_secs(config.egress.webhook.timeout_secs),
    );

    let task = tokio::spawn(async move {
        let mut last_scanned: hashbrown::HashMap<String, u64> = hashbrown::HashMap::new();
        let mut last_scan_attempts: hashbrown::HashMap<String, Instant> = hashbrown::HashMap::new();
        let mut unconfirmed_events: Vec<DepositEvent> = Vec::new();
        let mut recent_event_keys: hashbrown::HashSet<String> = hashbrown::HashSet::new();
        let mut recent_event_order: VecDeque<String> = VecDeque::new();
        let dedup_window_size = config.detector.dedup_window_size;
        let stale_multiplier = config.detector.stale_event_eviction_multiplier;
        let stale_min_blocks = config.detector.stale_event_eviction_min_blocks;
        let scan_tick = min_scan_interval(&config.chains);
        loop {
            tokio::select! {
                Some(cmd) = cmd_rx.recv() => {
                    match cmd {
                        Command::Watch(spec) => {
                            match resolve_watch_spec_to_watches(&config, &spec) {
                                Ok(resolved) => {
                                    let mut w = watched.write().await;
                                    for rw in resolved {
                                        tracing::info!(
                                            address = %rw.address,
                                            caip2 = %rw.caip2,
                                            symbol = %rw.symbol,
                                            "address watched"
                                        );
                                        let key = (rw.address.clone(), rw.caip2.clone(), rw.symbol.clone());
                                        w.insert(key, rw);
                                    }
                                    let _ = address_change_tx.send(());
                                }
                                Err(e) => {
                                    tracing::warn!(error = %e, "failed to resolve WatchSpec");
                                }
                            }
                        }
                        Command::Unwatch { address } => {
                            let normalized = normalize_address_key(&address);
                            watched.write().await.retain(|(addr, _, _), _| addr != &normalized);
                            unconfirmed_events.retain(|event| {
                                normalize_address_key(&event.data.address) != normalized
                            });
                            tracing::info!(%normalized, "address unwatched");
                            let _ = address_change_tx.send(());
                        }
                        Command::SyncAll(resolved) => {
                            let mut normalized = Vec::with_capacity(resolved.len());
                            for rw in resolved {
                                match normalize_resolved_watch_from_storage(&config, rw) {
                                    Ok(rw) => normalized.push(rw),
                                    Err(error) => {
                                        tracing::warn!(%error, "skipping invalid stored watch row");
                                    }
                                }
                            }
                            let mut current = watched.write().await;
                            current.clear();
                            for rw in &normalized {
                                current.insert(
                                    (rw.address.clone(), rw.caip2.clone(), rw.symbol.clone()),
                                    rw.clone(),
                                );
                            }
                            unconfirmed_events.retain(|event| {
                                current.contains_key(&(
                                    normalize_address_key(&event.data.address),
                                    event.data.caip2.clone(),
                                    event.data.symbol.clone(),
                                ))
                            });
                            tracing::info!(total = current.len(), "synced watched address state");
                            let _ = address_change_tx.send(());
                        }
                        Command::Shutdown => {
                            tracing::info!("detector shutting down");
                            break;
                        }
                    }
                }
                        _ = tokio::time::sleep(scan_tick) => {
                            let state = watched.read().await.clone();
                            if state.is_empty() { continue; }
                            tracing::debug!(watched_count = state.len(), "detector scan tick");
                            let now = Instant::now();

                            let scan_futures = scanners.iter().filter_map(|(scan_caip2, scanner)| {
                                let chain_cfg = config.chain_by_caip2(scan_caip2)?;
                                let chain_kind = ChainKind::from_caip2(scan_caip2)?;
                                let interval = chain_scan_interval(chain_cfg);
                                let is_due = last_scan_attempts
                                    .get(scan_caip2)
                                    .map(|last| now.duration_since(*last) >= interval)
                                    .unwrap_or(true);
                                if !is_due {
                                    return None;
                                }
                                last_scan_attempts.insert(scan_caip2.clone(), now);

                                let state = state.clone();
                                let config = config.clone();
                                let last_scanned = last_scanned.clone();
                                let scan_timeout = Duration::from_secs(
                                    chain_cfg.rpc_options_or_default().scan_timeout_secs.max(1),
                                );
                                let scan_caip2 = scan_caip2.clone();

                                Some(async move {
                                    let timeout_caip2 = scan_caip2.clone();
                                    match tokio::time::timeout(scan_timeout, async move {
                                    let chain_cfg = config.chain_by_caip2(&scan_caip2)?.clone();

                                    // Build TargetMap from watched state for this chain
                                    let mut targets: TargetMap = hashbrown::HashMap::new();
                                    // Compute per-chain effective start_block (minimum across
                                    // active watches, defaulting to chain_cfg.start_block)
                                    // and end_block (minimum non-zero, 0 = no cap)
                                    let mut effective_start: Option<u64> = None;
                                    let mut effective_end: Option<u64> = None;
                                    for ((address, caip2, _symbol), rw) in &state {
                                        if caip2 == &scan_caip2 {
                                            targets
                                                .entry(address.clone())
                                                .or_default()
                                                .push(ResolvedAsset {
                                                    symbol: rw.symbol.clone(),
                                                    contract: rw.contract.clone(),
                                                    token_program: rw.token_program.clone(),
                                                    decimals: rw.decimals,
                                                });
                                            // Track minimum start_block
                                            if let Some(sb) = rw.start_block {
                                                effective_start = Some(
                                                    effective_start
                                                        .map(|es| es.min(sb))
                                                        .unwrap_or(sb),
                                                );
                                            }
                                            // Track minimum non-zero end_block
                                            if let Some(eb) = rw.end_block
                                                && eb > 0
                                            {
                                                effective_end = Some(
                                                    effective_end
                                                        .map(|ee| ee.min(eb))
                                                        .unwrap_or(eb),
                                                );
                                            }
                                        }
                                    }
                                    if targets.is_empty() {
                                        return None;
                                    }

                                    let tip = match scanner.get_tip().await {
                                        Ok(t) => t,
                                        Err(e) => {
                                            tracing::warn!(caip2 = %scan_caip2, error = %e, "failed to get tip");
                                            return None;
                                        }
                                    };
                                    // end_block: per-watch minimum takes priority, then
                                    // chain_cfg, then tip as ultimate cap.
                                    let configured_end = chain_cfg.end_block.filter(|end| *end > 0);
                                    let per_watch_end = effective_end;
                                    let cap_end = match (per_watch_end, configured_end) {
                                        (Some(pw), Some(cfg)) => Some(pw.min(cfg)),
                                        (Some(pw), None) => Some(pw),
                                        (None, Some(cfg)) => Some(cfg),
                                        (None, None) => None,
                                    };
                                    let to_block = std::cmp::min(tip, cap_end.unwrap_or(tip));
                                    let lookback = chain_cfg.effective_scan_lookback_blocks();
                                    // start_block: per-watch minimum takes priority, then
                                    // chain_cfg, then tip minus lookback.
                                    let cfg_start = chain_cfg.start_block;
                                    let configured_start = match (effective_start, cfg_start) {
                                        (Some(pw), Some(cfg)) => Some(pw.min(cfg)),
                                        (Some(pw), None) => Some(pw),
                                        (None, Some(cfg)) => Some(cfg),
                                        (None, None) => None,
                                    }
                                    .unwrap_or_else(|| to_block.saturating_sub(lookback));
                                    let scan_start = |cursor_key: &str| {
                                        last_scanned
                                            .get(cursor_key)
                                            .copied()
                                            .unwrap_or(configured_start)
                                            .saturating_sub(lookback)
                                            .max(configured_start)
                                    };

                                    let mut scan_jobs = Vec::new();
                                    let native_symbols: Vec<String> = chain_cfg
                                        .assets
                                        .iter()
                                        .filter(|asset| asset.contract.is_none())
                                        .map(|asset| asset.symbol.clone())
                                        .collect();
                                    let contract_symbols: Vec<String> = chain_cfg
                                        .assets
                                        .iter()
                                        .filter(|asset| asset.contract.is_some())
                                        .map(|asset| asset.symbol.clone())
                                        .collect();

                                    if matches!(chain_kind, ChainKind::Evm)
                                        && !native_symbols.is_empty()
                                        && !contract_symbols.is_empty()
                                    {
                                        let token_cursor = format!("{scan_caip2}:erc20");
                                        if let Some(token_targets) = filter_targets_for_symbols(&targets, &contract_symbols) {
                                            let start = scan_start(&token_cursor);
                                            scan_jobs.push((token_cursor, start, to_block, token_targets));
                                        }
                                        let native_cursor = format!("{scan_caip2}:native");
                                        if let Some(native_targets) = filter_targets_for_symbols(&targets, &native_symbols) {
                                            let start = scan_start(&native_cursor);
                                            let scan_to_block = effective_evm_native_scan_to_block(&chain_cfg, start, to_block, lookback);
                                            scan_jobs.push((native_cursor, start, scan_to_block, native_targets));
                                        }
                                    } else if matches!(chain_kind, ChainKind::Evm)
                                        && !native_symbols.is_empty()
                                        && contract_symbols.is_empty()
                                    {
                                        let start = scan_start(&scan_caip2);
                                        let scan_to_block = effective_evm_native_scan_to_block(&chain_cfg, start, to_block, lookback);
                                        scan_jobs.push((scan_caip2.clone(), start, scan_to_block, targets));
                                    } else {
                                        let start = scan_start(&scan_caip2);
                                        let scan_to_block = effective_scan_to_block(&chain_cfg, start, to_block, lookback);
                                        scan_jobs.push((scan_caip2.clone(), start, scan_to_block, targets));
                                    }

                            let mut scan_outcomes = Vec::new();
                            for (cursor_key, start, scan_to_block, scan_targets) in scan_jobs {
                                if start <= scan_to_block {
                                    scan_outcomes.push(ScanOutcome {
                                        cursor_key,
                                        to_block: scan_to_block,
                                        events: scanner.scan(start, scan_to_block, &scan_targets).await,
                                    });
                                }
                            }

                            Some(ChainScanResult {
                                caip2: scan_caip2,
                                chain_cfg,
                                tip,
                                scan_outcomes,
                            })
                                    })
                                    .await
                                    {
                                        Ok(result) => result,
                                        Err(_) => {
                                            tracing::error!(
                                                caip2 = %timeout_caip2,
                                                timeout_secs = scan_timeout.as_secs(),
                                                "chain scan timed out"
                                            );
                                            None
                                        }
                                    }
                        })
                    });
                    let mut scan_results: FuturesUnordered<_> = scan_futures.collect();

                    while let Some(scan_result) = scan_results.next().await {
                        let Some(scan_result) = scan_result else { continue; };
                        let ChainScanResult { caip2, chain_cfg, tip, scan_outcomes } = scan_result;
                        for scan_outcome in scan_outcomes {
                            let ScanOutcome { cursor_key, to_block, events } = scan_outcome;
                            match events {
                                Ok(mut events) => {
                                    for event in &mut events {
                                        // Lookup the ResolvedWatch for per-address egress and thresholds.
                                        if let Some(rw) = find_resolved_watch(&state, &event.data.address, &caip2, &event.data.symbol) {
                                            event.data.internal_egress = rw.egress.clone();
                                            if let Some(min) = rw.min_amount.as_ref() {
                                                match (event.data.amount.parse::<u128>(), min.parse::<u128>()) {
                                                    (Ok(amount_val), Ok(min_val)) if amount_val < min_val => {
                                                        tracing::trace!(
                                                            address = %event.data.address,
                                                            symbol = %event.data.symbol,
                                                            amount = %event.data.amount,
                                                            min_amount = %min,
                                                            "skipping deposit below minimum threshold"
                                                        );
                                                        continue;
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        } else {
                                            tracing::warn!(
                                                address = %event.data.address,
                                                caip2 = %caip2,
                                                symbol = %event.data.symbol,
                                                "deposit event triad not found in watched set; possible normalization gap"
                                            );
                                        }

                                        let event_key = deposit_event_key(event);
                                        if !remember_event_key(
                                            &mut recent_event_keys,
                                            &mut recent_event_order,
                                            dedup_window_size,
                                            event_key,
                                        ) {
                                            continue;
                                        }
                                        let _ = events_tx.send(event.clone());
                                        if event.data.internal_egress.is_some() {
                                            enqueue_delivery(&delivery_tx, event);
                                        }
                                        unconfirmed_events.push(event.clone());
                                    }
                                    last_scanned.insert(cursor_key, to_block.saturating_add(1));
                                }
                                Err(e) => {
                                    tracing::error!(caip2 = %caip2, error = %e, "scan failed");
                                }
                            }
                        }
                        let mut i = 0;
                        while i < unconfirmed_events.len() {
                            if unconfirmed_events[i].data.caip2 == caip2 {
                                let confirmations = tip
                                    .saturating_sub(unconfirmed_events[i].data.block_number)
                                    .saturating_add(1) as u32;
                                // Use per-watch confirmed_blocks; fall back to chain cfg
                                let required_confs =
                                    find_resolved_watch(
                                        &state,
                                        &unconfirmed_events[i].data.address,
                                        &caip2,
                                        &unconfirmed_events[i].data.symbol,
                                    )
                                    .map(|rw| rw.confirmed_blocks)
                                    .unwrap_or(chain_cfg.confirmed_blocks);
                                if confirmations >= required_confs {
                                    let detected = unconfirmed_events.swap_remove(i);
                                    let confirmed = match DepositEvent::confirmed_from(&detected, confirmations) {
                                        Ok(event) => event,
                                        Err(error) => {
                                            tracing::error!(%error, event_id = %detected.event_id, "invalid confirmed deposit event");
                                            continue;
                                        }
                                    };
                                    let confirmed_key = deposit_event_key(&confirmed);
                                    if remember_event_key(
                                        &mut recent_event_keys,
                                        &mut recent_event_order,
                                        dedup_window_size,
                                        confirmed_key,
                                    ) {
                                        let _ = events_tx.send(confirmed.clone());
                                        if confirmed.data.internal_egress.is_some() {
                                            enqueue_delivery(&delivery_tx, &confirmed);
                                        }
                                    }
                                    continue;
                                }
                                let evict_threshold = u64::from(required_confs)
                                    .saturating_mul(stale_multiplier)
                                    .max(stale_min_blocks);
                                if tip.saturating_sub(unconfirmed_events[i].data.block_number) > evict_threshold
                                {
                                    tracing::warn!(event_id = %unconfirmed_events[i].event_id, caip2 = %caip2, "evicting stale unconfirmed event");
                                    unconfirmed_events.swap_remove(i);
                                    continue;
                                }
                            }
                            i += 1;
                        }
                    }
                }
            }
        }
    });

    Ok((handle, task))
}

/// Normalize and validate a flat watch row loaded from durable storage.
///
/// Database ingress stores only the symmetric triad plus nullable JSON blocks.
/// A NULL JSON block means "fall back to global config", so the detector must
/// hydrate those defaults before replacing runtime state.
fn normalize_resolved_watch_from_storage(
    config: &AppConfig,
    mut rw: ResolvedWatch,
) -> Result<ResolvedWatch> {
    let chain = config
        .chain_by_caip2(&rw.caip2)
        .ok_or_else(|| anyhow::anyhow!("unknown stored chain caip2: {}", rw.caip2))?;
    if rw
        .contract
        .as_deref()
        .is_some_and(|contract| contract.trim().is_empty())
    {
        rw.contract = None;
    }
    let kind = ChainKind::from_caip2(&rw.caip2)
        .ok_or_else(|| anyhow::anyhow!("unsupported stored chain caip2: {}", rw.caip2))?;
    rw.address = normalize_address_key(&rw.address);
    if !validate_address_for_chain(kind, &rw.address) {
        anyhow::bail!(
            "invalid stored address for chain {}: {}",
            rw.caip2,
            rw.address
        );
    }

    rw.start_block = rw.start_block.or(chain.start_block);
    rw.end_block = rw.end_block.or(chain.end_block);
    if rw.confirmed_blocks == 0 {
        rw.confirmed_blocks = chain.confirmed_blocks;
    }
    if rw.confirmed_blocks == 0 {
        anyhow::bail!("stored watch confirmed_blocks must be greater than 0");
    }

    if let Some(config_asset) = chain
        .assets
        .iter()
        .find(|asset| asset.symbol.eq_ignore_ascii_case(&rw.symbol))
    {
        rw.contract = rw.contract.or_else(|| config_asset.contract.clone());
        rw.token_program = rw
            .token_program
            .or_else(|| config_asset.token_program.clone());
        rw.decimals = rw.decimals.or(Some(config_asset.decimals));
        rw.min_amount = rw.min_amount.or_else(|| config_asset.min_amount.clone());
    } else if rw.contract.is_none() || rw.decimals.is_none() {
        anyhow::bail!(
            "stored custom asset '{}' on chain {} requires contract and decimals",
            rw.symbol,
            rw.caip2
        );
    }

    if let Some(decimals) = rw.decimals
        && decimals > config.detector.max_decimals
    {
        anyhow::bail!(
            "stored asset {} decimals {} exceeds maximum of {}",
            rw.symbol,
            decimals,
            config.detector.max_decimals
        );
    }
    validated_asset_min_amount(&rw.symbol, &rw.caip2, &rw.min_amount)?;
    validate_egress_override(&config.override_, &rw.egress)?;

    Ok(rw)
}

/// Unified resolver for WatchSpec → Vec<ResolvedWatch>.
///
/// Used by ALL ingress paths (HTTP API, file, queue, SQLite/PG) through
/// `Command::Watch`. Performs full validation including override permissions,
/// egress override gates, confirmed_blocks>0, decimals limit, min_amount
/// digit rules, address cascade, and custom asset requirements.
///
/// A Watch command that fails validation will NOT mutate state — the caller
/// (detector loop or API handler) logs/rejects accordingly.
pub fn resolve_watch_spec_to_watches(
    config: &AppConfig,
    spec: &WatchSpec,
) -> Result<Vec<ResolvedWatch>> {
    validate_watch_spec(config, spec)?;

    if spec.chains.is_empty() {
        return resolve_shorthand_watch(config, spec);
    }

    resolve_chain_watches(config, spec)
}

/// Validate request-level invariants before any expansion can mutate state.
fn validate_watch_spec(config: &AppConfig, spec: &WatchSpec) -> Result<()> {
    if spec.address.is_none() && spec.chains.is_empty() {
        anyhow::bail!("at least one of 'address' or 'chains' must be present");
    }

    if !spec.chains.is_empty() && config.override_.chains.is_none() {
        anyhow::bail!(
            "WatchSpec contains 'chains', but [override.chains] is not enabled in server config; remove 'chains' from the watch spec or add [override.chains] (for example: assets = true) to allow per-watch chain overrides"
        );
    }
    validate_egress_override(&config.override_, &spec.egress)
}

/// Expand an address-only request across compatible configured chains and assets.
fn resolve_shorthand_watch(config: &AppConfig, spec: &WatchSpec) -> Result<Vec<ResolvedWatch>> {
    let root_addr = spec
        .address
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("address is required when chains is empty"))?;
    let root_addr = normalize_address_key(root_addr);

    let mut resolved = Vec::new();
    for chain in &config.chains {
        let kind = ChainKind::from_caip2(&chain.caip2)
            .ok_or_else(|| anyhow::anyhow!("unsupported chain: {}", chain.caip2))?;
        if validate_address_for_chain(kind, &root_addr) {
            for asset in &chain.assets {
                validated_asset_min_amount(asset.symbol.as_str(), &chain.caip2, &asset.min_amount)?;
                resolved.push(ResolvedWatch {
                    address: root_addr.clone(),
                    caip2: chain.caip2.clone(),
                    symbol: asset.symbol.clone(),
                    contract: asset.contract.clone(),
                    token_program: asset.token_program.clone(),
                    decimals: Some(asset.decimals),
                    start_block: chain.start_block,
                    end_block: chain.end_block,
                    confirmed_blocks: chain.confirmed_blocks,
                    min_amount: asset.min_amount.clone(),
                    egress: spec.egress.clone(),
                });
            }
        }
    }
    if resolved.is_empty() {
        anyhow::bail!("address does not match any configured chain type");
    }
    Ok(resolved)
}

/// Resolve explicit chain entries, including permitted asset overrides.
fn resolve_chain_watches(config: &AppConfig, spec: &WatchSpec) -> Result<Vec<ResolvedWatch>> {
    let assets_allowed = config
        .override_
        .chains
        .as_ref()
        .map(|c| c.assets)
        .unwrap_or(false);

    let mut resolved = Vec::new();
    for entry in &spec.chains {
        let chain_cfg = config
            .chain_by_caip2(&entry.caip2)
            .ok_or_else(|| anyhow::anyhow!("unknown chain caip2: {}", entry.caip2))?;

        let kind = ChainKind::from_caip2(&entry.caip2)
            .ok_or_else(|| anyhow::anyhow!("unsupported chain: {}", entry.caip2))?;

        let effective_confirmed = entry.confirmed_blocks.unwrap_or(chain_cfg.confirmed_blocks);
        if effective_confirmed == 0 {
            anyhow::bail!(
                "chain {} confirmed_blocks must be greater than 0",
                entry.caip2
            );
        }

        if entry.assets.is_empty() {
            // No asset filter: expand all configured assets
            let chain_address = entry
                .address
                .as_ref()
                .or(spec.address.as_ref())
                .map(|a| normalize_address_key(a))
                .ok_or_else(|| anyhow::anyhow!("address required for chain {}", entry.caip2))?;

            if !validate_address_for_chain(kind, &chain_address) {
                anyhow::bail!(
                    "invalid address for chain {}: {}",
                    entry.caip2,
                    chain_address
                );
            }

            for asset in &chain_cfg.assets {
                resolved.push(ResolvedWatch {
                    address: chain_address.clone(),
                    caip2: entry.caip2.clone(),
                    symbol: asset.symbol.clone(),
                    contract: asset.contract.clone(),
                    token_program: asset.token_program.clone(),
                    decimals: Some(asset.decimals),
                    start_block: entry.start_block.or(chain_cfg.start_block),
                    end_block: entry.end_block.or(chain_cfg.end_block),
                    confirmed_blocks: effective_confirmed,
                    min_amount: asset.min_amount.clone(),
                    egress: spec.egress.clone(),
                });
            }
        } else {
            // Assets specified: require override.chains.assets = true
            if !assets_allowed {
                anyhow::bail!(
                    "'assets' not allowed: override.chains.assets is disabled in server config"
                );
            }

            for asset_entry in &entry.assets {
                let effective_address = asset_entry
                    .address
                    .as_ref()
                    .or(entry.address.as_ref())
                    .or(spec.address.as_ref())
                    .map(|a| normalize_address_key(a))
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "address required for asset {} on chain {}",
                            asset_entry.symbol,
                            entry.caip2
                        )
                    })?;

                if !validate_address_for_chain(kind, &effective_address) {
                    anyhow::bail!(
                        "invalid address for chain {}: {}",
                        entry.caip2,
                        effective_address
                    );
                }

                // Validate decimals if provided
                if let Some(dec) = asset_entry.decimals
                    && dec > config.detector.max_decimals
                {
                    anyhow::bail!(
                        "asset {} decimals {} exceeds maximum of {}",
                        asset_entry.symbol,
                        dec,
                        config.detector.max_decimals
                    );
                }

                // Validate min_amount if provided
                validated_asset_min_amount(
                    &asset_entry.symbol,
                    &entry.caip2,
                    &asset_entry.min_amount,
                )?;

                if let Some(config_asset) = chain_cfg
                    .assets
                    .iter()
                    .find(|a| a.symbol.eq_ignore_ascii_case(&asset_entry.symbol))
                {
                    let contract = asset_entry
                        .contract
                        .as_ref()
                        .or(config_asset.contract.as_ref())
                        .cloned();
                    let token_program = asset_entry
                        .token_program
                        .as_ref()
                        .or(config_asset.token_program.as_ref())
                        .cloned();
                    let decimals = asset_entry.decimals.or(Some(config_asset.decimals));
                    let min_amount = asset_entry
                        .min_amount
                        .as_ref()
                        .or(config_asset.min_amount.as_ref())
                        .cloned();

                    resolved.push(ResolvedWatch {
                        address: effective_address,
                        caip2: entry.caip2.clone(),
                        symbol: asset_entry.symbol.clone(),
                        contract,
                        token_program,
                        decimals,
                        start_block: entry.start_block.or(chain_cfg.start_block),
                        end_block: entry.end_block.or(chain_cfg.end_block),
                        confirmed_blocks: effective_confirmed,
                        min_amount,
                        egress: spec.egress.clone(),
                    });
                } else {
                    // Custom asset: requires contract and decimals
                    if asset_entry.contract.is_none() || asset_entry.decimals.is_none() {
                        anyhow::bail!(
                            "custom asset '{}' on chain {} requires both 'contract' and 'decimals'",
                            asset_entry.symbol,
                            entry.caip2
                        );
                    }
                    resolved.push(ResolvedWatch {
                        address: effective_address,
                        caip2: entry.caip2.clone(),
                        symbol: asset_entry.symbol.clone(),
                        contract: asset_entry.contract.clone(),
                        token_program: asset_entry.token_program.clone(),
                        decimals: asset_entry.decimals,
                        start_block: entry.start_block.or(chain_cfg.start_block),
                        end_block: entry.end_block.or(chain_cfg.end_block),
                        confirmed_blocks: effective_confirmed,
                        min_amount: asset_entry.min_amount.clone(),
                        egress: spec.egress.clone(),
                    });
                }
            }
        }
    }

    if resolved.is_empty() {
        anyhow::bail!("no valid watch targets resolved");
    }

    Ok(resolved)
}

/// Validate egress override permissions from the override config.
pub fn validate_egress_override(
    override_config: &crate::config::OverrideConfig,
    egress: &Option<EgressOverride>,
) -> anyhow::Result<()> {
    let Some(egress) = egress else {
        return Ok(());
    };
    if egress.webhook.is_some() && !override_config.egress.webhook {
        anyhow::bail!("'egress.webhook' not allowed: override.egress.webhook is disabled");
    }
    if egress.file.is_some() && !override_config.egress.file {
        anyhow::bail!("'egress.file' not allowed: override.egress.file is disabled");
    }
    if egress.pg.is_some() && !override_config.egress.pg {
        anyhow::bail!("'egress.pg' not allowed: override.egress.pg is disabled");
    }
    if egress.sqlite.is_some() && !override_config.egress.sqlite {
        anyhow::bail!("'egress.sqlite' not allowed: override.egress.sqlite is disabled");
    }
    if egress.queue.is_some() && !override_config.egress.queue {
        anyhow::bail!("'egress.queue' not allowed: override.egress.queue is disabled");
    }
    if egress.http.is_some() && !override_config.egress.http {
        anyhow::bail!("'egress.http' not allowed: override.egress.http is disabled");
    }
    Ok(())
}

fn validated_asset_min_amount(
    symbol: &str,
    caip2: &str,
    min_amount: &Option<String>,
) -> anyhow::Result<()> {
    if let Some(min) = min_amount
        && (min.is_empty()
            || !min.chars().all(|c| c.is_ascii_digit())
            || min == "0"
            || (min.len() > 1 && min.starts_with('0')))
    {
        anyhow::bail!(
            "asset {} on {} has invalid min_amount '{}': must be a positive integer without leading zeros",
            symbol,
            caip2,
            min
        );
    }
    Ok(())
}

fn find_resolved_watch<'a>(
    state: &'a hashbrown::HashMap<(String, String, String), ResolvedWatch>,
    address: &str,
    caip2: &str,
    symbol: &str,
) -> Option<&'a ResolvedWatch> {
    let key = (address.to_string(), caip2.to_string(), symbol.to_string());
    if let Some(rw) = state.get(&key) {
        return Some(rw);
    }
    // Try normalized address variant
    let norm = normalize_address_key(address);
    if norm != address {
        let key = (norm, caip2.to_string(), symbol.to_string());
        return state.get(&key);
    }
    None
}

fn spawn_delivery_workers(
    delivery_rx: mpsc::Receiver<DepositEvent>,
    worker_count: usize,
    delivery_router: EgressRouter,
) {
    let delivery_rx = Arc::new(Mutex::new(delivery_rx));
    for worker_id in 0..worker_count {
        let delivery_rx = delivery_rx.clone();
        let delivery_router = delivery_router.clone();
        tokio::spawn(async move {
            loop {
                let event = {
                    let mut delivery_rx = delivery_rx.lock().await;
                    delivery_rx.recv().await
                };
                let Some(event) = event else {
                    break;
                };
                delivery_router.route(&event).await;
            }
            tracing::debug!(worker_id, "delivery worker stopped");
        });
    }
}

fn enqueue_delivery(delivery_tx: &mpsc::Sender<DepositEvent>, event: &DepositEvent) {
    if let Err(error) = delivery_tx.try_send(event.clone()) {
        tracing::error!(
            %error,
            event_id = %event.event_id,
            "per-address egress queue full or closed; dropping egress override event"
        );
    }
}

struct ChainScanResult {
    caip2: String,
    chain_cfg: ChainConfig,
    tip: u64,
    scan_outcomes: Vec<ScanOutcome>,
}

struct ScanOutcome {
    cursor_key: String,
    to_block: u64,
    events: anyhow::Result<Vec<DepositEvent>>,
}

fn filter_targets_for_symbols(targets: &TargetMap, symbols: &[String]) -> Option<TargetMap> {
    if symbols.is_empty() {
        return None;
    }
    let mut filtered = TargetMap::default();
    for (address, assets) in targets {
        let matching: Vec<ResolvedAsset> = assets
            .iter()
            .filter(|ra| {
                symbols
                    .iter()
                    .any(|symbol| ra.symbol.eq_ignore_ascii_case(symbol))
            })
            .cloned()
            .collect();
        if !matching.is_empty() {
            filtered.insert(address.clone(), matching);
        }
    }
    (!filtered.is_empty()).then_some(filtered)
}

pub fn remember_event_key(
    recent_event_keys: &mut hashbrown::HashSet<String>,
    recent_event_order: &mut VecDeque<String>,
    dedup_window_size: usize,
    event_key: String,
) -> bool {
    if !recent_event_keys.insert(event_key.clone()) {
        return false;
    }
    recent_event_order.push_back(event_key);
    while dedup_window_size > 0 && recent_event_order.len() > dedup_window_size {
        if let Some(old_key) = recent_event_order.pop_front() {
            recent_event_keys.remove(&old_key);
        }
    }
    true
}
