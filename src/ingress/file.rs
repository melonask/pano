use crate::config::AppConfig;
use crate::detector::resolve_watch_spec_to_watches;
use crate::model::{Command, ResolvedWatch, WatchSpec};
use crate::shared::format::{FileFormat, infer_format};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

// ── Ingress file configuration ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FileIngressConfig {
    pub enabled: bool,
    pub path: String,
    pub poll_interval_secs: u64,
    pub authoritative: bool,
}

impl Default for FileIngressConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: String::new(),
            poll_interval_secs: 5,
            authoritative: true,
        }
    }
}

// ── Implementation ──────────────────────────────────────────────────────

/// Watch a file for address entries. Format inferred from extension.
pub async fn watch(
    path: String,
    tx: mpsc::Sender<Command>,
    config: AppConfig,
    poll_interval_secs: u64,
    authoritative: bool,
) -> Result<()> {
    let format = infer_format(&path);
    let mut last_modified = std::time::SystemTime::UNIX_EPOCH;
    let mut previous_specs: Option<Vec<WatchSpec>> = None;
    let mut missing_file_warned = false;
    let poll_interval = std::time::Duration::from_secs(poll_interval_secs.max(1));
    loop {
        if tx.is_closed() {
            return Ok(());
        }
        if let Ok(metadata) = tokio::fs::metadata(&path).await {
            missing_file_warned = false;
            let modified = metadata
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            if modified > last_modified {
                if let Err(e) = load_file_with_mode(
                    &path,
                    format,
                    &tx,
                    &config,
                    authoritative,
                    &mut previous_specs,
                )
                .await
                {
                    tracing::error!(error = %e, path = %path, "failed to load address file");
                } else {
                    last_modified = modified;
                }
            }
        } else if !missing_file_warned {
            tracing::warn!(
                path = %path,
                "ingress file does not exist yet; no watched addresses loaded from file ingress"
            );
            missing_file_warned = true;
        }
        tokio::select! {
            _ = tx.closed() => return Ok(()),
            _ = tokio::time::sleep(poll_interval) => {}
        }
    }
}

async fn load_file_with_mode(
    path: &str,
    format: FileFormat,
    tx: &mpsc::Sender<Command>,
    config: &AppConfig,
    authoritative: bool,
    previous_specs: &mut Option<Vec<WatchSpec>>,
) -> Result<()> {
    let content = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read {path}"))?;
    let specs = parse_specs(&content, format)?;

    if authoritative {
        let (resolved, valid_specs, rejected) = resolve_specs_partial(config, &specs, path);
        if rejected > 0 {
            tracing::warn!(
                path,
                rejected,
                accepted = valid_specs.len(),
                "file ingress skipped invalid watch specs"
            );
        }
        if !specs.is_empty() && resolved.is_empty() && rejected > 0 {
            anyhow::bail!(
                "all {rejected} watch specs in {path} were rejected; file ingress did not update watched addresses"
            );
        }
        if tx.send(Command::SyncAll(resolved)).await.is_err() {
            tracing::debug!("file ingress receiver closed");
            return Ok(());
        }
        *previous_specs = Some(valid_specs);
    } else {
        if let Some(previous) = previous_specs.as_ref() {
            let previous_addresses = resolved_addresses(config, previous, path);
            let current_addresses = resolved_addresses(config, &specs, path);
            for address in previous_addresses.difference(&current_addresses) {
                if tx
                    .send(Command::Unwatch {
                        address: address.clone(),
                    })
                    .await
                    .is_err()
                {
                    tracing::debug!("file ingress receiver closed");
                    return Ok(());
                }
            }
        }
        let mut valid_specs = Vec::new();
        for spec in specs.iter().filter(|spec| {
            previous_specs
                .as_ref()
                .is_none_or(|previous| !previous.contains(spec))
        }) {
            if let Err(error) = resolve_watch_spec_to_watches(config, spec) {
                tracing::error!(path, error = %error, "file ingress skipped invalid watch spec");
                continue;
            }
            if tx
                .send(Command::Watch(Box::new(spec.clone())))
                .await
                .is_err()
            {
                tracing::debug!("file ingress receiver closed");
                return Ok(());
            }
        }
        for spec in &specs {
            if resolve_watch_spec_to_watches(config, spec).is_ok() {
                valid_specs.push(spec.clone());
            }
        }
        *previous_specs = Some(valid_specs);
    }
    Ok(())
}

fn resolved_addresses(
    config: &AppConfig,
    specs: &[WatchSpec],
    path: &str,
) -> hashbrown::HashSet<String> {
    let mut addresses = hashbrown::HashSet::new();
    for (idx, spec) in specs.iter().enumerate() {
        match resolve_watch_spec_to_watches(config, spec) {
            Ok(watches) => {
                for watch in watches {
                    addresses.insert(watch.address);
                }
            }
            Err(error) => {
                tracing::error!(path, spec = idx + 1, error = %error, "file ingress skipped invalid watch spec while calculating removals");
            }
        }
    }
    addresses
}

fn resolve_specs_partial(
    config: &AppConfig,
    specs: &[WatchSpec],
    path: &str,
) -> (Vec<ResolvedWatch>, Vec<WatchSpec>, usize) {
    let mut resolved = Vec::new();
    let mut valid_specs = Vec::new();
    let mut rejected = 0;
    for (idx, spec) in specs.iter().enumerate() {
        match resolve_watch_spec_to_watches(config, spec) {
            Ok(watches) => {
                resolved.extend(watches);
                valid_specs.push(spec.clone());
            }
            Err(error) => {
                rejected += 1;
                tracing::error!(path, spec = idx + 1, error = %error, "file ingress skipped invalid watch spec");
            }
        }
    }
    (resolved, valid_specs, rejected)
}

fn parse_specs(content: &str, format: FileFormat) -> Result<Vec<WatchSpec>> {
    let mut batch = Vec::new();
    match format {
        FileFormat::Json => {
            if !content.trim().is_empty() {
                batch = serde_json::from_str(content)
                    .context("failed to parse JSON WatchSpec array")?;
            }
        }
        FileFormat::Jsonl => {
            for (line_no, line) in content.lines().enumerate() {
                if line.trim().is_empty() {
                    continue;
                }
                match serde_json::from_str::<WatchSpec>(line) {
                    Ok(spec) => batch.push(spec),
                    Err(e) => {
                        anyhow::bail!(
                            "malformed JSONL WatchSpec entry at line {}: {e}",
                            line_no + 1
                        );
                    }
                }
            }
        }
        FileFormat::Csv => {
            let mut reader = csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(content.as_bytes());
            for (row_no, result) in reader.records().enumerate() {
                let record = result
                    .with_context(|| format!("malformed CSV row at position {}", row_no + 1))?;
                let address = record.get(0).unwrap_or_default().trim().to_string();
                if address.is_empty() {
                    continue;
                }
                let chains = record
                    .get(1)
                    .filter(|s| !s.trim().is_empty())
                    .map(|raw| {
                        serde_json::from_str::<Vec<crate::model::ChainEntry>>(raw).with_context(
                            || {
                                format!(
                                    "malformed CSV chains JSON at row {} for address {address}",
                                    row_no + 1
                                )
                            },
                        )
                    })
                    .transpose()?
                    .unwrap_or_default();
                let egress = record
                    .get(2)
                    .filter(|s| !s.trim().is_empty())
                    .map(|raw| {
                        serde_json::from_str(raw).with_context(|| {
                            format!(
                                "malformed CSV egress JSON at row {} for address {address}",
                                row_no + 1
                            )
                        })
                    })
                    .transpose()?;
                batch.push(WatchSpec {
                    address: Some(address),
                    chains,
                    egress,
                });
            }
        }
    }
    Ok(batch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        AssetConfig, ChainConfig, DetectorConfig, EgressConfig, IngressConfig, OverrideConfig,
        RpcOptions, ServerConfig,
    };

    #[test]
    fn csv_parse_treats_first_row_as_data() {
        let specs = parse_specs("addr1\naddr2\n", FileFormat::Csv).unwrap();

        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].address.as_deref(), Some("addr1"));
        assert_eq!(specs[1].address.as_deref(), Some("addr2"));
    }

    #[test]
    fn partial_resolution_skips_invalid_specs() {
        let config = AppConfig {
            server: ServerConfig {
                enabled: false,
                bind: String::new(),
                port: 0,
                prefix: String::new(),
                dashboard: String::new(),
                dashboard_export: false,
                api_key: String::new(),
                shutdown_timeout_secs: 1,
            },
            detector: DetectorConfig::default(),
            chains: vec![ChainConfig {
                caip2: "eip155:1".to_string(),
                start_block: None,
                end_block: None,
                confirmed_blocks: 12,
                rpc: vec!["http://127.0.0.1:8545".to_string()],
                rpc_options: Some(RpcOptions::default()),
                assets: vec![AssetConfig {
                    symbol: "ETH".to_string(),
                    contract: None,
                    token_program: None,
                    decimals: 18,
                    min_amount: None,
                }],
            }],
            ingress: IngressConfig::default(),
            egress: EgressConfig::default(),
            override_: OverrideConfig::default(),
        };
        let valid = WatchSpec {
            address: Some("0xabcdef1234567890abcdef1234567890abcdef12".to_string()),
            chains: vec![],
            egress: None,
        };
        let invalid = WatchSpec {
            address: Some("not-an-evm-address".to_string()),
            chains: vec![],
            egress: None,
        };

        let (resolved, valid_specs, rejected) =
            resolve_specs_partial(&config, &[valid.clone(), invalid], "watched.jsonl");

        assert_eq!(resolved.len(), 1);
        assert_eq!(valid_specs, vec![valid]);
        assert_eq!(rejected, 1);
    }
}
