use crate::chain::{ChainScanner, asset_allowed};
use crate::config::{AssetConfig, ChainConfig};
use crate::model::{DepositData, DepositEvent, TargetMap};
use crate::rpc::RpcClient;
use futures::StreamExt;
use num_bigint::BigUint;

const TRANSFER_TOPIC: &str = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
const ADDRESS_PADDING: &str = "000000000000000000000000";

/// EVM chain scanner (Ethereum, Base, Polygon, etc.).
pub struct EvmScanner {
    chain: ChainConfig,
    rpc: RpcClient,
}

impl EvmScanner {
    pub fn new(chain: ChainConfig) -> anyhow::Result<Self> {
        let rpc = RpcClient::new(chain.clone());
        Ok(Self { chain, rpc })
    }
}

#[async_trait::async_trait]
impl ChainScanner for EvmScanner {
    fn caip2(&self) -> &str {
        &self.chain.caip2
    }

    async fn get_tip(&self) -> anyhow::Result<u64> {
        tracing::debug!(caip2 = %self.chain.caip2, "EvmScanner::get_tip");
        let res = self
            .rpc
            .send("eth_blockNumber", serde_json::json!([]))
            .await?;
        let hex = res
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("eth_blockNumber returned non-string result: {res}"))?;
        parse_hex_u64(hex)
    }

    async fn scan(
        &self,
        from_block: u64,
        to_block: u64,
        targets: &TargetMap,
    ) -> anyhow::Result<Vec<DepositEvent>> {
        let mut events = Vec::new();
        if targets.is_empty() || from_block > to_block {
            return Ok(events);
        }

        tracing::debug!(
            from_block,
            to_block,
            target_count = targets.len(),
            "EvmScanner::scan starting"
        );

        let batch_size = self.chain.rpc_options_or_default().batch_size.max(1);

        // Collect contracts from static config
        let mut contract_assets: hashbrown::HashMap<String, AssetConfig> = self
            .chain
            .assets
            .iter()
            .filter_map(|a| {
                a.contract
                    .as_ref()
                    .filter(|c| !c.trim().is_empty())
                    .map(|c| {
                        let lower = c.to_lowercase();
                        (lower, a.clone())
                    })
            })
            .collect();

        // Merge custom assets from runtime TargetMap (not in static config)
        for (_addr, assets) in targets.iter() {
            for ra in assets {
                if let Some(contract) = &ra.contract {
                    if contract.trim().is_empty() {
                        continue;
                    }
                    let lower = contract.to_lowercase();
                    if !contract_assets.contains_key(&lower) {
                        let decimals = ra.decimals.ok_or_else(|| {
                            anyhow::anyhow!(
                                "custom runtime ERC-20 asset {} ({}) is missing decimals; \
                                 decimals are required for runtime assets not present in the static chain config",
                                ra.symbol,
                                contract
                            )
                        })?;
                        contract_assets.insert(
                            lower,
                            AssetConfig {
                                symbol: ra.symbol.clone(),
                                contract: Some(contract.clone()),
                                token_program: None,
                                decimals,
                                min_amount: None,
                            },
                        );
                    }
                }
            }
        }

        let mut active_contracts: Vec<String> = contract_assets
            .iter()
            .filter(|(_, asset)| targets_allow_symbol(targets, &asset.symbol))
            .map(|(contract, _)| contract.clone())
            .collect();
        active_contracts.sort_unstable();
        let native_symbol = self
            .chain
            .assets
            .iter()
            .find(|asset| asset.contract.is_none())
            .map(|asset| asset.symbol.as_str());
        let wants_native =
            native_symbol.is_some_and(|symbol| targets_allow_any(targets, &[symbol]));

        for current_start in (from_block..=to_block).step_by(batch_size as usize) {
            let current_end = std::cmp::min(
                current_start.saturating_add(batch_size).saturating_sub(1),
                to_block,
            );
            if !active_contracts.is_empty() {
                let log_values = self
                    .fetch_erc20_logs(&active_contracts, current_start, current_end)
                    .await?;
                let mut missing_blocks: Vec<u64> = log_values
                    .iter()
                    .filter_map(|log| {
                        log.get("blockNumber")
                            .and_then(|v| v.as_str())
                            .and_then(|s| parse_hex_u64(s).ok())
                    })
                    .collect();
                missing_blocks.sort_unstable();
                missing_blocks.dedup();
                let timestamp_results = futures::future::join_all(
                    missing_blocks.into_iter().map(|block_number| async move {
                        let timestamp = self.block_timestamp(block_number).await.unwrap_or_else(|e| {
                            tracing::warn!(block_number, error = %e, "failed to fetch ERC-20 block timestamp");
                            String::new()
                        });
                        (block_number, timestamp)
                    }),
                )
                .await;
                let timestamp_cache: hashbrown::HashMap<u64, String> =
                    timestamp_results.into_iter().collect();
                for log in &log_values {
                    if log
                        .get("removed")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    let topics = log.get("topics").and_then(|v| v.as_array());
                    let Some(topics) = topics else {
                        continue;
                    };
                    if topics.len() < 3 {
                        continue;
                    }
                    let sender = match extract_topic_address(&topics[1]) {
                        Some(addr) => addr,
                        None => {
                            tracing::warn!(topic = %topics[1], "skipping ERC-20 log with malformed sender topic");
                            continue;
                        }
                    };
                    let to = match extract_topic_address(&topics[2]) {
                        Some(addr) => addr,
                        None => {
                            tracing::warn!(topic = %topics[2], "skipping ERC-20 log with malformed recipient topic");
                            continue;
                        }
                    };
                    let log_addr = log
                        .get("address")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_lowercase();
                    let Some(asset) = contract_assets.get(&log_addr) else {
                        continue;
                    };
                    if !asset_allowed(targets, &to, &asset.symbol) {
                        continue;
                    }
                    let raw = log.get("data").and_then(|v| v.as_str()).unwrap_or("0x0");
                    let Some(amount) = parse_hex_uint(raw) else {
                        tracing::warn!(raw, "skipping ERC-20 log with invalid amount");
                        continue;
                    };
                    if amount == "0" {
                        continue;
                    }
                    let block_number = match parse_hex_u64(
                        log.get("blockNumber")
                            .and_then(|v| v.as_str())
                            .unwrap_or("0x0"),
                    ) {
                        Ok(n) => n,
                        Err(e) => {
                            tracing::warn!(error = %e, "skipping ERC-20 log with invalid block number");
                            continue;
                        }
                    };
                    let log_index = log
                        .get("logIndex")
                        .and_then(|v| v.as_str())
                        .and_then(|s| parse_hex_u64(s).ok())
                        .unwrap_or(0);
                    let timestamp = timestamp_cache
                        .get(&block_number)
                        .cloned()
                        .unwrap_or_default();
                    events.push(DepositEvent::detected(DepositData {
                        tx_id: log
                            .get("transactionHash")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        caip2: self.chain.caip2.clone(),
                        symbol: asset.symbol.clone(),
                        address: to,
                        block_number,
                        log_index,
                        amount,
                        sender,
                        confirmations: 1,
                        timestamp,
                        internal_egress: None,
                    })?);
                }
            }

            tracing::debug!(
                erc20_events = events.len(),
                "EvmScanner ERC-20 scan complete"
            );
            if wants_native {
                self.scan_native_batch(current_start, current_end, targets, &mut events)
                    .await?;
            }
        }

        Ok(events)
    }
}

impl EvmScanner {
    async fn fetch_erc20_logs(
        &self,
        contracts: &[String],
        from_block: u64,
        to_block: u64,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        if contracts.is_empty() {
            return Ok(Vec::new());
        }

        let use_address_batching = self.chain.rpc_options_or_default().evm_log_address_batching;
        if use_address_batching {
            match self
                .fetch_erc20_logs_once(contracts, from_block, to_block)
                .await
            {
                Ok(logs) => return Ok(logs),
                Err(error) if contracts.len() > 1 => {
                    tracing::warn!(
                        caip2 = %self.chain.caip2,
                        contract_count = contracts.len(),
                        %from_block,
                        %to_block,
                        error = %error,
                        "batched eth_getLogs failed; retrying per contract"
                    );
                }
                Err(error) => return Err(error),
            }
        }

        let mut logs = Vec::new();
        for contract in contracts {
            logs.extend(
                self.fetch_erc20_logs_once(std::slice::from_ref(contract), from_block, to_block)
                    .await?,
            );
        }
        Ok(logs)
    }

    async fn fetch_erc20_logs_once(
        &self,
        contracts: &[String],
        from_block: u64,
        to_block: u64,
    ) -> anyhow::Result<Vec<serde_json::Value>> {
        let address = if contracts.len() == 1 {
            serde_json::json!(contracts[0])
        } else {
            serde_json::json!(contracts)
        };
        let payload = serde_json::json!([{
            "address": address,
            "fromBlock": format!("0x{from_block:x}"),
            "toBlock": format!("0x{to_block:x}"),
            "topics": [TRANSFER_TOPIC]
        }]);
        let logs = self.rpc.send("eth_getLogs", payload).await?;
        let Some(log_array) = logs.as_array() else {
            tracing::debug!(
                caip2 = %self.chain.caip2,
                contract_count = contracts.len(),
                %from_block,
                %to_block,
                "eth_getLogs returned non-array result; treating as empty"
            );
            return Ok(Vec::new());
        };
        tracing::debug!(
            caip2 = %self.chain.caip2,
            contract_count = contracts.len(),
            %from_block,
            %to_block,
            log_count = log_array.len(),
            "eth_getLogs returned"
        );
        Ok(log_array.clone())
    }

    async fn scan_native_batch(
        &self,
        from_block: u64,
        to_block: u64,
        targets: &TargetMap,
        events: &mut Vec<DepositEvent>,
    ) -> anyhow::Result<()> {
        let Some(asset) = self.chain.assets.iter().find(|a| a.contract.is_none()) else {
            return Ok(());
        };

        let fetch_concurrency = self.chain.rpc_options_or_default().max_concurrent;
        let mut block_results =
            futures::stream::iter((from_block..=to_block).map(|block_num| async move {
                let block = self
                    .rpc
                    .send(
                        "eth_getBlockByNumber",
                        serde_json::json!([format!("0x{block_num:x}"), true]),
                    )
                    .await;
                (block_num, block)
            }))
            .buffer_unordered(fetch_concurrency);

        while let Some((block_num, block)) = block_results.next().await {
            let block = block?;

            if block.is_null() {
                anyhow::bail!("eth_getBlockByNumber returned null for block {block_num}");
            }
            let timestamp_hex = block
                .get("timestamp")
                .and_then(|v| v.as_str())
                .unwrap_or("0x0");
            let timestamp_secs = parse_hex_u64(timestamp_hex)?;
            if timestamp_secs > i64::MAX as u64 {
                anyhow::bail!("block {block_num} timestamp {timestamp_secs} exceeds i64 range");
            }
            let timestamp = chrono::DateTime::from_timestamp(timestamp_secs as i64, 0)
                .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
                .unwrap_or_default();

            let transactions = block
                .get("transactions")
                .and_then(|t| t.as_array())
                .cloned()
                .unwrap_or_default();

            for tx in transactions {
                let to_addr = tx
                    .get("to")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_lowercase();
                let from_addr = tx
                    .get("from")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_lowercase();
                let value = tx.get("value").and_then(|v| v.as_str()).unwrap_or("0x0");
                let hash = tx.get("hash").and_then(|v| v.as_str()).unwrap_or("");

                let Some(amount) = parse_hex_uint(value) else {
                    tracing::warn!(value, "skipping native transaction with invalid amount");
                    continue;
                };
                if amount != "0"
                    && !to_addr.is_empty()
                    && asset_allowed(targets, &to_addr, &asset.symbol)
                {
                    let data = DepositData {
                        tx_id: hash.to_string(),
                        caip2: self.chain.caip2.clone(),
                        symbol: asset.symbol.clone(),
                        address: to_addr,
                        block_number: block_num,
                        log_index: 0,
                        amount,
                        sender: from_addr,
                        confirmations: 1,
                        timestamp: timestamp.clone(),
                        internal_egress: None,
                    };
                    events.push(DepositEvent::detected(data)?);
                }
            }
        }

        Ok(())
    }
}

impl EvmScanner {
    async fn block_timestamp(&self, block_num: u64) -> anyhow::Result<String> {
        let block = self
            .rpc
            .send(
                "eth_getBlockByNumber",
                serde_json::json!([format!("0x{block_num:x}"), false]),
            )
            .await?;
        if block.is_null() {
            anyhow::bail!("eth_getBlockByNumber returned null for block {block_num}");
        }
        let timestamp_hex = block
            .get("timestamp")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing timestamp for block {block_num}"))?;
        let secs = parse_hex_u64(timestamp_hex)?;
        if secs > i64::MAX as u64 {
            anyhow::bail!("block {block_num} timestamp {secs} exceeds i64 range");
        }
        Ok(chrono::DateTime::from_timestamp(secs as i64, 0)
            .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .unwrap_or_default())
    }
}

pub fn parse_hex_u64(raw: &str) -> anyhow::Result<u64> {
    let hex = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(raw);
    if hex.is_empty() {
        anyhow::bail!("empty hex string");
    }
    u64::from_str_radix(hex, 16).map_err(|e| anyhow::anyhow!("invalid hex u64 {raw:?}: {e}"))
}

pub fn parse_hex_uint(raw: &str) -> Option<String> {
    let hex = raw
        .strip_prefix("0x")
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(raw);
    if hex.is_empty() {
        return Some("0".to_string());
    }
    BigUint::parse_bytes(hex.as_bytes(), 16).map(|v| v.to_string())
}

pub fn extract_topic_address(topic: &serde_json::Value) -> Option<String> {
    let raw = topic.as_str()?.to_lowercase();
    if raw.len() == 66 && raw.starts_with("0x") && &raw[2..26] == ADDRESS_PADDING {
        let addr = &raw[26..];
        if addr.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(format!("0x{addr}"));
        }
    }
    None
}

fn targets_allow_any<S: AsRef<str>>(targets: &TargetMap, symbols: &[S]) -> bool {
    !symbols.is_empty()
        && targets.values().any(|assets| {
            assets.iter().any(|ra| {
                symbols
                    .iter()
                    .any(|symbol| ra.symbol.eq_ignore_ascii_case(symbol.as_ref()))
            })
        })
}

fn targets_allow_symbol(targets: &TargetMap, symbol: &str) -> bool {
    targets.values().any(|assets| {
        assets
            .iter()
            .any(|asset| asset.symbol.eq_ignore_ascii_case(symbol))
    })
}
