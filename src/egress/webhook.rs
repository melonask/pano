use serde::{Deserialize, Serialize};

// ── Egress webhook configuration ────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WebhookEgressConfig {
    pub enabled: bool,
    pub url: String,
    pub secret: String,
    /// HTTP header carrying the HMAC-SHA256 signature.
    pub signature_header: String,
    /// Number of retry rounds after the initial delivery attempt.
    pub max_retries: u32,
    /// Base retry delay in milliseconds (doubled each attempt).
    pub retry_base_ms: u64,
    /// Per-request HTTP timeout in seconds.
    pub timeout_secs: u64,
}

impl Default for WebhookEgressConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: String::new(),
            secret: String::new(),
            signature_header: "X-Pano-Signature".to_string(),
            max_retries: 3,
            retry_base_ms: 250,
            timeout_secs: 30,
        }
    }
}

// ── Implementation (requires webhook feature) ───────────────────────────

#[cfg(feature = "webhook")]
mod imp {
    use super::WebhookEgressConfig;
    use crate::model::DepositEvent;
    use anyhow::Result;
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    use std::time::Duration;
    use tokio::sync::broadcast;

    type HmacSha256 = Hmac<Sha256>;

    /// Deliver deposit events via HTTP webhook (POST with JSON body and HMAC-SHA256 signature).
    pub async fn deliver(
        config: WebhookEgressConfig,
        rx: &mut broadcast::Receiver<DepositEvent>,
    ) -> Result<()> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs.max(1)))
            .build()?;
        let secret = config.secret.clone();

        loop {
            let Some(event) = super::super::recv_event(rx).await else {
                break;
            };

            if config.url.is_empty() {
                tracing::error!(event_id = %event.event_id, "webhook egress enabled but URL is empty, skipping");
                continue;
            }

            if let Err(e) =
                deliver_single_with_client(&http, &config.url, &secret, &event, &config).await
            {
                tracing::error!(error = %e, event_id = %event.event_id, "failed to prepare webhook delivery");
            }
        }

        Ok(())
    }

    pub async fn deliver_single(url: &str, secret: &str, event: &DepositEvent) -> Result<()> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(
                WebhookEgressConfig::default().timeout_secs,
            ))
            .build()?;
        let cfg = WebhookEgressConfig {
            enabled: true,
            url: url.to_string(),
            secret: secret.to_string(),
            ..WebhookEgressConfig::default()
        };
        deliver_single_with_client(&http, url, secret, event, &cfg).await
    }

    pub async fn deliver_single_with_client(
        http: &reqwest::Client,
        url: &str,
        secret: &str,
        event: &DepositEvent,
        config: &WebhookEgressConfig,
    ) -> Result<()> {
        let payload = serde_json::to_string(event)?;
        let mut request = http
            .post(url)
            .header("Content-Type", "application/json")
            .header("X-Pano-Event", &event.event);
        if !secret.is_empty() {
            let signature = compute_hmac(secret, &payload)?;
            request = request.header(&config.signature_header, signature);
        }
        deliver_with_retry(request, payload, event, config).await
    }

    async fn deliver_with_retry(
        request: reqwest::RequestBuilder,
        payload: String,
        event: &DepositEvent,
        config: &WebhookEgressConfig,
    ) -> Result<()> {
        for attempt in 0..=config.max_retries {
            let attempt_number = attempt + 1;
            let Some(request) = request.try_clone() else {
                tracing::error!(event_id = %event.event_id, "webhook request could not be cloned for retry");
                anyhow::bail!("webhook request could not be cloned for retry");
            };
            match request.body(payload.clone()).send().await {
                Ok(resp) if resp.status().is_success() => {
                    tracing::debug!(event_id = %event.event_id, status = %resp.status(), attempt = attempt_number, "webhook delivered");
                    return Ok(());
                }
                Ok(resp) if resp.status().is_server_error() || resp.status().as_u16() == 429 => {
                    tracing::warn!(event_id = %event.event_id, status = %resp.status(), attempt = attempt_number, "webhook transient failure");
                }
                Ok(resp) => {
                    tracing::warn!(event_id = %event.event_id, status = %resp.status(), "webhook permanent failure");
                    anyhow::bail!("webhook permanent failure: HTTP {}", resp.status());
                }
                Err(e) => {
                    tracing::warn!(event_id = %event.event_id, error = %e, attempt = attempt_number, "webhook delivery failed");
                }
            }
            if attempt < config.max_retries {
                let delay_ms = config.retry_base_ms.saturating_mul(2_u64.pow(attempt));
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
        }
        anyhow::bail!(
            "webhook delivery failed after {} attempts",
            config.max_retries.saturating_add(1)
        );
    }

    /// Compute HMAC-SHA256 signature for webhook payload verification.
    pub fn compute_hmac(key: &str, data: &str) -> Result<String> {
        let mut mac = HmacSha256::new_from_slice(key.as_bytes())?;
        mac.update(data.as_bytes());
        let result = mac.finalize();
        let code_bytes = result.into_bytes();
        Ok(data_encoding::HEXLOWER.encode(&code_bytes))
    }
}

#[cfg(feature = "webhook")]
pub use imp::*;

#[cfg(not(feature = "webhook"))]
pub async fn deliver(
    _config: WebhookEgressConfig,
    _rx: &mut tokio::sync::broadcast::Receiver<crate::model::DepositEvent>,
) -> anyhow::Result<()> {
    anyhow::bail!("webhook egress requires feature \"webhook\" (rebuild with --features webhook)");
}
