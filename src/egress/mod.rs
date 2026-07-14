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
use std::fmt;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

/// A bounded, lossless queue feeding one durable or external egress adapter.
#[derive(Debug, Clone)]
pub struct PersistentSinkSender {
    name: &'static str,
    sender: mpsc::Sender<DepositEvent>,
}

impl PersistentSinkSender {
    pub fn new(name: &'static str, sender: mpsc::Sender<DepositEvent>) -> Self {
        Self { name, sender }
    }
}

/// Returned when one or more configured persistent adapters have stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressPublishError {
    pub closed_sinks: Vec<&'static str>,
}

impl fmt::Display for EgressPublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "persistent egress sink channel(s) closed: {}",
            self.closed_sinks.join(", ")
        )
    }
}

impl std::error::Error for EgressPublishError {}

/// Handle for publishing detector events to durable sinks and the lossy stream.
#[derive(Debug, Clone)]
pub struct EgressHandle {
    stream_tx: broadcast::Sender<DepositEvent>,
    persistent_sinks: Vec<PersistentSinkSender>,
}

impl EgressHandle {
    pub fn new(
        stream_tx: broadcast::Sender<DepositEvent>,
        persistent_sinks: Vec<PersistentSinkSender>,
    ) -> Self {
        Self {
            stream_tx,
            persistent_sinks,
        }
    }

    /// The bounded broadcast sender used only by SSE/WebSocket clients.
    pub fn stream_sender(&self) -> broadcast::Sender<DepositEvent> {
        self.stream_tx.clone()
    }

    /// Publish to every persistent sink with backpressure, then to the lossy stream.
    ///
    /// A closed persistent sink is reported after every other sink has received the
    /// event, so a failed adapter cannot prevent healthy adapters from progressing.
    pub async fn publish_event(&self, event: DepositEvent) -> Result<(), EgressPublishError> {
        let mut closed_sinks = Vec::new();
        for sink in &self.persistent_sinks {
            if sink.sender.send(event.clone()).await.is_err() {
                closed_sinks.push(sink.name);
            }
        }
        // Broadcast receivers are intentionally best-effort stream consumers.
        let _ = self.stream_tx.send(event);

        if closed_sinks.is_empty() {
            Ok(())
        } else {
            Err(EgressPublishError { closed_sinks })
        }
    }
}

/// Start all enabled egress targets and return their tasks for shutdown waits.
/// Per-address delivery overrides are handled by async detector-owned delivery
/// workers, not by this egress layer.
pub fn start_with_tasks(config: EgressConfig) -> Result<(EgressHandle, Vec<JoinHandle<()>>)> {
    let (stream_tx, _) = broadcast::channel::<DepositEvent>(config.broadcast_capacity.max(1));
    let queue_capacity = config.persistent_queue_capacity.max(1);
    let mut persistent_sinks = Vec::new();
    let mut tasks = Vec::new();

    if config.file.enabled {
        let (tx, rx) = mpsc::channel(queue_capacity);
        persistent_sinks.push(PersistentSinkSender::new("file", tx));
        let path = config.file.path.clone();
        tasks.push(tokio::spawn(async move {
            if let Err(error) = file::write_events(path, rx).await {
                tracing::error!(%error, "file egress failed");
            }
        }));
    }

    #[cfg(feature = "sqlite")]
    if config.sqlite.enabled {
        let (tx, rx) = mpsc::channel(queue_capacity);
        persistent_sinks.push(PersistentSinkSender::new("sqlite", tx));
        let cfg = config.sqlite.clone();
        tasks.push(tokio::spawn(async move {
            if let Err(error) = sqlite::write_events(cfg, rx).await {
                tracing::error!(%error, "sqlite egress failed");
            }
        }));
    }

    #[cfg(feature = "postgres")]
    if config.pg.enabled {
        let (tx, rx) = mpsc::channel(queue_capacity);
        persistent_sinks.push(PersistentSinkSender::new("postgres", tx));
        let cfg = config.pg.clone();
        tasks.push(tokio::spawn(async move {
            if let Err(error) = pg::write_events(cfg, rx).await {
                tracing::error!(%error, "pg egress failed");
            }
        }));
    }

    #[cfg(feature = "amqp")]
    if config.queue.enabled {
        let (tx, rx) = mpsc::channel(queue_capacity);
        persistent_sinks.push(PersistentSinkSender::new("amqp", tx));
        let cfg = config.queue.clone();
        tasks.push(tokio::spawn(async move {
            if let Err(error) = queue::publish(cfg, rx).await {
                tracing::error!(%error, "queue egress failed");
            }
        }));
    }

    #[cfg(feature = "webhook")]
    if config.webhook.enabled {
        let (tx, rx) = mpsc::channel(queue_capacity);
        persistent_sinks.push(PersistentSinkSender::new("webhook", tx));
        let cfg = config.webhook.clone();
        tasks.push(tokio::spawn(async move {
            if let Err(error) = webhook::deliver(cfg, rx).await {
                tracing::error!(%error, "webhook egress failed");
            }
        }));
    }

    Ok((EgressHandle::new(stream_tx, persistent_sinks), tasks))
}
