pub mod file;
pub mod pg;
pub mod queue;
pub mod sqlite;
pub mod sse;
pub mod webhook;
pub mod ws;

use crate::config::EgressConfig;
use crate::model::DepositEvent;
use anyhow::Result;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

/// Handle for broadcasting deposit events to all enabled egress targets.
#[derive(Debug, Clone)]
pub struct EgressHandle {
    pub event_tx: broadcast::Sender<DepositEvent>,
}

/// Receive the next event from a broadcast channel, handling lag gracefully.
/// Returns `None` when the channel is closed.
pub async fn recv_event(rx: &mut broadcast::Receiver<DepositEvent>) -> Option<DepositEvent> {
    loop {
        match rx.recv().await {
            Ok(ev) => return Some(ev),
            Err(broadcast::error::RecvError::Lagged(missed)) => {
                tracing::warn!(missed, "broadcast receiver lagging, skipping missed events");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => return None,
        }
    }
}

/// Start all enabled egress targets and return their tasks for shutdown waits.
/// Per-address delivery overrides are handled by async detector-owned delivery
/// workers, NOT from this broadcast layer.
pub fn start_with_tasks(config: EgressConfig) -> Result<(EgressHandle, Vec<JoinHandle<()>>)> {
    let (event_tx, _) = broadcast::channel::<DepositEvent>(config.broadcast_capacity.max(1));
    let mut tasks = Vec::new();

    if config.file.enabled {
        let mut rx = event_tx.subscribe();
        let path = config.file.path.clone();
        tasks.push(tokio::spawn(async move {
            if let Err(e) = file::write_events(path, &mut rx).await {
                tracing::error!(error = %e, "file egress failed");
            }
        }));
    }

    #[cfg(feature = "sqlite")]
    if config.sqlite.enabled {
        let mut rx = event_tx.subscribe();
        let cfg = config.sqlite.clone();
        tasks.push(tokio::spawn(async move {
            if let Err(e) = sqlite::write_events(cfg, &mut rx).await {
                tracing::error!(error = %e, "sqlite egress failed");
            }
        }));
    }

    #[cfg(feature = "postgres")]
    if config.pg.enabled {
        let mut rx = event_tx.subscribe();
        let cfg = config.pg.clone();
        tasks.push(tokio::spawn(async move {
            if let Err(e) = pg::write_events(cfg, &mut rx).await {
                tracing::error!(error = %e, "pg egress failed");
            }
        }));
    }

    #[cfg(feature = "amqp")]
    if config.queue.enabled {
        let mut rx = event_tx.subscribe();
        let queue_cfg = config.queue.clone();
        tasks.push(tokio::spawn(async move {
            if let Err(e) = queue::publish(queue_cfg, &mut rx).await {
                tracing::error!(error = %e, "queue egress failed");
            }
        }));
    }

    #[cfg(feature = "webhook")]
    if config.webhook.enabled {
        let mut rx = event_tx.subscribe();
        let webhook_cfg = config.webhook.clone();
        tasks.push(tokio::spawn(async move {
            if let Err(e) = webhook::deliver(webhook_cfg, &mut rx).await {
                tracing::error!(error = %e, "webhook egress failed");
            }
        }));
    }

    // SSE and WebSocket egress are handled by router routes — no spawn needed here.

    Ok((EgressHandle { event_tx }, tasks))
}
