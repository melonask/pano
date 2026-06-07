use crate::config::ChainConfig;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// RPC client with failover and rate-limiting for a single chain.
#[derive(Debug, Clone)]
pub struct RpcClient {
    pub chain: Arc<ChainConfig>,
    http: reqwest::Client,
    semaphore: Arc<Semaphore>,
    delay: Option<std::time::Duration>,
    max_retries: u32,
    retry_base: std::time::Duration,
    current_idx: Arc<std::sync::atomic::AtomicUsize>,
    /// Monotonically increasing JSON-RPC request ID counter.
    id_counter: Arc<std::sync::atomic::AtomicU64>,
}

impl RpcClient {
    pub fn new(chain: ChainConfig) -> Self {
        let rpc_options = chain.rpc_options_or_default();
        let max_concurrent = rpc_options.max_concurrent;
        let delay = chain
            .rpc_options
            .as_ref()
            .map(|_| std::time::Duration::from_millis(rpc_options.delay_ms));
        let request_timeout_secs = rpc_options.request_timeout_secs.max(1);
        let max_retries = rpc_options.max_retries;
        let retry_base_ms = rpc_options.retry_base_ms;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(request_timeout_secs))
            .build()
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "failed to build configured RPC HTTP client; using default client");
                reqwest::Client::new()
            });
        Self {
            chain: Arc::new(chain),
            http,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            delay,
            max_retries,
            retry_base: std::time::Duration::from_millis(retry_base_ms),
            current_idx: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            id_counter: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        }
    }

    /// Send a JSON-RPC request, trying the next endpoint on failure.
    pub async fn send(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let result = self.send_inner_with_retry(method, params).await;
        if let Some(d) = self.delay {
            tokio::time::sleep(d).await;
        }
        result
    }

    async fn send_inner_with_retry(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let endpoints = &self.chain.rpc;
        let mut last_error = None;
        for round in 0..=self.max_retries {
            let start = self.current_idx.load(std::sync::atomic::Ordering::Relaxed);
            for i in 0..endpoints.len() {
                let idx = (start + i) % endpoints.len();
                let permit = self.semaphore.acquire().await?;
                let call_result = self.json_rpc_call(&endpoints[idx], method, &params).await;
                drop(permit);
                match call_result {
                    Ok(val) => {
                        self.current_idx
                            .store(idx, std::sync::atomic::Ordering::Relaxed);
                        return Ok(val);
                    }
                    Err(e) => {
                        tracing::warn!(endpoint = %endpoints[idx], method, params = %params, error = %e, "RPC call failed, trying next");
                        last_error = Some(e);
                    }
                }
            }
            if round < self.max_retries {
                tokio::time::sleep(self.retry_backoff(round)).await;
            }
        }
        anyhow::bail!(
            "all RPC endpoints failed for chain {}: {:?}",
            self.chain.caip2,
            last_error
        )
    }

    pub fn retry_backoff(&self, round: u32) -> std::time::Duration {
        self.retry_base.saturating_mul(2_u32.saturating_pow(round))
    }

    async fn json_rpc_call(
        &self,
        url: &str,
        method: &str,
        params: &serde_json::Value,
    ) -> Result<serde_json::Value> {
        let id = self
            .id_counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let body =
            serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let resp = self.http.post(url).json(&body).send().await?;
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            anyhow::bail!("RPC rate limited (429)");
        }
        let resp = resp.error_for_status()?;
        let json: serde_json::Value = resp.json().await?;
        if let Some(error) = json.get("error") {
            anyhow::bail!("JSON-RPC error: {}", error);
        }
        Ok(json
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }
}
