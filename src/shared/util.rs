use crate::config::AppConfig;

/// Mask sensitive fields in a JSON value: secrets, keys, tokens, passwords, and
/// URLs containing embedded credentials. Used by dashboard export and config display.
pub fn mask_secret_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map.iter_mut() {
                if is_sensitive_key(key) {
                    if value.as_str().is_some_and(|s| !s.is_empty()) {
                        *value = serde_json::Value::String("***".to_string());
                    }
                } else if key == "url" {
                    if let Some(s) = value.as_str()
                        && let Ok(parsed) = url::Url::parse(s)
                        && (parsed.password().is_some() || !parsed.username().is_empty())
                    {
                        *value = serde_json::Value::String(mask_rpc_url_mut(parsed));
                    }
                } else {
                    mask_secret_value(value);
                }
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(mask_secret_value),
        _ => {}
    }
}

pub fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.contains("key")
        || lower.contains("secret")
        || lower.contains("token")
        || lower.contains("password")
        || lower.contains("passphrase")
}

/// Mask credentials and long path segments/query strings in a URL.
pub fn mask_rpc_url_mut(mut url: url::Url) -> String {
    if url.password().is_some() {
        let _ = url.set_password(Some("***"));
    }
    if !url.username().is_empty() {
        let _ = url.set_username("***");
    }
    url.to_string()
}

/// Mask credentials in a URL string, returning the sanitized string.
/// Also masks long path segments and query strings.
pub fn mask_rpc_url(input: &str) -> String {
    if let Ok(mut url) = url::Url::parse(input) {
        if url.password().is_some() {
            let _ = url.set_password(Some("***"));
        }
        if !url.username().is_empty() {
            let _ = url.set_username("***");
        }
        let path = url.path().to_string();
        let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
        if segments.len() >= 2 && segments.last().is_some_and(looks_like_secret_segment) {
            let mut new_segments = segments;
            if let Some(last) = new_segments.last_mut() {
                *last = "***";
            }
            url.set_path(&format!("/{}", new_segments.join("/")));
        }
        if url.query().is_some() {
            url.set_query(Some("***"));
        }
        url.to_string()
    } else {
        input.to_string()
    }
}

fn looks_like_secret_segment(segment: &&str) -> bool {
    let segment = *segment;
    segment.len() >= 16
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && segment.chars().any(|c| c.is_ascii_digit())
}

/// Produce a masked JSON value from the full AppConfig, suitable for dashboard export.
pub fn mask_config(config: &AppConfig) -> serde_json::Value {
    let mut chains = config.chains.clone();
    for chain in &mut chains {
        chain.rpc = chain.rpc.iter().map(|url| mask_rpc_url(url)).collect();
    }
    let mut response = serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "name": "pano",
        "server": &config.server,
        "detector": &config.detector,
        "chains": chains,
        "ingress": &config.ingress,
        "egress": &config.egress,
        "override": &config.override_,
    });
    mask_secret_value(&mut response);
    response
}
