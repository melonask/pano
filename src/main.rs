use clap::{Parser, Subcommand};
use pano::{config::AppConfig, run};
use tracing_subscriber::EnvFilter;

/// Pano — Multi-chain deposit detector
#[derive(Parser, Debug)]
#[command(name = "pano", version, about = "Argos Panoptes sees all")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,

    /// Path to configuration file
    #[arg(short, long, env = "PANO_CONFIG", default_value = "Config.toml")]
    config: String,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Verify that the daemon process is present in this container
    Ping {
        /// Process ID to probe. Docker containers normally run the daemon as PID 1.
        #[arg(long, default_value_t = 1)]
        pid: u32,
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
            Command::Ping { pid } => {
                ping(pid)?;
                println!("pong");
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

#[cfg(target_os = "linux")]
fn ping(pid: u32) -> anyhow::Result<()> {
    let stat_path = format!("/proc/{pid}/stat");
    let stat = std::fs::read_to_string(&stat_path)
        .map_err(|e| anyhow::anyhow!("pano daemon process {pid} is not available: {e}"))?;
    let state = stat
        .rsplit_once(") ")
        .and_then(|(_, rest)| rest.chars().next())
        .ok_or_else(|| anyhow::anyhow!("unable to read process state for pid {pid}"))?;
    if state == 'Z' {
        anyhow::bail!("pano daemon process {pid} is a zombie");
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn ping(_pid: u32) -> anyhow::Result<()> {
    anyhow::bail!("pano ping requires Linux procfs and is intended for container healthchecks")
}
