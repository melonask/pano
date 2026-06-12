#[cfg(feature = "server")]
use crate::detector::DetectorHandle;
#[cfg(feature = "server")]
use crate::model::{Command, WatchSpec, normalize_address_key};
#[cfg(feature = "server")]
use crate::server::error::{ApiError, ApiResult};
#[cfg(feature = "server")]
use axum::Json;
#[cfg(feature = "server")]
use axum::extract::{Path, State};
#[cfg(feature = "server")]
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpIngressConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_addresses_path")]
    pub addresses: String,
}

impl Default for HttpIngressConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            addresses: default_addresses_path(),
        }
    }
}

fn default_addresses_path() -> String {
    "addresses".to_string()
}

// ── HTTP handler functions (only compiled with server feature) ────────────

#[cfg(feature = "server")]
/// Fallback 404 for unknown routes.
pub async fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "not_found", "unknown route")
}

#[cfg(feature = "server")]
/// POST /v1/addresses — Add a new address watch.
///
/// Resolves and validates before enqueueing the mutation for the detector loop.
pub async fn add_address(
    State(handle): State<DetectorHandle>,
    Json(spec): Json<WatchSpec>,
) -> ApiResult<StatusCode> {
    reject_if_authoritative_file_ingress(&handle)?;

    // Resolve first so HTTP callers get synchronous validation errors.
    let resolved = crate::detector::resolve_watch_spec_to_watches(&handle.config, &spec)
        .map_err(|e| ApiError::new(StatusCode::BAD_REQUEST, "bad_request", e.to_string()))?;

    let watched = handle.watched.read().await;
    for rw in &resolved {
        let key = (rw.address.clone(), rw.caip2.clone(), rw.symbol.clone());
        if watched.contains_key(&key) {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "conflict",
                format!(
                    "triad ({}, {}, {}) already watched",
                    rw.address, rw.caip2, rw.symbol
                ),
            ));
        }
    }
    let count = resolved.len();
    drop(watched);

    handle
        .cmd_tx
        .send(Command::Watch(Box::new(spec)))
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "detector command channel closed",
            )
        })?;

    tracing::debug!(count, "queued watch mutation via API");
    Ok(StatusCode::CREATED)
}

#[cfg(feature = "server")]
/// DELETE /v1/addresses/{address} — Remove a watched address (all triads).
pub async fn remove_address(
    State(handle): State<DetectorHandle>,
    Path(raw_address): Path<String>,
) -> ApiResult<StatusCode> {
    reject_if_authoritative_file_ingress(&handle)?;
    let address = normalize_address_key(&raw_address);

    let watched = handle.watched.read().await;
    let existed = watched.keys().any(|(addr, _, _)| addr == &address);
    if !existed {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "address not watched",
        ));
    }
    let remaining_count = watched.len();
    drop(watched);

    handle
        .cmd_tx
        .send(Command::Unwatch {
            address: address.clone(),
        })
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "detector command channel closed",
            )
        })?;

    tracing::info!(%address, remaining = remaining_count, "address unwatched via HTTP API");
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(feature = "server")]
fn reject_if_authoritative_file_ingress(handle: &DetectorHandle) -> ApiResult<()> {
    if handle.config.ingress.file.enabled && handle.config.ingress.file.authoritative {
        return Err(ApiError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "HTTP address mutations are disabled while authoritative file ingress is enabled",
        ));
    }
    Ok(())
}
