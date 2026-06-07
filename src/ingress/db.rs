use crate::model::{EgressOverride, ResolvedWatch};
use anyhow::{Context, Result};

pub(crate) type WatchedAddressTuple = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

#[derive(Debug, Clone)]
pub(crate) struct WatchedAddressRow {
    pub(crate) address: String,
    pub(crate) caip2: String,
    pub(crate) symbol: String,
    pub(crate) asset_config: Option<String>,
    pub(crate) chain_config: Option<String>,
    pub(crate) egress_json: Option<String>,
}

impl From<WatchedAddressTuple> for WatchedAddressRow {
    fn from(
        (address, caip2, symbol, asset_config, chain_config, egress_json): WatchedAddressTuple,
    ) -> Self {
        Self {
            address,
            caip2,
            symbol,
            asset_config,
            chain_config,
            egress_json,
        }
    }
}

pub(crate) fn into_resolved(row: WatchedAddressRow) -> Result<ResolvedWatch> {
    let asset_val = parse_optional_json(row.asset_config.as_deref(), "asset_config")?;
    let chain_val = parse_optional_json(row.chain_config.as_deref(), "chain_config")?;
    let egress = row
        .egress_json
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| serde_json::from_str::<EgressOverride>(s).context("failed to parse egress JSON"))
        .transpose()?;

    let contract = asset_val.as_ref().and_then(|v| {
        v.get("contract")
            .and_then(|c| c.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
    });
    let decimals = asset_val
        .as_ref()
        .and_then(|v| v.get("decimals").and_then(|d| d.as_u64()).map(|d| d as u32));
    let token_program = asset_val.as_ref().and_then(|v| {
        v.get("token_program")
            .and_then(|c| c.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string())
    });
    let min_amount = asset_val.as_ref().and_then(|v| {
        v.get("min_amount")
            .and_then(|m| m.as_str())
            .map(|s| s.to_string())
    });
    let start_block = chain_val
        .as_ref()
        .and_then(|v| v.get("start_block").and_then(|b| b.as_u64()));
    let end_block = chain_val
        .as_ref()
        .and_then(|v| v.get("end_block").and_then(|b| b.as_u64()));
    let confirmed_blocks = chain_val
        .as_ref()
        .and_then(|v| v.get("confirmed_blocks").and_then(|b| b.as_u64()))
        .unwrap_or(0) as u32;

    Ok(ResolvedWatch {
        address: row.address,
        caip2: row.caip2,
        symbol: row.symbol,
        contract,
        token_program,
        decimals,
        start_block,
        end_block,
        confirmed_blocks,
        min_amount,
        egress,
    })
}

fn parse_optional_json(raw: Option<&str>, field: &str) -> Result<Option<serde_json::Value>> {
    raw.filter(|s| !s.trim().is_empty())
        .map(|s| serde_json::from_str(s).with_context(|| format!("failed to parse {field} JSON")))
        .transpose()
}
