---
name: pano
description: Use when the application needs blockchain deposit detection — tracking user deposits/top-ups on EVM chains, Bitcoin, or Solana. Use for cryptocurrency exchanges, payment gateways, merchant checkout flows, or any service that generates addresses and needs to know when funds arrive. Pano watches addresses across multiple chains and delivers standardized deposit events via files, databases, webhooks, message queues, SSE, or WebSockets.
---

Pano monitors blockchain addresses across multiple chains (EVM, Bitcoin, Solana) and emits standardised deposit events the moment a transfer is detected. It is designed to run as a sidecar service or standalone daemon, feeding downstream systems via files, databases, message queues, webhooks, SSE, or WebSockets.

---

## Overview

Pano watches blockchain addresses for incoming deposits. It ingests addresses via file, database, HTTP API, or AMQP queue; scans EVM, Bitcoin, and Solana chains through configurable JSON-RPC endpoints; deduplicates and tracks confirmations; and delivers standardised deposit events to files, databases, AMQP, webhooks, SSE, or WebSockets.

The processing flow is linear: `ingress → detector → egress`. All behaviour is driven by a single TOML config file.

## Feature Flags

Pano uses Cargo features to keep the default build minimal:

| Feature    | Default | Enables                                           |
|------------|---------|---------------------------------------------------|
| `sqlite`   | **yes** | SQLite ingress/egress (via `sqlx`)                |
| `server`   | no      | HTTP server, API, SSE, WebSocket, dashboard       |
| `amqp`     | no      | AMQP (RabbitMQ) ingress/egress (via `lapin`)     |
| `postgres` | no      | PostgreSQL ingress/egress (via `sqlx`)            |
| `pg`       | no      | Alias for `postgres`                              |
| `webhook`  | no      | HMAC-signed webhook egress delivery               |
| `full`     | no      | All features: `server`, `webhook`, `sqlite`, `postgres`, `amqp` |

Enable features at build time:

```bash
cargo install pano --features "server,postgres,amqp"
```

Config sections that require a disabled feature produce a clear error at startup, e.g.: `pano.egress.webhook.enabled requires feature "webhook"`.

## Install

```bash
cargo install pano
```

Or from source:

```bash
git clone https://github.com/melonask/pano
cd pano
cargo build --release
```

## Quick Start

```bash
cp Config.example.toml Config.toml
# Edit chains, assets, and RPC endpoints, then:
pano --config Config.toml
```

Or with environment variable:

```bash
PANO_CONFIG=Config.toml pano
```

## CLI

```
pano [OPTIONS] [COMMAND]

Commands:
  ping    Verify the daemon process is present (container healthcheck)
  help    Print this message or the help of the given subcommand(s)

Options:
  -c, --config <CONFIG>  Path to configuration file [env: PANO_CONFIG=Config.toml]
  -h, --help             Print help
  -V, --version          Print version
```

Pano reads `Config.toml` by default. Use `--config` to point at any TOML file, including a merged multi-package config (see Universal Integration Config below).

## Configuration

Pano is configured through a single TOML file. The configuration model has two layers:

1. **Shared root sections** — reusable profiles for chains, assets, paths, transports, and stores. These may be shared with other packages (Ladon, Bria, Oracles) when running from a merged config.
2. **`[pano]` namespace** — Pano-specific behaviour: server, detector, ingress, egress, and override permissions.

Environment variable substitution uses `${VAR}` (required) and `${VAR:-default}` (optional fallback).

### Universal Integration Config

Pano can read a merged `Config.toml` that contains sections for multiple packages. It:

- Parses shared root sections (`[chains]`, `[assets]`, `[paths]`, `[transports]`, `[stores]`, `[http]`, `[log]`, `[meta]`, `[runtime]`)
- Parses the `[pano]` namespace
- **Ignores** `[ladon]`, `[bria]`, and `[oracles]` sections
- **Rejects** unknown fields inside `[pano]`

This means the same `Config.toml` works for all packages:

```bash
ladon   --config Config.toml
pano    --config Config.toml
bria    --config Config.toml
oracles --config Config.toml
```

### Shared Profiles

Pano resolves package-local references against shared profiles:

| Pano field | Resolves to | Local override |
|---|---|---|
| `pano.chains = ["eth"]` | `[chains.eth]` | N/A |
| `pano.assets = ["eth"]` | `[assets.eth]` | N/A |
| `pano.ingress.file.path_ref = "pano_watches"` | `[paths.pano_watches].path` | `path = "..."` |
| `pano.egress.file.path_ref = "pano_events"` | `[paths.pano_events].path` | `path = "..."` |
| `pano.ingress.amqp.transport = "local"` | `[transports.amqp.local]` | N/A |
| `pano.egress.amqp.transport = "local"` | `[transports.amqp.local]` | N/A |
| `pano.egress.webhook.transport = "ops"` | `[transports.webhook.ops]` | URL, timeout, retries |
| `pano.ingress.sqlite.store = "pano"` | `[stores.pano]` | N/A |
| `pano.egress.pg.store = "postgres"` | `[stores.postgres]` | N/A |

Explicit package-local values override profile values. Unknown references fail with actionable errors.

### Package-Specific Configuration

All Pano behaviour lives under the `[pano]` namespace:

#### `[pano]`

| Field | Default | Description |
|---|---|---|
| `chains` | `[]` | Chain ids from `[chains.<id>]` to scan |
| `assets` | `[]` | Asset ids from `[assets.<id>]` to detect |

#### `[pano.server]`

Requires feature `server`.

| Field | Default | Description |
|---|---|---|
| `enabled` | `false` | Start the HTTP server |
| `bind` | `"0.0.0.0"` | Bind address |
| `port` | `3210` | Listen port |
| `prefix` | `"v1"` | URL prefix for API routes |
| `api_key` | `""` | Optional shared API key (Bearer or X-Pano-API-Key header) |
| `dashboard_path_ref` | `""` | Path profile from `[paths]` for static dashboard |
| `dashboard_export` | `false` | Export masked `config.json` and `addresses.json` into dashboard dir |
| `shutdown_timeout_secs` | `1` | Graceful shutdown timeout for background tasks |

#### `[pano.detector]`

| Field | Default | Description |
|---|---|---|
| `dedup_window_size` | `100000` | Max recent event keys for in-memory deduplication (0 = unbounded) |
| `delivery_workers` | `8` | Async workers for per-address egress override delivery |
| `delivery_queue_capacity` | `4096` | Queue capacity for override delivery |
| `command_queue_capacity` | `256` | Queue capacity between ingress and detector |
| `stale_event_eviction_multiplier` | `10` | Multiplier for stale event eviction threshold |
| `stale_event_eviction_min_blocks` | `1000` | Minimum blocks before stale event eviction |
| `max_decimals` | `30` | Maximum accepted asset decimal places |

#### `[pano.rpc_defaults]`

Applied to all chains as default RPC tuning.

| Field | Default | Description |
|---|---|---|
| `max_concurrent` | `10` | Max concurrent RPC requests per chain |
| `delay_ms` | `0` | Fixed delay between RPC calls |
| `batch_size` | `200` | Block batch size for EVM/Bitcoin scan cycles |
| `scan_lookback_blocks` | `50` | Re-scan depth for reorg safety (Solana effective default: 500) |
| `scan_interval_secs` | `5` | Minimum seconds between scans |
| `scan_timeout_secs` | `60` | Max wall-clock seconds per chain scan |
| `max_native_scan_per_cycle` | `100` | EVM native-coin blocks scanned per cycle |
| `request_timeout_secs` | `15` | Per-request RPC HTTP timeout |
| `max_retries` | `3` | Retry rounds across configured endpoints |
| `retry_base_ms` | `500` | Base delay for exponential retry backoff |
| `solana_max_supported_transaction_version` | `0` | Solana getTransaction option |
| `solana_scan_mode` | `"blocks"` | Solana scan mode: `"blocks"` or `"signatures"` |
| `evm_log_address_batching` | `true` | EVM address-array batching for eth_getLogs |

#### `[pano.overrides.chain]`

| Field | Default | Description |
|---|---|---|
| `assets` | `false` | Allow per-watch asset overrides |

#### `[pano.overrides.egress]`

| Field | Default | Description |
|---|---|---|
| `webhook` / `file` / `pg` / `sqlite` / `queue` / `http` | `false` | Allow per-watch egress channel overrides |

#### `[pano.ingress]`

| Field | Default | Description |
|---|---|---|
| `command_queue_capacity` | `4096` | Bounded command queue capacity |

- `[pano.ingress.file]` — `enabled`, `path_ref`, `path` (override), `poll_interval_secs`
- `[pano.ingress.http]` — (requires `server`) `enabled`, `path`, `transport`
- `[pano.ingress.sqlite]` — (requires `sqlite`) `enabled`, `store`, `poll_interval_secs`, `table`
- `[pano.ingress.pg]` — (requires `pg`/`postgres`) `enabled`, `store`, `poll_interval_secs`, `table`
- `[pano.ingress.amqp]` — (requires `amqp`) `enabled`, `transport`, `exchange`, `routing_key`, `consumer_tag`

#### `[pano.egress]`

- `[pano.egress.file]` — `enabled`, `path_ref`, `path` (override), `template`
- `[pano.egress.sqlite]` — (requires `sqlite`) `enabled`, `store`, `table`
- `[pano.egress.pg]` — (requires `pg`/`postgres`) `enabled`, `store`, `table`
- `[pano.egress.amqp]` — (requires `amqp`) `enabled`, `transport`, `exchange`, `detected_routing_key`, `confirmed_routing_key`
- `[pano.egress.webhook]` — (requires `webhook`) `enabled`, `transport`, `url` (override), `secret`, `signature_header`, `timeout_secs`, `max_retries`, `retry_base_ms`
- `[pano.egress.stream]` — (requires `server`) `enabled`, `sse`, `websocket`, `ws_heartbeat_secs`, `sse_keepalive_secs`, `broadcast_capacity`

## Database Backends

| Backend | Feature | Default | Notes |
|---|---|---|---|
| SQLite | `sqlite` | **Yes** | In-process; ideal for single-instance and local dev. |
| PostgreSQL | `pg` or `postgres` | No | Multi-instance; shared connection pooling. |

Database connections are configured through shared `[stores.<id>]` profiles:

```toml
[stores.pano]
driver = "sqlite"
url = "sqlite://data/pano/pano.db"

# Or for Postgres:
[stores.postgres]
driver = "postgres"
url = "${DATABASE_URL:-postgres://localhost/pano}"
```

Ingress and egress sections reference stores via `store = "<id>"`.

## Path and Transport Profiles

Instead of repeating file paths and connection details in every Pano section, define them once in shared profiles:

```toml
[paths.pano_watches]
kind = "file"
path = "data/pano/addresses.jsonl"
format = "jsonl"

[transports.amqp.local]
url = "amqp://localhost:5672"
reconnect_secs = 5

[transports.webhook.ops]
url = "${OPS_WEBHOOK_URL:-}"
timeout_secs = 10
max_retries = 3
```

Then reference them from Pano:

```toml
[pano.ingress.file]
enabled = true
path_ref = "pano_watches"

[pano.egress.amqp]
enabled = true
transport = "local"
exchange = "pano.egress"

[pano.egress.webhook]
enabled = true
transport = "ops"
```

## Environment Variables

| Variable | Description |
|---|---|
| `PANO_CONFIG` | Path to the TOML config file (default: `Config.toml`) |
| `RUST_LOG` | Log filter (e.g. `pano=info`, `pano=debug`) |

Any `${VAR}` or `${VAR:-default}` in config values is resolved at load time. Required vars without a default cause a load error.

## Examples

See [Config.example.toml](Config.example.toml) for a complete annotated example with all sections.

## Architecture

```
                 ┌──────────────────────────────┐
                 │           Ingress            │
                 │  HTTP / File / DB / Queue    │
                 └──────────────┬───────────────┘
                                │ Watch / Unwatch / SyncAll commands
                                ▼
                 ┌──────────────────────────────┐
                 │          Detector            │
                 │  Chain scanners (BTC/EVM/SOL)│
                 │  Dedup window                │
                 │  Confirmation tracker        │
                 │  Egress router               │
                 └──────────────┬───────────────┘
                                │ DepositEvent (broadcast)
                                ▼
                 └──────────────────────────────┘
                 │           Egress             │
                 │  File / DB / Queue / Webhook │
                 │  SSE / WebSocket             │
                 └──────────────────────────────┘
```

## Supported Chains

| Chain type | CAIP-2 namespace | Notes |
|---|---|---|
| EVM-compatible | `eip155` | Native ETH and ERC-20 tokens via eth_getLogs |
| Bitcoin | `bip122` | Native BTC via getblock (verbosity 2) |
| Solana | `solana` | Native SOL and SPL tokens via getBlock |

## Event Schema

All events share this envelope:

```json
{
  "event_id": "01HXYZ...",
  "event": "pano.deposit.detected",
  "version": 1,
  "occurred_at": "2025-06-01T12:00:00Z",
  "data": {
    "tx_id": "0xabc...",
    "caip2": "eip155:1",
    "symbol": "USDC",
    "address": "0xrecipient...",
    "block_number": 19000000,
    "log_index": 3,
    "amount": "1000000",
    "sender": "0xsender...",
    "confirmations": 1,
    "timestamp": "2025-06-01T11:59:58Z"
  }
}
```

## Development

```bash
# Run tests with default features (sqlite only)
cargo test

# Run all tests including optional features
cargo test --features full

# Build with specific features
cargo build --features "server,postgres,amqp"

# Lint
cargo clippy --features full

# Format
cargo fmt
```

## Testing

```bash
# Unit and integration tests
cargo test --features full

# Run end-to-end tests (requires local blockchain nodes)
cargo test e2e_multichain --features full -- --ignored --nocapture
```
