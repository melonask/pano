// ── SSE handler (requires server feature) ───────────────────────────────

#[cfg(feature = "server")]
mod imp {
    use crate::detector::DetectorHandle;
    use axum::extract::State;
    use axum::response::sse::{Event, KeepAlive, Sse};
    use futures::stream::Stream;
    use std::convert::Infallible;
    use std::time::Duration;
    use tokio_stream::StreamExt;
    use tokio_stream::wrappers::BroadcastStream;

    /// GET /v1/sse — Server-Sent Events stream for real-time deposit notifications.
    pub async fn sse_handler(
        State(handle): State<DetectorHandle>,
    ) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
        let keepalive_secs = handle.config.egress.http.sse_keepalive_secs.max(1);
        let receiver = handle.events_tx.subscribe();
        let stream = BroadcastStream::new(receiver).filter_map(|result| match result {
            Ok(event) => {
                let data = serde_json::to_string(&event).ok()?;
                Some(Ok(Event::default().event(&event.event).data(data)))
            }
            Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(missed)) => {
                Some(Ok(Event::default()
                    .event("pano.stream.lag")
                    .data(serde_json::json!({"missed": missed}).to_string())))
            }
        });
        Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(keepalive_secs)))
    }
}

#[cfg(feature = "server")]
pub use imp::*;
