use crate::model::DepositEvent;
use crate::shared::format::{FileFormat, infer_format, serialize_event, serialize_events};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::LazyLock;
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast;

// ── Per-path write serialization for JSON files ──────────────────────────

#[derive(Debug, Clone, Default)]
pub struct FileWriteLocks {
    locks: Arc<tokio::sync::Mutex<hashbrown::HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
}

static SHARED_FILE_WRITE_LOCKS: LazyLock<FileWriteLocks> = LazyLock::new(FileWriteLocks::default);

pub fn shared_write_locks() -> FileWriteLocks {
    SHARED_FILE_WRITE_LOCKS.clone()
}

/// Resolve a stable canonical path for use as a lock key.
fn canonical_path(path: &str) -> String {
    std::fs::canonicalize(path)
        .ok()
        .and_then(|p| p.to_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| path.to_owned())
}

async fn with_json_file_lock_from<F, T>(locks: &FileWriteLocks, path: &str, f: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    let key = canonical_path(path);
    let lock: Arc<tokio::sync::Mutex<()>> = {
        let mut map = locks.locks.lock().await;
        map.entry(key)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    // Drop map lock before acquiring per-file lock to avoid holding it during I/O.
    let _guard = lock.lock().await;
    f.await
}

// ── Egress file configuration ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FileEgressConfig {
    pub enabled: bool,
    pub path: String,
}

/// Write deposit events to a file. Format inferred from extension.
pub async fn write_events(path: String, rx: &mut broadcast::Receiver<DepositEvent>) -> Result<()> {
    let locks = shared_write_locks();
    let format = infer_format(&path);
    let mut file = if format == FileFormat::Json {
        None
    } else {
        Some(
            tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await
                .with_context(|| format!("failed to open {path}"))?,
        )
    };
    loop {
        let Some(event) = super::recv_event(rx).await else {
            break;
        };

        if format == FileFormat::Json {
            if let Err(e) = write_event_to_path_with_locks(&locks, &path, &event).await {
                tracing::error!(error = %e, path = %path, "failed to write JSON file");
            }
        } else if let Some(file) = file.as_mut() {
            let line = match serialize_event(&event, format) {
                Ok(line) => line,
                Err(e) => {
                    tracing::error!(error = %e, event_id = %event.event_id, "failed to serialize event");
                    continue;
                }
            };
            if let Err(e) = file.write_all(format!("{line}\n").as_bytes()).await {
                tracing::error!(error = %e, path = %path, "failed to write to file");
                continue;
            }
            if let Err(e) = file.flush().await {
                tracing::error!(error = %e, path = %path, "failed to flush file");
            }
        }
    }
    Ok(())
}

pub async fn write_event_to_path(path: &str, event: &DepositEvent) -> Result<()> {
    let locks = shared_write_locks();
    write_event_to_path_with_locks(&locks, path, event).await
}

pub async fn write_event_to_path_with_locks(
    locks: &FileWriteLocks,
    path: &str,
    event: &DepositEvent,
) -> Result<()> {
    match infer_format(path) {
        FileFormat::Json => {
            with_json_file_lock_from(locks, path, async {
                let mut events: Vec<DepositEvent> = match tokio::fs::read_to_string(path).await {
                    Ok(content) if !content.trim().is_empty() => serde_json::from_str(&content)
                        .with_context(|| {
                            format!("failed to parse existing JSON event array in {path}")
                        })?,
                    _ => Vec::new(),
                };
                events.push(event.clone());
                let tmp_path = format!("{path}.tmp");
                tokio::fs::write(&tmp_path, serialize_events(&events, FileFormat::Json)?).await?;
                tokio::fs::rename(&tmp_path, path).await?;
                Ok::<_, anyhow::Error>(())
            })
            .await?;
        }
        format => {
            let line = serialize_event(event, format)?;
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .await?;
            file.write_all(format!("{line}\n").as_bytes()).await?;
            file.flush().await?;
        }
    }
    Ok(())
}
