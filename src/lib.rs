//! # Pano — multi-chain, real-time deposit detector.
//!
//! ## Design Philosophy
//!
//! Pano is built on four principles:
//!
//! 1. **Configuration over hardcoding** — all behaviour is configurable; avoid
//!    hardcoded values, assumptions, and environment-specific logic.
//! 2. **Stateless by design** — the package does not own, persist, or manage
//!    state. State management is the caller's responsibility.
//! 3. **Simplicity** — the processing flow is linear: `ingress → detector →
//!    egress`. Avoid abstractions, layers, or features that do not provide clear
//!    value.
//! 4. **Consistency** — data structures, parameters, APIs, behaviours, and
//!    naming conventions follow the same patterns throughout the package.
//!
//! ## Architecture
//!
//! ```text
//! Ingress (HTTP / File / DB / Queue)
//!   │  Watch / Unwatch / SyncAll commands (mpsc)
//!   ▼
//! Detector (chain scanners, dedup, confirmations)
//!   │  DepositEvent (broadcast)
//!   ▼
//! Egress (File / DB / Queue / Webhook / SSE / WS)
//! ```
//!
//! Ingress and egress are fully decoupled. The detector is the single point of
//! truth for the watched address set. Chain scanners are stateless functions;
//! all operational state (scan cursors, dedup window, unconfirmed events) lives
//! exclusively in the detector task and is discarded on shutdown.

pub mod chain;
pub mod config;
pub mod delivery;
pub mod detector;
pub mod egress;
pub mod ingress;
pub mod model;
pub mod rpc;
#[cfg(feature = "server")]
pub mod server;
pub mod shared;

use crate::config::AppConfig;
use crate::detector::DetectorHandle;
use crate::model::ResolvedWatch;
use anyhow::Result;
use std::time::Duration;
use tokio::task::JoinHandle;

pub async fn run(config: AppConfig) -> Result<()> {
    let (ingress_handle, ingress_tasks) =
        ingress::start_with_tasks(config.ingress.clone(), Some(config.clone()))?;
    let (egress_handle, egress_tasks) = egress::start_with_tasks(config.egress.clone())?;
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel::<()>(1);
    let (detector_handle, detector_task) =
        detector::start_with_tasks(config.clone(), ingress_handle, egress_handle.clone())?;
    let cmd_tx = detector_handle.cmd_tx.clone();
    tokio::spawn(async move {
        if let Err(error) = wait_for_shutdown_signal().await {
            tracing::warn!(%error, "failed while waiting for shutdown signal");
        }
        tracing::info!("initiating graceful shutdown");
        if let Err(error) = cmd_tx.try_send(crate::model::Command::Shutdown) {
            tracing::warn!(%error, "detector shutdown command could not be queued");
        }
        if shutdown_tx.send(()).await.is_err() {
            tracing::debug!("runtime shutdown receiver already closed");
        }
    });

    if config.server.dashboard_export && !config.server.dashboard.trim().is_empty() {
        export_dashboard_files(&config, &detector_handle).await;
        let dashboard_path = config.server.dashboard.clone();
        let handle = detector_handle.clone();
        let mut address_change_rx = detector_handle.address_change_tx.subscribe();
        tokio::spawn(async move {
            let mut debounce = tokio::time::interval(Duration::from_millis(500));
            debounce.tick().await;
            loop {
                tokio::select! {
                    res = address_change_rx.changed() => {
                        if res.is_err() {
                            tracing::debug!(
                                "address change channel closed, shutting down debounce task"
                            );
                            break;
                        }
                        debounce.reset();
                    }
                    _ = debounce.tick() => {
                        let _ = address_change_rx.borrow_and_update();
                        rewrite_addresses_json(&dashboard_path, &handle).await;
                    }
                }
            }
        });
    }

    #[cfg(feature = "server")]
    if config.server.enabled {
        let addr = format!("{}:{}", config.server.bind, config.server.port);
        let app = server::router::router(detector_handle.clone());
        let listener = tokio::net::TcpListener::bind(&addr).await?;
        tracing::info!(%addr, "HTTP server listening");
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.recv().await;
            })
            .await?;
    } else {
        tracing::info!("HTTP server disabled; running in headless mode");
        let _ = shutdown_rx.recv().await;
    }

    #[cfg(not(feature = "server"))]
    {
        if config.server.enabled {
            tracing::warn!(
                "server.enabled is true but the 'server' feature is not compiled in; running headless"
            );
        } else {
            tracing::info!("HTTP server disabled; running in headless mode");
        }
        let _ = shutdown_rx.recv().await;
    }

    drop(detector_handle);
    drop(egress_handle);
    let shutdown_timeout = Duration::from_secs(config.server.shutdown_timeout_secs.max(1));
    await_task("detector", detector_task, shutdown_timeout).await;
    await_tasks("ingress", ingress_tasks, shutdown_timeout).await;
    await_tasks("egress", egress_tasks, shutdown_timeout).await;
    Ok(())
}

async fn export_dashboard_files(config: &AppConfig, handle: &DetectorHandle) {
    let dir = &config.server.dashboard;
    if let Err(e) = tokio::fs::create_dir_all(dir).await {
        tracing::warn!(path = %dir, error = %e, "failed to create dashboard directory");
        return;
    }

    let config_path = std::path::Path::new(dir).join("config.json");
    match serde_json::to_string_pretty(&shared::util::mask_config(config)) {
        Ok(json) => {
            if let Err(e) = tokio::fs::write(&config_path, json).await {
                tracing::warn!(path = %config_path.display(), error = %e, "failed to write config.json");
            }
        }
        Err(e) => tracing::warn!(error = %e, "failed to serialize config"),
    }

    rewrite_addresses_json(dir, handle).await;
}

async fn rewrite_addresses_json(dir: &str, handle: &DetectorHandle) {
    let path = std::path::Path::new(dir).join("addresses.json");
    let watched = handle.watched.read().await;
    let addresses: Vec<ResolvedWatch> = watched.values().cloned().collect();
    drop(watched);
    match serde_json::to_string_pretty(&addresses) {
        Ok(json) => {
            if let Err(e) = tokio::fs::write(&path, json).await {
                tracing::warn!(path = %path.display(), error = %e, "failed to rewrite addresses.json");
            }
        }
        Err(e) => tracing::warn!(error = %e, "failed to serialize addresses"),
    }
}

async fn await_tasks(name: &str, tasks: Vec<JoinHandle<()>>, timeout: Duration) {
    for task in tasks {
        await_task(name, task, timeout).await;
    }
}

async fn await_task(name: &str, task: JoinHandle<()>, timeout: Duration) {
    let mut task = task;
    tokio::select! {
        result = &mut task => {
            if let Err(e) = result {
                tracing::warn!(task = name, error = %e, "background task failed during shutdown");
            }
        }
        _ = tokio::time::sleep(timeout) => {
            task.abort();
            tracing::warn!(task = name, "timed out waiting for background task shutdown");
        }
    }
}

#[cfg(unix)]
async fn wait_for_shutdown_signal() -> Result<()> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        _ = interrupt.recv() => {}
        _ = terminate.recv() => {}
    }
    Ok(())
}

#[cfg(not(unix))]
async fn wait_for_shutdown_signal() -> Result<()> {
    tokio::signal::ctrl_c().await?;
    Ok(())
}
