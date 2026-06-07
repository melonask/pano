pub mod api;
pub(crate) mod db;
pub mod file;
pub mod pg;
pub mod queue;
pub mod sqlite;

use crate::config::{AppConfig, IngressConfig};
use crate::model::Command;
use anyhow::Result;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Handle for sending watched addresses into the detector.
#[derive(Debug)]
pub struct IngressHandle {
    pub command_rx: mpsc::Receiver<Command>,
}

/// Start all enabled ingress sources and return their tasks for shutdown waits.
pub fn start_with_tasks(
    config: IngressConfig,
    app_config: Option<AppConfig>,
) -> Result<(IngressHandle, Vec<JoinHandle<()>>)> {
    let (command_tx, command_rx) = mpsc::channel::<Command>(config.command_queue_capacity.max(1));
    let mut tasks = Vec::new();

    if config.file.enabled {
        let Some(app_config) = app_config.clone() else {
            anyhow::bail!("file ingress requires full app config for watch resolution");
        };
        let tx = command_tx.clone();
        let path = config.file.path.clone();
        let poll_interval_secs = config.file.poll_interval_secs;
        let authoritative = config.file.authoritative;
        tasks.push(tokio::spawn(async move {
            if let Err(e) =
                file::watch(path, tx, app_config, poll_interval_secs, authoritative).await
            {
                tracing::error!(error = %e, "file ingress failed");
            }
        }));
    }

    if config.sqlite.enabled {
        let tx = command_tx.clone();
        let cfg = config.sqlite.clone();
        tasks.push(tokio::spawn(async move {
            let result = if cfg.poll_interval_secs == 0 {
                sqlite::load(cfg, tx).await
            } else {
                sqlite::watch(cfg, tx).await
            };
            if let Err(e) = result {
                tracing::error!(error = %e, "sqlite ingress failed");
            }
        }));
    }

    if config.pg.enabled {
        let tx = command_tx.clone();
        let cfg = config.pg.clone();
        tasks.push(tokio::spawn(async move {
            let result = if cfg.poll_interval_secs == 0 {
                pg::load(cfg, tx).await
            } else {
                pg::watch(cfg, tx).await
            };
            if let Err(e) = result {
                tracing::error!(error = %e, "pg ingress failed");
            }
        }));
    }

    if config.queue.enabled {
        let tx = command_tx.clone();
        let queue_cfg = config.queue.clone();
        tasks.push(tokio::spawn(async move {
            if let Err(e) = queue::consume(queue_cfg, tx).await {
                tracing::error!(error = %e, "queue ingress failed");
            }
        }));
    }

    Ok((IngressHandle { command_rx }, tasks))
}
