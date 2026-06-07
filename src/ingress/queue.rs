use crate::model::{Command, UnwatchAddressRequest, WatchSpec};
use crate::shared::amqp::build_amqp_url;
use anyhow::Result;
use futures::StreamExt;
use lapin::{Connection, ConnectionProperties, options::*, types::FieldTable};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

// ── Ingress queue configuration ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct QueueIngressConfig {
    pub enabled: bool,
    pub url: String,
    pub username: String,
    pub password: String,
    pub exchange: String,
    pub watch_routing_key: String,
    pub unwatch_routing_key: String,
    /// Seconds to wait before retrying on connection failure.
    pub reconnect_secs: u64,
    /// AMQP QoS prefetch count for the consumer channel.
    pub qos_prefetch: u16,
    /// Consumer tag for this Pano instance.
    pub consumer_tag: String,
}

impl Default for QueueIngressConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: String::new(),
            username: String::new(),
            password: String::new(),
            exchange: String::new(),
            watch_routing_key: String::new(),
            unwatch_routing_key: String::new(),
            reconnect_secs: 5,
            qos_prefetch: 100,
            consumer_tag: "pano-ingress".to_string(),
        }
    }
}

// ── Implementation ──────────────────────────────────────────────────────

/// Consume watched addresses from a message queue.
/// Credentials are read from the queue config URL or username/password fields.
pub async fn consume(config: QueueIngressConfig, tx: mpsc::Sender<Command>) -> Result<()> {
    let url = build_amqp_url(&config.url, &config.username, &config.password)?;
    let reconnect_delay = std::time::Duration::from_secs(config.reconnect_secs);

    loop {
        if tx.is_closed() {
            return Ok(());
        }

        tracing::info!(%url, exchange = %config.exchange, "queue ingress connecting");
        let conn = match Connection::connect(&url, ConnectionProperties::default()).await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::error!(error = %e, "AMQP connection failed, retrying in {}s", config.reconnect_secs);
                sleep_or_closed(&tx, reconnect_delay).await?;
                continue;
            }
        };
        let channel = match conn.create_channel().await {
            Ok(channel) => channel,
            Err(e) => {
                tracing::error!(error = %e, "AMQP channel creation failed, retrying in {}s", config.reconnect_secs);
                sleep_or_closed(&tx, reconnect_delay).await?;
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
                FieldTable::default(),
            )
            .await
        {
            tracing::error!(error = %e, "AMQP exchange declaration failed, retrying in {}s", config.reconnect_secs);
            sleep_or_closed(&tx, reconnect_delay).await?;
            continue;
        }
        let queue = match channel
            .queue_declare(
                "".into(),
                QueueDeclareOptions {
                    exclusive: true,
                    durable: false,
                    auto_delete: true,
                    ..Default::default()
                },
                FieldTable::default(),
            )
            .await
        {
            Ok(queue) => queue,
            Err(e) => {
                tracing::error!(error = %e, "AMQP queue declaration failed, retrying in {}s", config.reconnect_secs);
                sleep_or_closed(&tx, reconnect_delay).await?;
                continue;
            }
        };
        if let Err(e) = channel
            .queue_bind(
                queue.name().clone(),
                config.exchange.as_str().into(),
                config.watch_routing_key.as_str().into(),
                QueueBindOptions::default(),
                FieldTable::default(),
            )
            .await
        {
            tracing::error!(error = %e, "AMQP queue bind failed, retrying in {}s", config.reconnect_secs);
            sleep_or_closed(&tx, reconnect_delay).await?;
            continue;
        }
        if let Err(e) = channel
            .queue_bind(
                queue.name().clone(),
                config.exchange.as_str().into(),
                config.unwatch_routing_key.as_str().into(),
                QueueBindOptions::default(),
                FieldTable::default(),
            )
            .await
        {
            tracing::error!(error = %e, "AMQP unwatch queue bind failed, retrying in {}s", config.reconnect_secs);
            sleep_or_closed(&tx, reconnect_delay).await?;
            continue;
        }
        if let Err(e) = channel
            .basic_qos(config.qos_prefetch, BasicQosOptions::default())
            .await
        {
            tracing::error!(error = %e, "AMQP basic_qos failed, retrying in {}s", config.reconnect_secs);
            sleep_or_closed(&tx, reconnect_delay).await?;
            continue;
        }
        let mut consumer = match channel
            .basic_consume(
                queue.name().clone(),
                config.consumer_tag.as_str().into(),
                BasicConsumeOptions::default(),
                FieldTable::default(),
            )
            .await
        {
            Ok(consumer) => consumer,
            Err(e) => {
                tracing::error!(error = %e, "AMQP consumer creation failed, retrying in {}s", config.reconnect_secs);
                sleep_or_closed(&tx, reconnect_delay).await?;
                continue;
            }
        };

        loop {
            let next_delivery = tokio::select! {
                _ = tx.closed() => return Ok(()),
                delivery = consumer.next() => delivery,
            };
            match next_delivery {
                Some(Ok(delivery)) => {
                    let command = if delivery.routing_key.as_str() == config.watch_routing_key {
                        serde_json::from_slice::<WatchSpec>(&delivery.data)
                            .map(|spec| Command::Watch(Box::new(spec)))
                    } else if delivery.routing_key.as_str() == config.unwatch_routing_key {
                        serde_json::from_slice::<UnwatchAddressRequest>(&delivery.data).map(|req| {
                            Command::Unwatch {
                                address: req.address,
                            }
                        })
                    } else {
                        tracing::warn!(routing_key = %delivery.routing_key, "unexpected address ingress routing key");
                        if let Err(e) = delivery
                            .nack(BasicNackOptions {
                                requeue: false,
                                ..Default::default()
                            })
                            .await
                        {
                            tracing::error!(error = %e, "AMQP nack failed, reconnecting");
                            break;
                        }
                        continue;
                    };
                    match command {
                        Ok(cmd) => {
                            if tx.send(cmd).await.is_err() {
                                return Ok(());
                            }
                            if let Err(e) = delivery.ack(BasicAckOptions::default()).await {
                                tracing::error!(error = %e, "AMQP ack failed, reconnecting");
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, routing_key = %delivery.routing_key, "invalid address ingress message");
                            if let Err(e) = delivery
                                .nack(BasicNackOptions {
                                    requeue: false,
                                    ..Default::default()
                                })
                                .await
                            {
                                tracing::error!(error = %e, "AMQP nack failed, reconnecting");
                                break;
                            }
                        }
                    }
                }
                Some(Err(e)) => {
                    tracing::error!(error = %e, "AMQP delivery failed, reconnecting");
                    break;
                }
                None => {
                    tracing::warn!("AMQP consumer ended, reconnecting");
                    break;
                }
            }
        }

        sleep_or_closed(&tx, reconnect_delay).await?;
    }
}

async fn sleep_or_closed(tx: &mpsc::Sender<Command>, delay: std::time::Duration) -> Result<()> {
    tokio::select! {
        _ = tx.closed() => Ok(()),
        _ = tokio::time::sleep(delay) => Ok(()),
    }
}
