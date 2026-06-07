use crate::model::DepositEvent;
use crate::shared::amqp::build_amqp_url;
use anyhow::Result;
use lapin::{BasicProperties, Connection, ConnectionProperties, options::*};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::broadcast;

// ── Egress queue configuration ──────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct QueueEgressConfig {
    pub enabled: bool,
    pub url: String,
    pub username: String,
    pub password: String,
    pub exchange: String,
    pub detected_routing_key: String,
    pub confirmed_routing_key: String,
    /// Seconds to wait before retrying on connection failure.
    pub reconnect_secs: u64,
}

impl Default for QueueEgressConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: String::new(),
            username: String::new(),
            password: String::new(),
            exchange: String::new(),
            detected_routing_key: "detected".to_string(),
            confirmed_routing_key: "confirmed".to_string(),
            reconnect_secs: 5,
        }
    }
}

// ── QueueConnection ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct QueueConnection {
    _conn: Arc<Connection>,
    channel: lapin::Channel,
    exchange: String,
    detected_routing_key: String,
    confirmed_routing_key: String,
}

impl QueueConnection {
    pub(crate) async fn connect(config: QueueEgressConfig) -> Result<Self> {
        let url = build_amqp_url(&config.url, &config.username, &config.password)?;
        let conn = Connection::connect(&url, ConnectionProperties::default()).await?;
        let channel = conn.create_channel().await?;
        channel
            .exchange_declare(
                config.exchange.as_str().into(),
                lapin::ExchangeKind::Topic,
                ExchangeDeclareOptions {
                    durable: true,
                    ..Default::default()
                },
                Default::default(),
            )
            .await?;
        Ok(Self {
            _conn: Arc::new(conn),
            channel,
            exchange: config.exchange,
            detected_routing_key: config.detected_routing_key,
            confirmed_routing_key: config.confirmed_routing_key,
        })
    }

    fn routing_key_for(&self, event: &DepositEvent) -> &str {
        match event.status() {
            crate::model::DepositStatus::Detected => &self.detected_routing_key,
            crate::model::DepositStatus::Confirmed => &self.confirmed_routing_key,
        }
    }

    pub(crate) async fn publish(&self, event: &DepositEvent) -> Result<()> {
        let payload = serde_json::to_string(event)?;
        let routing_key = self.routing_key_for(event);
        let confirm = self
            .channel
            .basic_publish(
                self.exchange.as_str().into(),
                routing_key.into(),
                BasicPublishOptions::default(),
                payload.as_bytes(),
                BasicProperties::default().with_content_type("application/json".into()),
            )
            .await?;
        confirm.await?;
        Ok(())
    }
}

// ── Publish loop ────────────────────────────────────────────────────────

/// Publish deposit events to a message queue.
/// Credentials are read from the queue config URL or username/password fields.
/// Routing key is dynamic: "detected" or "confirmed" based on event type.
pub async fn publish(
    config: QueueEgressConfig,
    rx: &mut broadcast::Receiver<DepositEvent>,
) -> Result<()> {
    let url = build_amqp_url(&config.url, &config.username, &config.password)?;
    let mut current_event = None;
    let mut pending_events = VecDeque::new();
    let reconnect_delay = std::time::Duration::from_secs(config.reconnect_secs);

    loop {
        if current_event.is_some() && drain_closed_receiver(rx, &mut pending_events) {
            tracing::warn!(
                "queue egress shutting down with an unpublished event after broadcast closed"
            );
            return Ok(());
        }
        tracing::info!(%url, exchange = %config.exchange, "queue egress connecting");
        let conn = match Connection::connect(&url, ConnectionProperties::default()).await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::error!(error = %e, "AMQP connection failed, retrying in {}s", config.reconnect_secs);
                tokio::time::sleep(reconnect_delay).await;
                continue;
            }
        };
        let channel = match conn.create_channel().await {
            Ok(channel) => channel,
            Err(e) => {
                tracing::error!(error = %e, "AMQP channel creation failed, retrying in {}s", config.reconnect_secs);
                tokio::time::sleep(reconnect_delay).await;
                continue;
            }
        };
        if let Err(e) = channel
            .exchange_declare(
                config.exchange.as_str().into(),
                lapin::ExchangeKind::Topic,
                ExchangeDeclareOptions {
                    durable: true,
                    ..Default::default()
                },
                Default::default(),
            )
            .await
        {
            tracing::error!(error = %e, "AMQP exchange declaration failed, retrying in {}s", config.reconnect_secs);
            tokio::time::sleep(reconnect_delay).await;
            continue;
        }

        loop {
            let event = match next_event_or_retry(rx, &mut current_event, &mut pending_events).await
            {
                Some(event) => event,
                None => return Ok(()),
            };

            let payload = match serde_json::to_string(&event) {
                Ok(payload) => payload,
                Err(e) => {
                    tracing::error!(error = %e, event_id = %event.event_id, "failed to serialize queue payload");
                    continue;
                }
            };
            let routing_key = match event.status() {
                crate::model::DepositStatus::Detected => config.detected_routing_key.as_str(),
                crate::model::DepositStatus::Confirmed => config.confirmed_routing_key.as_str(),
            };
            let publish = channel
                .basic_publish(
                    config.exchange.as_str().into(),
                    routing_key.into(),
                    BasicPublishOptions::default(),
                    payload.as_bytes(),
                    BasicProperties::default().with_content_type("application/json".into()),
                )
                .await;
            match publish {
                Ok(confirm) => match confirm.await {
                    Ok(_) => {
                        tracing::debug!(event_id = %event.event_id, %routing_key, "event published to queue")
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "AMQP publish was not confirmed, reconnecting");
                        current_event = Some(event);
                        break;
                    }
                },
                Err(e) => {
                    tracing::error!(error = %e, "AMQP publish failed, reconnecting");
                    current_event = Some(event);
                    break;
                }
            }
        }

        tokio::time::sleep(reconnect_delay).await;
    }
}

async fn next_event_or_retry(
    rx: &mut broadcast::Receiver<DepositEvent>,
    current_event: &mut Option<DepositEvent>,
    pending_events: &mut VecDeque<DepositEvent>,
) -> Option<DepositEvent> {
    if let Some(event) = current_event.take() {
        return Some(event);
    }
    if let Some(event) = pending_events.pop_front() {
        return Some(event);
    }

    super::recv_event(rx).await
}

fn drain_closed_receiver(
    rx: &mut broadcast::Receiver<DepositEvent>,
    pending_events: &mut VecDeque<DepositEvent>,
) -> bool {
    loop {
        match rx.try_recv() {
            Ok(event) => pending_events.push_back(event),
            Err(broadcast::error::TryRecvError::Lagged(missed)) => {
                tracing::warn!(missed, "broadcast receiver lagging, skipping missed events");
            }
            Err(broadcast::error::TryRecvError::Empty) => return false,
            Err(broadcast::error::TryRecvError::Closed) => return true,
        }
    }
}
