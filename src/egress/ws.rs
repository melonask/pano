use serde::{Deserialize, Serialize};

// ── HTTP egress configuration ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpEgressConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_sse_path")]
    pub sse: String,
    #[serde(default = "default_websocket_path")]
    pub websocket: String,
    /// WebSocket heartbeat interval in seconds.
    #[serde(default = "default_ws_heartbeat_secs")]
    pub ws_heartbeat_secs: u64,
    /// SSE keepalive interval in seconds.
    #[serde(default = "default_sse_keepalive_secs")]
    pub sse_keepalive_secs: u64,
    /// Maximum accepted WebSocket message size in bytes.
    #[serde(default = "default_ws_max_message_size")]
    pub ws_max_message_size: usize,
    /// Maximum accepted WebSocket frame size in bytes.
    #[serde(default = "default_ws_max_frame_size")]
    pub ws_max_frame_size: usize,
}

fn default_sse_path() -> String {
    "events".to_string()
}
fn default_websocket_path() -> String {
    "ws".to_string()
}
fn default_ws_heartbeat_secs() -> u64 {
    15
}
fn default_sse_keepalive_secs() -> u64 {
    15
}
fn default_ws_max_message_size() -> usize {
    64 * 1024
}
fn default_ws_max_frame_size() -> usize {
    64 * 1024
}

impl Default for HttpEgressConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            sse: default_sse_path(),
            websocket: default_websocket_path(),
            ws_heartbeat_secs: default_ws_heartbeat_secs(),
            sse_keepalive_secs: default_sse_keepalive_secs(),
            ws_max_message_size: default_ws_max_message_size(),
            ws_max_frame_size: default_ws_max_frame_size(),
        }
    }
}

// ── WebSocket handler (requires server feature) ─────────────────────────

#[cfg(feature = "server")]
mod imp {
    use crate::detector::DetectorHandle;
    use axum::extract::State;
    use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
    use axum::response::IntoResponse;
    use futures::{SinkExt, StreamExt};
    use std::time::Duration;
    use tokio_stream::wrappers::BroadcastStream;
    use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

    /// Interval between server-initiated WebSocket heartbeat pings.
    const WS_HEARTBEAT_PAYLOAD: &[u8] = b"pano-heartbeat";

    fn ws_lag_message(missed: u64) -> Message {
        Message::Text(
            serde_json::json!({
                "event": "pano.stream.lag",
                "data": { "missed": missed },
            })
            .to_string()
            .into(),
        )
    }

    /// GET /v1/ws — WebSocket handler for real-time deposit notifications.
    pub async fn ws_handler(
        State(handle): State<DetectorHandle>,
        ws: WebSocketUpgrade,
    ) -> impl IntoResponse {
        ws.max_message_size(handle.config.egress.http.ws_max_message_size.max(1))
            .max_frame_size(handle.config.egress.http.ws_max_frame_size.max(1))
            .on_upgrade(move |socket| handle_ws(socket, handle))
    }

    async fn handle_ws(socket: WebSocket, handle: DetectorHandle) {
        let heartbeat_interval =
            Duration::from_secs(handle.config.egress.http.ws_heartbeat_secs.max(1));

        let mut rx = BroadcastStream::new(handle.events_tx.subscribe());
        let (mut sender, mut receiver) = socket.split();
        let mut heartbeat = tokio::time::interval(heartbeat_interval);
        let mut awaiting_heartbeat_pong = false;

        // tokio intervals tick immediately by default; skip that first tick so a
        // newly-upgraded connection is not pinged before it has settled.
        heartbeat.tick().await;

        loop {
            tokio::select! {
                result = rx.next() => match result {
                    Some(Ok(event)) => {
                        let data = match serde_json::to_string(&event) {
                            Ok(d) => d,
                            Err(e) => {
                                tracing::error!(error = %e, "failed to serialize event for WebSocket");
                                continue;
                            }
                        };
                        if sender.send(Message::Text(data.into())).await.is_err() {
                            break;
                        }
                    }
                    Some(Err(BroadcastStreamRecvError::Lagged(missed))) => {
                        tracing::warn!(missed, "websocket client lagged behind broadcast stream");
                        if sender.send(ws_lag_message(missed)).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                },
                _ = heartbeat.tick() => {
                    if awaiting_heartbeat_pong {
                        tracing::debug!("websocket heartbeat pong timed out");
                        break;
                    }

                    if sender
                        .send(Message::Ping(WS_HEARTBEAT_PAYLOAD.to_vec().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                    awaiting_heartbeat_pong = true;
                },
                inbound = receiver.next() => match inbound {
                    Some(Ok(Message::Ping(payload))) => {
                        if sender.send(Message::Pong(payload)).await.is_err() { break; }
                    }
                    Some(Ok(Message::Pong(payload))) => {
                        if payload.as_ref() == WS_HEARTBEAT_PAYLOAD {
                            awaiting_heartbeat_pong = false;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        tracing::debug!(error = %e, "websocket receive failed");
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(feature = "server")]
pub use imp::*;
