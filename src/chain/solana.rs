use crate::chain::{ChainScanner, asset_allowed};
use crate::config::{ChainConfig, SolanaScanMode};
use crate::model::{DepositData, DepositEvent, TargetMap};
use crate::rpc::RpcClient;
use futures::StreamExt;
use solana_pubkey::Pubkey;
use std::str::FromStr;

const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022_PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
const ASSOCIATED_TOKEN_PROGRAM_ID: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";

/// Solana chain scanner.
pub struct SolanaScanner {
    chain: ChainConfig,
    rpc: RpcClient,
}

impl SolanaScanner {
    pub fn new(chain: ChainConfig) -> anyhow::Result<Self> {
        let rpc = RpcClient::new(chain.clone());
        Ok(Self { chain, rpc })
    }
}

#[async_trait::async_trait]
impl ChainScanner for SolanaScanner {
    fn caip2(&self) -> &str {
        &self.chain.caip2
    }

    async fn get_tip(&self) -> anyhow::Result<u64> {
        let res = self
            .rpc
            .send(
                "getSlot",
                serde_json::json!([{ "commitment": "confirmed" }]),
            )
            .await?;
        res.as_u64()
            .ok_or_else(|| anyhow::anyhow!("getSlot returned non-u64 result: {res}"))
    }

    async fn scan(
        &self,
        from_slot: u64,
        to_slot: u64,
        targets: &TargetMap,
    ) -> anyhow::Result<Vec<DepositEvent>> {
        let mut events = Vec::new();
        if targets.is_empty() || from_slot > to_slot {
            return Ok(events);
        }

        let scan_targets = expand_solana_scan_targets(targets);
        tracing::debug!(
            original_targets = targets.len(),
            scan_targets = scan_targets.len(),
            "expanded Solana scan targets"
        );

        let mode = self.chain.rpc_options_or_default().solana_scan_mode;
        match mode {
            SolanaScanMode::Blocks => {
                tracing::debug!(from_slot, to_slot, "Solana block scan starting");
                self.scan_blocks(from_slot, to_slot, &scan_targets, &mut events)
                    .await?;
            }
            SolanaScanMode::Signatures => {
                self.scan_signatures(from_slot, to_slot, &scan_targets, &mut events)
                    .await?;
            }
        }

        Ok(events)
    }
}

/// Block-based scanning: call getBlock for each slot, process all transactions.
/// Zero dependency on RPC signature indexing.
impl SolanaScanner {
    async fn scan_blocks(
        &self,
        from_slot: u64,
        to_slot: u64,
        targets: &TargetMap,
        events: &mut Vec<DepositEvent>,
    ) -> anyhow::Result<()> {
        let max_version = self
            .chain
            .rpc_options_or_default()
            .solana_max_supported_transaction_version;
        let concurrency = self.chain.rpc_options_or_default().max_concurrent.max(1);

        let slots: Vec<u64> = (from_slot..=to_slot).collect();

        for chunk in slots.chunks(concurrency) {
            let futs: Vec<_> = chunk
                .iter()
                .map(|&slot| {
                    let rpc = self.rpc.clone();
                    let params = serde_json::json!([
                        slot,
                        {
                            "encoding": "json",
                            "transactionDetails": "full",
                            "rewards": false,
                            "commitment": "confirmed",
                            "maxSupportedTransactionVersion": max_version,
                        }
                    ]);
                    async move {
                        let result = rpc.send("getBlock", params).await;
                        (slot, result)
                    }
                })
                .collect();

            let results: Vec<(u64, Result<serde_json::Value, anyhow::Error>)> =
                futures::future::join_all(futs).await;

            for (slot, result) in results {
                let block = match result {
                    Ok(block) => block,
                    Err(e) => {
                        let msg = e.to_string();
                        if msg.contains("-32004") || msg.contains("block not available") {
                            continue;
                        }
                        if msg.contains("was skipped") || msg.contains("skipped slot") {
                            continue;
                        }
                        tracing::warn!(slot, error = %e, "getBlock failed for slot");
                        continue;
                    }
                };

                let block_time = block.get("blockTime").and_then(|v| v.as_i64());

                let Some(txs) = block.get("transactions").and_then(|v| v.as_array()) else {
                    continue;
                };

                for tx in txs {
                    if tx
                        .get("meta")
                        .and_then(|m| m.get("err"))
                        .is_some_and(|e| !e.is_null())
                    {
                        continue;
                    }
                    if !tx_involves_watched(tx, targets) {
                        continue;
                    }
                    let sig = tx
                        .pointer("/transaction/signatures/0")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if let Err(e) =
                        self.process_transaction(sig, slot, tx, targets, events, block_time)
                    {
                        tracing::warn!(signature = %sig, slot, error = %e, "failed to process Solana transaction");
                    }
                }
            }
        }

        tracing::debug!(
            from_slot,
            to_slot,
            events = events.len(),
            "Solana block scan complete"
        );
        Ok(())
    }

    /// Signature-based scanning: the original per-address getSignaturesForAddress path.
    async fn scan_signatures(
        &self,
        from_slot: u64,
        to_slot: u64,
        targets: &TargetMap,
        events: &mut Vec<DepositEvent>,
    ) -> anyhow::Result<()> {
        let page_limit = self
            .chain
            .rpc_options_or_default()
            .batch_size
            .clamp(1, 1000);
        let mut seen_signatures = hashbrown::HashSet::new();

        for addr in targets.keys() {
            let mut before: Option<String> = None;
            let mut cursor_lost = false;
            loop {
                let mut params = serde_json::json!([
                    addr,
                    {"limit": page_limit, "minContextSlot": from_slot, "commitment": "confirmed"}
                ]);
                if let Some(cursor) = before.as_ref() {
                    if let Some(obj) = params[1].as_object_mut() {
                        obj.insert("before".to_string(), serde_json::json!(cursor));
                    } else {
                        tracing::warn!(
                            "unexpected getSignaturesForAddress params structure, breaking pagination"
                        );
                        break;
                    }
                }
                let sigs = match self.rpc.send("getSignaturesForAddress", params).await {
                    Ok(sigs) => sigs,
                    Err(error) if before.is_some() && is_solana_pruned_cursor_error(&error) => {
                        tracing::warn!(
                            address = %addr,
                            from_slot,
                            error = %error,
                            "solana cursor lost (tx pruned), falling back to block-based rescan from {from_slot}"
                        );
                        before = None;
                        cursor_lost = true;
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                let Some(sig_array) = sigs.as_array() else {
                    break;
                };
                if sig_array.is_empty() {
                    tracing::debug!(address = %addr, "no Solana signatures for scan target");
                    break;
                }
                tracing::debug!(address = %addr, signatures = sig_array.len(), "fetched Solana signatures for scan target");

                let mut reached_older_slots = false;
                let mut candidates = Vec::new();
                for sig_entry in sig_array {
                    if sig_entry.get("err").is_some_and(|e| !e.is_null()) {
                        continue;
                    }
                    let Some(slot) = sig_entry.get("slot").and_then(|v| v.as_u64()) else {
                        tracing::warn!(?sig_entry, "skipping Solana signature entry without slot");
                        continue;
                    };
                    if slot < from_slot {
                        reached_older_slots = true;
                        continue;
                    }
                    if slot > to_slot {
                        continue;
                    }
                    let Some(sig) = sig_entry.get("signature").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    if seen_signatures.insert(sig.to_string()) {
                        candidates.push((sig.to_string(), slot));
                    }
                }

                let batch_concurrency = self.chain.rpc_options_or_default().max_concurrent.max(1);
                let transaction_results: Vec<(String, u64, anyhow::Result<serde_json::Value>)> = {
                    let rpc = self.rpc.clone();
                    let max_supported_transaction_version = self
                        .chain
                        .rpc_options_or_default()
                        .solana_max_supported_transaction_version;
                    let sigs: Vec<(String, u64)> = candidates.clone();
                    let futures_iter = sigs.into_iter().map(|(sig, slot)| {
                        let rpc = rpc.clone();
                        async move {
                            let txn = rpc
                                .send(
                                    "getTransaction",
                                    serde_json::json!([
                                        sig.as_str(),
                                        {"encoding": "json", "commitment": "confirmed", "maxSupportedTransactionVersion": max_supported_transaction_version}
                                    ]),
                                )
                                .await;
                            (sig, slot, txn)
                        }
                    });
                    futures::stream::iter(futures_iter)
                        .buffer_unordered(batch_concurrency)
                        .collect()
                        .await
                };
                for (sig, slot, txn) in transaction_results {
                    match txn {
                        Ok(txn) => {
                            self.process_transaction(&sig, slot, &txn, targets, events, None)?;
                        }
                        Err(e) => {
                            tracing::warn!(signature = %sig, error = %e, "failed to fetch Solana transaction");
                        }
                    }
                }

                if reached_older_slots {
                    break;
                }
                if cursor_lost {
                    break;
                }
                let next_before = sig_array
                    .last()
                    .and_then(|entry| entry.get("signature"))
                    .and_then(|sig| sig.as_str())
                    .map(ToOwned::to_owned);
                if next_before == before {
                    tracing::warn!(address = %addr, ?before, "Solana pagination cursor did not advance");
                    break;
                }
                before = next_before;
                if before.is_none() {
                    break;
                }
            }
        }

        Ok(())
    }
}

fn expand_solana_scan_targets(targets: &TargetMap) -> TargetMap {
    let mut expanded = targets.clone();
    for (owner, assets) in targets {
        for asset in assets.iter().filter(|asset| asset.contract.is_some()) {
            let Some(mint) = asset.contract.as_deref() else {
                continue;
            };
            let token_programs: Vec<&str> = match asset.token_program.as_deref() {
                Some(token_program) => vec![token_program],
                None => vec![SPL_TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID],
            };
            for token_program in token_programs {
                match derive_associated_token_address(owner, mint, Some(token_program)) {
                    Ok(ata) => {
                        tracing::debug!(
                            %owner,
                            %mint,
                            %token_program,
                            %ata,
                            "derived Solana associated token account scan target"
                        );
                        let assets = expanded.entry(ata).or_default();
                        if !assets.iter().any(|existing| {
                            existing.symbol.eq_ignore_ascii_case(&asset.symbol)
                                && existing.contract == asset.contract
                                && existing.token_program == asset.token_program
                        }) {
                            assets.push(asset.clone());
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%owner, %mint, %token_program, %error, "failed to derive Solana associated token account");
                    }
                }
            }
        }
    }
    expanded
}

fn derive_associated_token_address(
    owner: &str,
    mint: &str,
    token_program: Option<&str>,
) -> anyhow::Result<String> {
    let owner = Pubkey::from_str(owner)?;
    let mint = Pubkey::from_str(mint)?;
    let token_program = Pubkey::from_str(token_program.unwrap_or(SPL_TOKEN_PROGRAM_ID))?;
    let associated_token_program = Pubkey::from_str(ASSOCIATED_TOKEN_PROGRAM_ID)?;
    let (ata, _) = Pubkey::find_program_address(
        &[owner.as_ref(), token_program.as_ref(), mint.as_ref()],
        &associated_token_program,
    );
    Ok(ata.to_string())
}

impl SolanaScanner {
    fn process_transaction(
        &self,
        tx_id: &str,
        slot: u64,
        tx: &serde_json::Value,
        targets: &TargetMap,
        events: &mut Vec<DepositEvent>,
        block_time_override: Option<i64>,
    ) -> anyhow::Result<()> {
        if tx.is_null() {
            tracing::warn!(signature = %tx_id, "getTransaction returned null, skipping transaction");
            return Ok(());
        }
        let block_time = block_time_override
            .or_else(|| tx.get("blockTime").and_then(|v| v.as_i64()))
            .unwrap_or(0);
        let timestamp = chrono::DateTime::from_timestamp(block_time, 0)
            .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .unwrap_or_default();
        let Some(native_asset) = self.chain.assets.iter().find(|a| a.contract.is_none()) else {
            return Ok(());
        };
        let meta = tx.get("meta");
        let message = tx.get("transaction").and_then(|t| t.get("message"));
        if let (Some(meta), Some(message)) = (meta, message) {
            if meta.get("err").is_some_and(|e| !e.is_null()) {
                return Ok(());
            }
            let pre_balances = meta
                .get("preBalances")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let post_balances = meta
                .get("postBalances")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let account_keys = message
                .get("accountKeys")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            // Best effort: this can select the fee payer instead of the actual sender
            // when priority fees dominate the SOL balance decrease.
            let from_addr = pre_balances
                .iter()
                .zip(post_balances.iter())
                .enumerate()
                .filter_map(|(i, (pre, post))| {
                    let pre = pre.as_u64()?;
                    let post = post.as_u64()?;
                    if pre > post {
                        Some((i, pre - post))
                    } else {
                        None
                    }
                })
                .max_by_key(|(_, diff)| *diff)
                .and_then(|(i, _)| account_keys.get(i))
                .and_then(account_key_pubkey)
                .unwrap_or("")
                .to_string();
            for (i, (pre, post)) in pre_balances.iter().zip(post_balances.iter()).enumerate() {
                let pre_val = pre.as_u64().unwrap_or(0);
                let post_val = post.as_u64().unwrap_or(0);
                if post_val <= pre_val {
                    continue;
                }
                let addr = account_keys
                    .get(i)
                    .and_then(account_key_pubkey)
                    .unwrap_or("")
                    .to_string();
                if !asset_allowed(targets, &addr, &native_asset.symbol) {
                    continue;
                }
                let amount = (post_val - pre_val).to_string();
                events.push(DepositEvent::detected(DepositData {
                    tx_id: tx_id.to_string(),
                    caip2: self.chain.caip2.clone(),
                    symbol: native_asset.symbol.clone(),
                    address: addr,
                    block_number: slot,
                    log_index: i as u64,
                    amount,
                    sender: from_addr.clone(),
                    confirmations: 1,
                    timestamp: timestamp.clone(),
                    internal_egress: None,
                })?);
            }

            let pre_tokens = meta
                .get("preTokenBalances")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let post_tokens = meta
                .get("postTokenBalances")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let mut token_log_index = 0_u64;
            for post in &post_tokens {
                let owner = post.get("owner").and_then(|v| v.as_str()).unwrap_or("");
                let mint = post.get("mint").and_then(|v| v.as_str()).unwrap_or("");

                // Retrieve custom/allowed assets active for this specific owner address
                let Some(allowed_assets) = targets.get(owner) else {
                    continue;
                };

                // Find matching custom or static asset from the TargetMap entries
                let Some(resolved_asset) = allowed_assets.iter().find(|ra| {
                    ra.contract
                        .as_deref()
                        .is_some_and(|c| c.eq_ignore_ascii_case(mint))
                }) else {
                    continue;
                };

                if !asset_allowed(targets, owner, &resolved_asset.symbol) {
                    continue;
                }
                let idx = post.get("accountIndex").and_then(|v| v.as_u64());
                let pre_amount = pre_tokens
                    .iter()
                    .find(|p| {
                        p.get("accountIndex").and_then(|v| v.as_u64()) == idx
                            && p.get("mint").and_then(|v| v.as_str()) == Some(mint)
                    })
                    .map(token_amount)
                    .unwrap_or(0);
                let post_amount = token_amount(post);
                if post_amount > pre_amount {
                    let from_addr = find_spl_sender(&pre_tokens, &post_tokens, mint, idx, owner);
                    let amount = (post_amount - pre_amount).to_string();
                    events.push(DepositEvent::detected(DepositData {
                        tx_id: tx_id.to_string(),
                        caip2: self.chain.caip2.clone(),
                        symbol: resolved_asset.symbol.clone(),
                        address: owner.to_string(),
                        block_number: slot,
                        log_index: token_log_index,
                        amount,
                        sender: from_addr,
                        confirmations: 1,
                        timestamp: timestamp.clone(),
                        internal_egress: None,
                    })?);
                    token_log_index += 1;
                }
            }
        }
        Ok(())
    }
}

/// Quick pre-filter: check if a getBlock transaction involves any watched address.
/// Avoids calling process_transaction for irrelevant txs.
fn tx_involves_watched(tx: &serde_json::Value, targets: &TargetMap) -> bool {
    // Check account keys
    if let Some(keys) = tx
        .pointer("/transaction/message/accountKeys")
        .and_then(|v| v.as_array())
    {
        for k in keys {
            let pk = k
                .as_str()
                .or_else(|| k.get("pubkey").and_then(|p| p.as_str()));
            if pk.is_some_and(|p| targets.contains_key(p)) {
                return true;
            }
        }
    }
    // Also check token balance owners (wallet may not be an account key)
    for field in ["preTokenBalances", "postTokenBalances"] {
        if let Some(tokens) = tx
            .get("meta")
            .and_then(|m| m.get(field))
            .and_then(|v| v.as_array())
        {
            for t in tokens {
                if t.get("owner")
                    .and_then(|v| v.as_str())
                    .is_some_and(|o| targets.contains_key(o))
                {
                    return true;
                }
            }
        }
    }
    false
}

fn token_amount(entry: &serde_json::Value) -> u128 {
    entry
        .pointer("/uiTokenAmount/amount")
        .and_then(|v| {
            v.as_str()
                .and_then(|amount| amount.parse::<u128>().ok())
                .or_else(|| v.as_u64().map(u128::from))
        })
        .unwrap_or(0)
}

fn account_key_pubkey(key: &serde_json::Value) -> Option<&str> {
    key.as_str()
        .or_else(|| key.get("pubkey").and_then(|pubkey| pubkey.as_str()))
}

fn is_solana_pruned_cursor_error(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("-32020") || message.contains("Transaction not found")
}

fn find_spl_sender(
    pre_tokens: &[serde_json::Value],
    post_tokens: &[serde_json::Value],
    mint: &str,
    receiver_idx: Option<u64>,
    receiver_owner: &str,
) -> String {
    for pre in pre_tokens {
        if pre.get("mint").and_then(|v| v.as_str()).unwrap_or("") != mint {
            continue;
        }
        let idx = pre.get("accountIndex").and_then(|v| v.as_u64());
        if idx == receiver_idx {
            continue;
        }
        let owner = pre.get("owner").and_then(|v| v.as_str()).unwrap_or("");
        if owner.is_empty() || owner == receiver_owner {
            continue;
        }
        let pre_amount = token_amount(pre);
        let post_amount = post_tokens
            .iter()
            .find(|post| post.get("accountIndex").and_then(|v| v.as_u64()) == idx)
            .map(token_amount)
            .unwrap_or(0);
        if pre_amount > post_amount {
            return owner.to_string();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ResolvedAsset;

    const OWNER: &str = "11111111111111111111111111111111";
    const MINT: &str = "So11111111111111111111111111111111111111112";

    fn token_targets(token_program: Option<String>) -> TargetMap {
        let mut targets = TargetMap::new();
        targets.insert(
            OWNER.to_string(),
            vec![ResolvedAsset {
                symbol: "TEST".to_string(),
                contract: Some(MINT.to_string()),
                token_program,
                decimals: Some(6),
            }],
        );
        targets
    }

    #[test]
    fn omitted_solana_token_program_scans_classic_and_token_2022_atas() {
        let expanded = expand_solana_scan_targets(&token_targets(None));
        let classic = derive_associated_token_address(OWNER, MINT, Some(SPL_TOKEN_PROGRAM_ID))
            .expect("classic ATA");
        let token_2022 = derive_associated_token_address(OWNER, MINT, Some(TOKEN_2022_PROGRAM_ID))
            .expect("Token-2022 ATA");

        assert!(expanded.contains_key(OWNER));
        assert!(expanded.contains_key(&classic));
        assert!(expanded.contains_key(&token_2022));
    }

    #[test]
    fn explicit_solana_token_program_scans_only_that_ata() {
        let expanded =
            expand_solana_scan_targets(&token_targets(Some(TOKEN_2022_PROGRAM_ID.to_string())));
        let classic = derive_associated_token_address(OWNER, MINT, Some(SPL_TOKEN_PROGRAM_ID))
            .expect("classic ATA");
        let token_2022 = derive_associated_token_address(OWNER, MINT, Some(TOKEN_2022_PROGRAM_ID))
            .expect("Token-2022 ATA");

        assert!(expanded.contains_key(OWNER));
        assert!(!expanded.contains_key(&classic));
        assert!(expanded.contains_key(&token_2022));
    }
}
