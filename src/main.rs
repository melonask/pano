use anyhow::Context;
use clap::{Parser, Subcommand};
use pano::{
    config::{AppConfig, ServerConfig},
    run,
};
use tracing_subscriber::EnvFilter;

/// Pano — Multi-chain deposit detector
#[derive(Parser, Debug)]
#[command(name = "pano", version, about = "Argos Panoptes sees all")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    /// Path to configuration file
    #[arg(long, env = "PANO_CONFIG", default_value = "Config.toml")]
    config: String,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Validate the configured Pano namespace and every referenced shared profile.
    Check,
    /// Verify that the configured internal HTTP server and detector command loop are live.
    Healthcheck {
        /// Maximum time to wait for the internal health endpoint.
        #[arg(long, default_value_t = 3)]
        timeout_secs: u64,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();
    let args = Args::parse();
    if let Some(command) = args.command {
        match command {
            Command::Check => {
                AppConfig::load(&args.config)?;
                println!("configuration is valid");
                return Ok(());
            }
            Command::Healthcheck { timeout_secs } => {
                let config = AppConfig::load(&args.config)?;
                healthcheck(&config.server, timeout_secs).await?;
                println!("healthy");
                return Ok(());
            }
        }
    }
    if !std::path::Path::new(&args.config).exists() {
        anyhow::bail!(
            "config file not found at {}\nhelp: mount a config file or set PANO_CONFIG to a valid path",
            args.config
        );
    }
    let config = AppConfig::load(&args.config)?;
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "pano starting");
    run(config).await
}

async fn healthcheck(server: &ServerConfig, timeout_secs: u64) -> anyhow::Result<()> {
    if !server.enabled {
        anyhow::bail!("pano.server.enabled must be true to run a healthcheck");
    }
    if timeout_secs == 0 {
        anyhow::bail!("healthcheck timeout_secs must be greater than 0");
    }
    let host = match server.bind.as_str() {
        "0.0.0.0" => "127.0.0.1".to_string(),
        "::" | "[::]" => "[::1]".to_string(),
        bind if bind.contains(':') && !bind.starts_with('[') => format!("[{bind}]"),
        bind => bind.to_string(),
    };
    let url = format!("http://{host}:{}/healthz", server.port);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .context("failed to create healthcheck HTTP client")?;
    let mut request = client.get(&url);
    if !server.api_key.is_empty() {
        request = request.header("x-pano-api-key", &server.api_key);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("health endpoint {url} is unavailable"))?;
    if response.status() != reqwest::StatusCode::NO_CONTENT {
        anyhow::bail!("health endpoint {url} returned {}", response.status());
    }
    Ok(())
}
