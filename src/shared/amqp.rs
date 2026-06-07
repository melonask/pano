use anyhow::Result;

/// Build an AMQP URL with credentials from explicit configuration fields.
/// Credentials embedded in the URL take precedence over the separate fields.
pub fn build_amqp_url(base_url: &str, username: &str, password: &str) -> Result<String> {
    let mut parsed = url::Url::parse(base_url)?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Ok(parsed.to_string());
    }
    if username.is_empty() && password.is_empty() {
        return Ok(parsed.to_string());
    }
    if username.is_empty() && !password.is_empty() {
        anyhow::bail!("queue password requires queue username");
    }
    parsed
        .set_username(username)
        .map_err(|_| anyhow::anyhow!("invalid queue username"))?;
    if !password.is_empty() {
        parsed
            .set_password(Some(password))
            .map_err(|_| anyhow::anyhow!("invalid queue password"))?;
    }
    Ok(parsed.to_string())
}
