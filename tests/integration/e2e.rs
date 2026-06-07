// ── E2E Live Test ──────────────────────────────────────────────────────────
// Launches real anvil + solana-test-validator + bitcoind regtest,
// runs Pano in-process, sends native + token transfers on all three chains,
// and verifies every deposit detected + confirmed.
//
// Prerequisites: `anvil`, `solana-test-validator`, `bitcoind`, `bitcoin-cli`
//                `cast`, `spl-token`, `solana`, `solana-keygen` on PATH.
/// Run:  cargo test e2e_multichain -- --ignored --nocapture

use pano::config::{
    AppConfig, AssetConfig, ChainConfig, DetectorConfig, EgressConfig, IngressConfig,
    OverrideConfig, RpcOptions, ServerConfig,
};
use pano::egress::EgressHandle;
use pano::ingress::IngressHandle;
use pano::model::{Command, DepositEvent, WatchSpec};
use std::process::{Child, Command as StdCommand};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};

// ── Helpers ──────────────────────────────────────────────────────────────

/// Spawn a child process, killed on drop.
struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self(Some(child))
    }
    fn kill(&mut self) {
        if let Some(mut c) = self.0.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Collect deposit events from the broadcast channel within the timeout.
async fn collect_events(
    rx: &mut broadcast::Receiver<DepositEvent>,
    timeout: Duration,
    max_count: usize,
) -> Vec<DepositEvent> {
    let mut events = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    while events.len() < max_count {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Ok(event)) => events.push(event),
            Ok(Err(broadcast::error::RecvError::Lagged(n))) => {
                eprintln!("e2e_live: broadcast lagged by {n}, skipping");
                continue;
            }
            Ok(Err(broadcast::error::RecvError::Closed)) => break,
            Err(_timeout) => break,
        }
    }
    events
}

fn app_config(chains: Vec<ChainConfig>) -> AppConfig {
    AppConfig {
        server: ServerConfig {
            enabled: false,
            bind: String::new(),
            port: 0,
            prefix: String::new(),
            dashboard: String::new(),
            dashboard_export: false,
            api_key: String::new(),
            shutdown_timeout_secs: 1,
        },
        detector: DetectorConfig {
            dedup_window_size: 10_000,
            delivery_workers: 1,
            delivery_queue_capacity: 128,
            command_queue_capacity: 256,
            stale_event_eviction_multiplier: 10,
            stale_event_eviction_min_blocks: 1_000,
            max_decimals: 30,
        },
        chains,
        ingress: IngressConfig::default(),
        egress: EgressConfig::default(),
        override_: OverrideConfig::default(),
    }
}

fn start_detector(
    config: AppConfig,
) -> (
    pano::detector::DetectorHandle,
    tokio::task::JoinHandle<()>,
    broadcast::Receiver<DepositEvent>,
) {
    let (_ingress_tx, ingress_rx) = mpsc::channel::<Command>(256);
    let ingress = IngressHandle {
        command_rx: ingress_rx,
    };
    let (events_tx, _) = broadcast::channel::<DepositEvent>(4096);
    let egress = EgressHandle {
        event_tx: events_tx.clone(),
    };
    let (handle, task) =
        pano::detector::start_with_tasks(config, ingress, egress).expect("detector start");
    let events_rx = events_tx.subscribe();
    (handle, task, events_rx)
}

/// Run bitcoin-cli with regtest credentials and -rpcport.
fn bcli(rpc_port: u16, args: &[&str]) -> String {
    let mut cmd = StdCommand::new("bitcoin-cli");
    cmd.args([
        "-regtest",
        &format!("-rpcport={rpc_port}"),
        "-rpcuser=rpcuser",
        "-rpcpassword=rpcpass",
    ]);
    cmd.args(args);
    let output = cmd.output().expect(&format!("bitcoin-cli {:?}", args));
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

/// Pick a random free TCP port.
fn rand_port() -> u16 {
    use std::net::TcpListener;
    TcpListener::bind("127.0.0.1:0")
        .map(|l| l.local_addr().unwrap().port())
        .unwrap_or(9900 + (std::process::id() % 1000) as u16)
}

// ── Test ──────────────────────────────────────────────────────────────────

#[tokio::test]
#[ignore = "requires anvil, solana-test-validator, bitcoind, bitcoin-cli, cast, spl-token, solana, solana-keygen on PATH"]
async fn e2e_multichain() {
    // ── Cleanup ─────────────────────────────────────────────────────────
    // Kill any leftover chain processes from previous failed runs.
    let _ = StdCommand::new("pkill").args(["-f", "anvil"]).output();
    let _ = StdCommand::new("pkill").args(["-f", "solana-test-validator"]).output();
    let _ = StdCommand::new("pkill").args(["-f", "bitcoind.*regtest"]).output();
    tokio::time::sleep(Duration::from_secs(1)).await;

    let sol_port = rand_port();
    let btc_port = rand_port();

    // ── 1. Spawn blockchain nodes ───────────────────────────────────────

    // Anvil (EVM, block time 1s, random port)
    let mut anvil = ChildGuard::new(
        StdCommand::new("anvil")
            .args(["--host", "0.0.0.0", "-b", "1", "--port", "0"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("anvil — install foundry"),
    );

    // Solana test validator (random port)
    let mut solana_validator = ChildGuard::new(
        StdCommand::new("solana-test-validator")
            .args([
                "--reset",
                &format!("--rpc-port={sol_port}"),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("solana-test-validator"),
    );

    // Bitcoin regtest (random port, temp datadir)
    let btc_datadir = std::env::temp_dir().join(format!("pano-e2e-btc-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&btc_datadir);

    let mut bitcoind = ChildGuard::new(
        StdCommand::new("bitcoind")
            .args([
                "-regtest",
                "-txindex",
                &format!("-datadir={}", btc_datadir.display()),
                &format!("-rpcport={btc_port}"),
                "-rpcuser=rpcuser",
                "-rpcpassword=rpcpass",
                "-rpcallowip=0.0.0.0/0",
                "-rpcbind=0.0.0.0",
                "-server",
                "-fallbackfee=0.00001",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("bitcoind"),
    );

    // ── 2. Wait for readiness ───────────────────────────────────────────

    // Anvil — parse listening port from stdout
    let anvil_url = {
        use std::io::{BufRead, BufReader};
        let stdout = anvil.0.as_mut().unwrap().stdout.take().unwrap();
        let reader = BufReader::new(stdout);
        let mut port = 0u16;
        for line in reader.lines() {
            let line = line.unwrap();
            if line.contains("Listening on") {
                port = line.rsplit(':').next().unwrap().parse().expect("anvil port");
                break;
            }
        }
        assert!(port > 0, "anvil didn't report port");
        format!("http://127.0.0.1:{port}")
    };
    eprintln!("e2e_live: anvil        on {anvil_url}");

    // Solana — poll getGenesisHash until ready
    let sol_url = format!("http://127.0.0.1:{sol_port}");
    for _ in 0..120 {
        let out = StdCommand::new("curl")
            .args([
                "-s", "-X", "POST", "-H", "Content-Type: application/json",
                "-d", r#"{"jsonrpc":"2.0","id":1,"method":"getGenesisHash"}"#,
                &sol_url,
            ])
            .output()
            .unwrap();
        if out.status.success() {
            let body: serde_json::Value =
                serde_json::from_slice(&out.stdout).unwrap_or_default();
            if body.get("result").is_some() {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    let genesis_hash = {
        let resp = StdCommand::new("curl")
            .args([
                "-s", "-X", "POST", "-H", "Content-Type: application/json",
                "-d", r#"{"jsonrpc":"2.0","id":1,"method":"getGenesisHash"}"#,
                &sol_url,
            ])
            .output()
            .unwrap();
        let body_str = String::from_utf8_lossy(&resp.stdout);
        let body: serde_json::Value = serde_json::from_slice(&resp.stdout)
            .expect(&format!("solana genesis hash parse: {body_str}"));
        body["result"].as_str().unwrap().to_string()
    };
    let sol_caip2 = format!("solana:{genesis_hash}");
    eprintln!("e2e_live: solana       on {sol_url}  caip2={sol_caip2}");

    // Bitcoin — poll getblockchaininfo until ready
    let btc_url = format!("http://127.0.0.1:{btc_port}");
    for _ in 0..120 {
        let out = StdCommand::new("curl")
            .args([
                "-s", "--user", "rpcuser:rpcpass",
                "-X", "POST", "-H", "Content-Type: application/json",
                "-d", r#"{"jsonrpc":"1.0","id":1,"method":"getblockchaininfo"}"#,
                &btc_url,
            ])
            .output()
            .unwrap();
        if out.status.success() {
            let body: serde_json::Value =
                serde_json::from_slice(&out.stdout).unwrap_or_default();
            if body.get("result").is_some() {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    bcli(btc_port, &["createwallet", "testwallet"]);
    bcli(btc_port, &["-generate", "101"]);
    let btc_addr = bcli(btc_port, &["getnewaddress"]);
    eprintln!("e2e_live: bitcoin      on {btc_url}  watching {btc_addr}");

    // ── 3. Deploy test tokens ───────────────────────────────────────────

    // ERC-20 USDT on Anvil
    let usdt_contract = {
        use std::io::Write;
        let tmp = std::env::temp_dir();
        let token_dir = tmp.join(format!("pano-e2e-token-{}", std::process::id()));
        let src_dir = token_dir.join("src");
        let _ = std::fs::create_dir_all(&src_dir);
        let token_sol = src_dir.join("Token.sol");
        let mut f = std::fs::File::create(&token_sol).unwrap();
        writeln!(f, r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;
contract Token {{
    string public name; string public symbol; uint8 public immutable decimals;
    mapping(address => uint256) public balanceOf;
    event Transfer(address indexed from, address indexed to, uint256 value);
    constructor(string memory n, string memory s, uint8 d, uint256 supply) {{
        name = n; symbol = s; decimals = d; balanceOf[msg.sender] = supply;
        emit Transfer(address(0), msg.sender, supply);
    }}
    function transfer(address to, uint256 value) external returns (bool) {{
        require(balanceOf[msg.sender] >= value, "balance");
        balanceOf[msg.sender] -= value; balanceOf[to] += value;
        emit Transfer(msg.sender, to, value); return true;
    }}
}}"#).unwrap();

        let contract_spec = format!("src/Token.sol:Token");
        let out = StdCommand::new("forge")
            .args([
                "create", &contract_spec,
                "--root", token_dir.to_str().unwrap(),
                "--rpc-url", &anvil_url,
                "--private-key",
                "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
                "--broadcast",
                "--constructor-args", "Test USDT", "USDT", "6", "1000000000000",
            ])
            .output()
            .expect("forge create USDT");
        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        combined
            .lines()
            .find(|l| l.starts_with("Deployed to:"))
            .and_then(|l| l.split_whitespace().nth(2))
            .unwrap_or_else(|| panic!("USDT deploy failed:\n{combined}"))
            .to_string()
    };
    eprintln!("e2e_live: USDT         at {usdt_contract}");

    let anvil_acct0 = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";
    let anvil_acct1 = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8";
    let anvil_pk0 = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    let anvil_pk1 = "0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d";

    // Fund acct1 with USDT from deployer acct0
    let _ = StdCommand::new("cast")
        .args([
            "send", &usdt_contract, "transfer(address,uint256)",
            anvil_acct1, "500000000",
            "--private-key", anvil_pk0,
            "--rpc-url", &anvil_url, "--legacy",
        ])
        .output()
        .expect("cast send USDT fund");

    // SPL USDC on Solana — use default keypair as sender (simpler)
    let usdc_mint = {
        let out = StdCommand::new("spl-token")
            .args(["create-token", "--decimals", "6", "--url", &sol_url])
            .output()
            .expect("spl-token create-token");
        let stdout = String::from_utf8_lossy(&out.stdout);
        stdout
            .lines()
            .find(|l| l.starts_with("Creating token"))
            .and_then(|l| l.split_whitespace().nth(2))
            .expect("create SPL token")
            .to_string()
    };
    eprintln!("e2e_live: USDC mint    = {usdc_mint}");

    // Create ATA for default keypair and mint
    let _ = StdCommand::new("spl-token")
        .args(["create-account", &usdc_mint, "--url", &sol_url])
        .output()
        .expect("spl-token create-account default");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mint_out = StdCommand::new("spl-token")
        .args(["mint", &usdc_mint, "1000", "--url", &sol_url])
        .output()
        .expect("spl-token mint");
    let mint_stdout = String::from_utf8_lossy(&mint_out.stdout);
    let mint_stderr = String::from_utf8_lossy(&mint_out.stderr);
    if !mint_stdout.contains("Minting") && !mint_stdout.contains("Signature") {
        panic!("SPL mint failed: {mint_stdout} {mint_stderr}");
    }
    eprintln!("e2e_live: minted 1000 USDC to default keypair");

    // Create recipient keypair
    let sol_key = format!("/tmp/pano-e2e-live-keypair-{}", std::process::id());
    let _ = StdCommand::new("solana-keygen")
        .args(["new", "--no-bip39-passphrase", "-o", &sol_key, "--force"])
        .output()
        .expect("solana-keygen recipient");
    let sol_test_pubkey = {
        let out = StdCommand::new("solana")
            .args(["address", "-k", &sol_key])
            .output()
            .expect("solana address recipient");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    // Fund recipient with SOL
    let _ = StdCommand::new("solana")
        .args(["transfer", &sol_test_pubkey, "2", "--allow-unfunded-recipient", "--url", &sol_url])
        .output()
        .expect("solana transfer fund");
    eprintln!("e2e_live: sol pubkey (recipient) = {sol_test_pubkey}");

    // ── 4. Build Pano config ────────────────────────────────────────────

    let chains = vec![
        // EVM (Anvil, chain ID 31337)
        ChainConfig {
            caip2: "eip155:31337".to_string(),
            start_block: None,
            end_block: None,
            confirmed_blocks: 1,
            rpc: vec![anvil_url.clone()],
            rpc_options: Some(RpcOptions {
                scan_interval_secs: 1,
                scan_lookback_blocks: 50,
                max_native_scan_per_cycle: 20,
                evm_log_address_batching: true,
                ..RpcOptions::default()
            }),
            assets: vec![
                AssetConfig {
                    symbol: "ETH".to_string(),
                    contract: None,
                    token_program: None,
                    decimals: 18,
                    min_amount: None,
                },
                AssetConfig {
                    symbol: "USDT".to_string(),
                    contract: Some(usdt_contract.clone()),
                    token_program: None,
                    decimals: 6,
                    min_amount: None,
                },
            ],
        },
        // Solana
        ChainConfig {
            caip2: sol_caip2.clone(),
            start_block: None,
            end_block: None,
            confirmed_blocks: 1,
            rpc: vec![sol_url.clone()],
            rpc_options: Some(RpcOptions {
                scan_interval_secs: 1,
                scan_lookback_blocks: 32,
                solana_scan_mode: pano::config::SolanaScanMode::Blocks,
                ..RpcOptions::default()
            }),
            assets: vec![
                AssetConfig {
                    symbol: "SOL".to_string(),
                    contract: None,
                    token_program: None,
                    decimals: 9,
                    min_amount: None,
                },
                AssetConfig {
                    symbol: "USDC".to_string(),
                    contract: Some(usdc_mint.clone()),
                    token_program: None,
                    decimals: 6,
                    min_amount: None,
                },
            ],
        },
        // Bitcoin regtest
        ChainConfig {
            caip2: "bip122:0f9188f13cb7b2c71f2a335e3a4fc328".to_string(),
            start_block: None,
            end_block: None,
            confirmed_blocks: 1,
            rpc: vec![format!("http://rpcuser:rpcpass@127.0.0.1:{btc_port}")],
            rpc_options: Some(RpcOptions {
                scan_interval_secs: 1,
                scan_lookback_blocks: 30,
                ..RpcOptions::default()
            }),
            assets: vec![AssetConfig {
                symbol: "BTC".to_string(),
                contract: None,
                token_program: None,
                decimals: 8,
                min_amount: None,
            }],
        },
    ];

    let config = app_config(chains);

    // ── 5. Start detector ───────────────────────────────────────────────

    let (handle, _task, mut events_rx) = start_detector(config);

    // ── 6. Watch addresses ──────────────────────────────────────────────

    for addr in [anvil_acct0, anvil_acct1, &*sol_test_pubkey, &*btc_addr] {
        handle
            .cmd_tx
            .send(Command::Watch(Box::new(WatchSpec {
                address: Some(addr.to_string()),
                chains: vec![],
                egress: None,
            })))
            .await
            .expect("watch");
    }

    tokio::time::sleep(Duration::from_millis(500)).await;

    // ── 7. Send deposits ────────────────────────────────────────────────

    // EVM: 0.001 ETH from acct0 -> acct1
    let _ = StdCommand::new("cast")
        .args([
            "send", "--private-key", anvil_pk0,
            "--value", "1000000000000000",
            anvil_acct1, "--rpc-url", &anvil_url, "--legacy",
        ])
        .output()
        .expect("cast send ETH");

    // EVM: 0.001 USDT from acct1 -> acct0
    let _ = StdCommand::new("cast")
        .args([
            "send", &usdt_contract, "transfer(address,uint256)",
            anvil_acct0, "1000",
            "--private-key", anvil_pk1,
            "--rpc-url", &anvil_url, "--legacy",
        ])
        .output()
        .expect("cast send USDT");

    // Solana: 0.001 SOL -> test pubkey
    let _ = StdCommand::new("solana")
        .args([
            "transfer", &sol_test_pubkey, "0.001",
            "--allow-unfunded-recipient", "--url", &sol_url,
        ])
        .output()
        .expect("solana transfer SOL");

    // Solana: 0.001 USDC from default sender -> recipient
    let usdc_xfer = StdCommand::new("spl-token")
        .args([
            "transfer", &usdc_mint, "0.001", &sol_test_pubkey,
            "--allow-unfunded-recipient", "--fund-recipient",
            "--url", &sol_url,
        ])
        .output()
        .expect("spl-token transfer USDC");
    let usdc_out = format!(
        "{}\n{}",
        String::from_utf8_lossy(&usdc_xfer.stdout),
        String::from_utf8_lossy(&usdc_xfer.stderr)
    );
    if !usdc_out.contains("Signature") {
        panic!("SPL transfer failed: {usdc_out}");
    }
    eprintln!("e2e_live: USDC transferred");

    // Bitcoin: 0.001 BTC -> watched address, then mine
    let _ = bcli(btc_port, &["sendtoaddress", &btc_addr, "0.001"]);
    bcli(btc_port, &["-generate", "1"]);

    // ── 8. Collect & verify events ──────────────────────────────────────

    // 5 transfers × 2 (detected + confirmed) = 10 events, but
    // past-detected events (deployer mints, fund transfers) from the initial
    // scan may appear first. Collect generously.
    let events = collect_events(&mut events_rx, Duration::from_secs(90), 50).await;

    eprintln!("e2e_live: collected {} events", events.len());
    for ev in &events {
        eprintln!(
            "  {:>12}  {:20}  {:4}  {:42}  {:>15}",
            &ev.event[13..], ev.data.caip2, ev.data.symbol, ev.data.address, ev.data.amount
        );
    }

    assert!(
        events.len() >= 10,
        "expected >= 10 events, got {}",
        events.len()
    );

    let check = |symbol: &str, expected_amount: &str| {
        let matching: Vec<_> = events.iter().filter(|e| e.data.symbol == symbol).collect();
        assert!(
            matching.len() >= 2,
            "{symbol}: expected >= 2 events, got {}",
            matching.len()
        );
        assert!(
            matching.iter().any(|e| e.event == "pano.deposit.detected"),
            "{symbol}: missing detected"
        );
        assert!(
            matching.iter().any(|e| e.event == "pano.deposit.confirmed"),
            "{symbol}: missing confirmed"
        );
        assert!(
            matching.iter().any(|e| e.data.amount == expected_amount),
            "{symbol}: expected amount {expected_amount}, got: {amounts:?}",
            amounts = matching.iter().map(|e| &e.data.amount).collect::<Vec<_>>()
        );
    };

    check("ETH", "1000000000000000");
    check("USDT", "1000");
    check("SOL", "1000000");
    check("USDC", "1000");
    check("BTC", "100000");

    // ── 9. Shutdown ─────────────────────────────────────────────────────

    handle.cmd_tx.send(Command::Shutdown).await.expect("shutdown");

    anvil.kill();
    solana_validator.kill();
    bitcoind.kill();

    let _ = std::fs::remove_file(&sol_key);
    let _ = std::fs::remove_dir_all(&btc_datadir);

    eprintln!("e2e_live: PASS — 5 tokens × 2 = 10+ events (EVM + Solana + Bitcoin)");
}
