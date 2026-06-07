use crate::chain::{ChainScanner, asset_allowed};
use crate::config::ChainConfig;
use crate::model::{DepositData, DepositEvent, TargetMap};
use crate::rpc::RpcClient;

/// Bitcoin chain scanner.
pub struct BtcScanner {
    chain: ChainConfig,
    rpc: RpcClient,
}

impl BtcScanner {
    pub fn new(chain: ChainConfig) -> anyhow::Result<Self> {
        let rpc = RpcClient::new(chain.clone());
        Ok(Self { chain, rpc })
    }
}

#[async_trait::async_trait]
impl ChainScanner for BtcScanner {
    fn caip2(&self) -> &str {
        &self.chain.caip2
    }

    async fn get_tip(&self) -> anyhow::Result<u64> {
        let res = self
            .rpc
            .send("getblockcount", serde_json::json!([]))
            .await?;
        res.as_u64()
            .ok_or_else(|| anyhow::anyhow!("getblockcount returned non-u64 result: {res}"))
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
        let Some(native_asset) = self.chain.assets.iter().find(|a| a.contract.is_none()) else {
            return Ok(events);
        };

        for block_height in from_block..=to_block {
            let block_hash = self.block_hash(block_height).await?;

            let block = self
                .rpc
                .send("getblock", serde_json::json!([&block_hash, 2]))
                .await?;

            let block_time = block.get("time").and_then(|v| v.as_i64()).unwrap_or(0);
            let timestamp = chrono::DateTime::from_timestamp(block_time, 0)
                .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
                .unwrap_or_default();

            let txs = block
                .get("tx")
                .and_then(|t| t.as_array())
                .cloned()
                .unwrap_or_default();

            for tx in txs {
                let txid = tx.get("txid").and_then(|v| v.as_str()).unwrap_or("");
                let sender = first_input_address(&tx);
                let vout = tx
                    .get("vout")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();

                for output in vout {
                    let log_index = output.get("n").and_then(|v| v.as_u64()).unwrap_or(0);
                    let addresses: Vec<String> = output
                        .get("scriptPubKey")
                        .and_then(|s| {
                            s.get("address")
                                .and_then(|a| a.as_str())
                                .map(|a| vec![a.to_string()])
                                .or_else(|| {
                                    s.get("addresses").and_then(|a| a.as_array()).map(|arr| {
                                        arr.iter()
                                            .filter_map(|a| a.as_str().map(ToOwned::to_owned))
                                            .collect()
                                    })
                                })
                        })
                        .unwrap_or_default();
                    let exact_val_str = output
                        .get("value")
                        .and_then(|v| {
                            v.as_str()
                                .map(|s| s.to_string())
                                .or_else(|| v.as_number().map(|n| n.to_string()))
                        })
                        .unwrap_or_else(|| "0".to_string());
                    let satoshis = match btc_to_sats(&exact_val_str) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::warn!(error = %e, tx_id = %txid, "invalid BTC amount, skipping output");
                            continue;
                        }
                    };

                    if satoshis != "0" {
                        for address in addresses
                            .iter()
                            .filter(|address| asset_allowed(targets, address, &native_asset.symbol))
                        {
                            let data = DepositData {
                                tx_id: txid.to_string(),
                                caip2: self.chain.caip2.clone(),
                                symbol: native_asset.symbol.clone(),
                                address: address.to_string(),
                                block_number: block_height,
                                log_index,
                                amount: satoshis.clone(),
                                sender: sender.clone(),
                                confirmations: 1,
                                timestamp: timestamp.clone(),
                                internal_egress: None,
                            };
                            events.push(DepositEvent::detected(data)?);
                        }
                    }
                }
            }
        }

        Ok(events)
    }
}

impl BtcScanner {
    async fn block_hash(&self, block_height: u64) -> anyhow::Result<String> {
        let hash = self
            .rpc
            .send("getblockhash", serde_json::json!([block_height]))
            .await?;
        hash.as_str()
            .ok_or_else(|| anyhow::anyhow!("getblockhash returned non-string result: {hash}"))
            .map(|s| s.to_string())
    }
}

pub fn btc_to_sats(value_btc: &str) -> anyhow::Result<String> {
    use rust_decimal::Decimal;
    use std::str::FromStr;
    let max_satoshis = Decimal::new(2_100_000_000_000_000i64, 0);
    let decimal_val = Decimal::from_str(value_btc)
        .or_else(|_| Decimal::from_scientific(value_btc))
        .map_err(|e| anyhow::anyhow!("invalid BTC amount '{value_btc}': {e}"))?;
    if decimal_val.is_sign_negative() {
        anyhow::bail!("negative BTC amount: {value_btc}");
    }
    let satoshis = (decimal_val * Decimal::new(100_000_000, 0)).round();
    if satoshis > max_satoshis {
        anyhow::bail!("BTC amount exceeds maximum supply: {value_btc}");
    }
    Ok(satoshis.to_string())
}

pub fn first_input_address(tx: &serde_json::Value) -> String {
    tx.get("vin")
        .and_then(|vin| vin.as_array())
        .and_then(|vin| vin.first())
        .and_then(|input| {
            input
                .pointer("/prevout/scriptPubKey/address")
                .and_then(|address| address.as_str())
                .or_else(|| {
                    input
                        .pointer("/prevout/scriptPubKey/addresses/0")
                        .and_then(|address| address.as_str())
                })
        })
        .unwrap_or("")
        .to_string()
}
