use crate::detector::DetectorHandle;
use crate::server::error::ApiError;
use axum::extract::{DefaultBodyLimit, Request};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use axum::{
    Router,
    routing::{delete, get, post},
};
use tower_http::trace::TraceLayer;

/// Build the Axum router with all HTTP routes.
pub fn router(handle: DetectorHandle) -> Router {
    let cfg = handle.config.clone();
    let health_handle = handle.clone();
    let mut app = Router::new();

    if cfg.ingress.http.enabled && !cfg.ingress.http.addresses.trim().is_empty() {
        let addresses = route_path(&cfg.ingress.http.addresses);
        app = app
            .route(&addresses, post(crate::ingress::api::add_address))
            .route(
                &format!("{addresses}/{{address}}"),
                delete(crate::ingress::api::remove_address),
            )
            .layer(DefaultBodyLimit::max(
                cfg.ingress.http.max_body_bytes.min(usize::MAX as u64) as usize,
            ));
    }
    if cfg.egress.http.enabled && !cfg.egress.http.sse.trim().is_empty() {
        app = app.route(
            &route_path(&cfg.egress.http.sse),
            get(crate::egress::sse::sse_handler),
        );
    }
    if cfg.egress.http.enabled && !cfg.egress.http.websocket.trim().is_empty() {
        app = app.route(
            &route_path(&cfg.egress.http.websocket),
            get(crate::egress::ws::ws_handler),
        );
    }

    let app = app
        .with_state(handle)
        .fallback(crate::ingress::api::not_found);

    let prefix = cfg.server.prefix.trim().trim_matches('/');
    let mut root = if prefix.is_empty() {
        app
    } else {
        Router::new().nest(&format!("/{prefix}"), app)
    };
    root = root.route("/healthz", get(move || health(health_handle.clone())));

    if !cfg.server.dashboard.trim().is_empty() {
        let dashboard_route = if prefix.is_empty() {
            format!("/{}", cfg.server.dashboard.trim().trim_matches('/'))
        } else {
            format!(
                "/{prefix}/{}",
                cfg.server.dashboard.trim().trim_matches('/')
            )
        };
        root = root.nest_service(
            &dashboard_route,
            tower_http::services::ServeDir::new(&cfg.server.dashboard),
        );
    }

    let api_key = cfg.server.api_key.clone();
    let root = root.layer(axum::middleware::from_fn(
        move |headers: HeaderMap, request: Request, next: Next| {
            require_api_key(headers, request, next, api_key.clone())
        },
    ));

    root.layer(TraceLayer::new_for_http())
}

/// Internal liveness endpoint for orchestrators. A successful response proves
/// that the HTTP server is accepting work and the detector command loop is live.
pub async fn health(handle: DetectorHandle) -> StatusCode {
    if handle.cmd_tx.is_closed() {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::NO_CONTENT
    }
}

async fn require_api_key(
    headers: HeaderMap,
    request: Request,
    next: Next,
    api_key: String,
) -> Result<Response, ApiError> {
    if api_key.is_empty() || api_key_matches(&headers, &api_key) {
        Ok(next.run(request).await)
    } else {
        tracing::warn!(
            path = %request.uri().path(),
            "authentication failed: invalid or missing API key"
        );
        Err(ApiError::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "invalid or missing API key",
        ))
    }
}

fn api_key_matches(headers: &HeaderMap, expected: &str) -> bool {
    if expected.is_empty() {
        return false;
    }
    let header_key = headers
        .get("x-pano-api-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let bearer = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    aws_lc_rs::constant_time::verify_slices_are_equal(header_key.as_bytes(), expected.as_bytes())
        .is_ok()
        || aws_lc_rs::constant_time::verify_slices_are_equal(bearer.as_bytes(), expected.as_bytes())
            .is_ok()
}

pub fn route_path(path: &str) -> String {
    format!("/{}", path.trim().trim_matches('/'))
}
