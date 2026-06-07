# Pano

<img align="right" src="https://raw.githubusercontent.com/melonask/pano/refs/heads/main/logo.svg" alt="Pano monitors blockchain addresses across multiple chains (EVM, Bitcoin, Solana) and emits standardised deposit events the moment a transfer is detected" width="160" />

> **Argos Panoptes sees all** — a multi-chain, real-time deposit detector.

Pano monitors blockchain addresses across multiple chains (EVM, Bitcoin, Solana) and emits standardised deposit events the moment a transfer is detected. It is designed to run as a sidecar service or standalone daemon, feeding downstream systems via files, databases, message queues, webhooks, SSE, or WebSockets.

---

## Features

- **Multi-chain scanning** — Ethereum/EVM-compatible chains, Bitcoin, and Solana, all from one process.
- **Two-phase deposit events** — `pano.deposit.detected` fires on first observation; `pano.deposit.confirmed` fires after the configured confirmation depth.
- **Flexible ingress** — watch addresses via HTTP API, file (JSON / JSONL / CSV), PostgreSQL, SQLite, or AMQP message queue.
- **Flexible egress** — deliver events to files, PostgreSQL, SQLite, AMQP, webhook (HMAC-signed), Server-Sent Events, or WebSocket.
- **Per-address egress overrides** — each watched address can route its events to a different egress target, gated by operator-controlled permissions.
- **In-memory deduplication** — a configurable sliding window prevents duplicate events across scan cycles.
- **Hot-reload address list** — file and database ingress sources are polled for changes at runtime; the watched set updates without restarting.
- **Config-driven, no code changes required** — all behaviour is controlled through a single TOML file.

---

## Design Philosophy

Pano is built on four principles that govern every design decision. Contributors and downstream users should understand these to avoid introducing unnecessary complexity.

### 1. Configuration over hardcoding

All behaviour is driven by a single TOML config file. There are no hardcoded values, no environment-specific assumptions, and no code paths that cannot be altered through configuration. When adding a feature, ask: *can this be turned off or tuned through config without a code change?* If not, it does not belong.

### 2. Stateless by design

Pano does not own, persist, or manage state. All in-process state (scan cursors, dedup window, unconfirmed events) is ephemeral — it lives only for the lifetime of the process and is discarded on restart. The watched address set comes exclusively from caller-controlled ingress sources. Pano never writes state to disk for its own purposes; it is a pipeline, not a database.

### 3. Simplicity — `ingress → tracking → egress`

The processing flow follows a single, linear path: addresses come in through **ingress**, the detector **tracks** deposits across chains, and events go out through **egress**. There is exactly one internal channel between each stage. No hidden abstraction layers, no plugin systems, no internal routing graphs. Every piece of complexity must earn its place by providing clear, demonstrable value over a simpler alternative.

### 4. Consistency

Data structures, config keys, API endpoints, event schemas, error envelopes, and naming conventions follow the same patterns everywhere. A field called `confirmed_blocks` means the same thing in every context. An error response always uses `{"error":"...","message":"..."}`. Module layout mirrors domain boundaries (`chain/`, `ingress/`, `egress/`, `detector/`). New code should look like existing code — predictability over cleverness.

---

## Quick Start

### Prerequisites

- Rust 1.96+ (matches `rust-version` in `Cargo.toml`)
- Access to one or more JSON-RPC nodes for your target chains

### Install

```bash
cargo install pano
```

### Run

#### Local cargo

```bash
# Copy and edit the example config, then run:
cp Config.example.toml Config.toml
PANO_CONFIG=Config.toml pano
```

Or explicitly:

```bash
pano --config /path/to/Config.toml
```

The config file path defaults to `Config.toml` and can be set via the `PANO_CONFIG` environment variable or the `--config`/`-c` flag.

#### Local Docker build

```bash
docker build -t pano .
docker run --rm \
  --name pano \
  -p 3210:3210 \
  -v "$(pwd)/Config.toml:/etc/pano/Config.toml:ro" \
  pano
```

Override the config path with `PANO_CONFIG` or `--config`:

```bash
docker run --rm \
  --name pano \
  -p 3210:3210 \
  -v "$(pwd)/Config.toml:/etc/pano/Config.toml:ro" \
  pano --config /etc/pano/Config.toml
```

#### Remote GHCR image

Pre-built images are published to `ghcr.io/melonask/pano:latest` on every push to `main`.

```bash
docker run --rm \
  --name pano \
  -p 3210:3210 \
  -v "$(pwd)/Config.toml:/etc/pano/Config.toml:ro" \
  ghcr.io/melonask/pano:latest
```

> **Mount note:** The image expects the config at `/etc/pano/Config.toml` (set via the `PANO_CONFIG` env var). Mount your config file read-only at that path, or set `PANO_CONFIG` / `--config` to a different location inside the container. Map port `3210` (the default HTTP server port) to your host to access the API, SSE, and WebSocket endpoints.

To verify a running container without depending on the optional HTTP server, use the CLI probe:

```bash
docker exec pano pano ping
```

Docker image healthchecks use the same command, so `[server].enabled = false` deployments remain probeable.

#### Local End-To-End Test

The repository includes an e2e harness that runs Pano, Ladon, Postgres, and a
small Python app against local (non-containerised) blockchains. Full
architecture and operational walkthrough live in
[`tests/e2e/README.md`](tests/e2e/README.md).

**Prerequisites**

- Docker Desktop 24+ (or Docker Engine + Compose v2)
- Ports 8545, 8899, 18443, 3210, and 8080 free
- `solana-test-validator`, `anvil`, `cast`, and `forge` installed and on `$PATH`
- `bitcoin-cli` and `bitcoind` installed and on `$PATH`

**Quick start**

```bash
# 1. Start local blockchain nodes (separate terminals)
anvil --host 0.0.0.0 -b 1
solana-test-validator --reset
bitcoind -regtest -txindex -rpcuser=rpcuser -rpcpassword=rpcpass \
  -rpcallowip=0.0.0.0/0 -rpcbind=0.0.0.0 -server -fallbackfee=0.00001 -daemon

# 2. Deploy test tokens and gather env vars (see tests/e2e/README.md)

# 3. Build and launch
docker compose -f tests/e2e/docker-compose.e2e.yml --env-file /tmp/pano-e2e.env up -d

# 4. Run through the manual scenario documented in tests/e2e/README.md
```

See [`tests/e2e/README.md`](tests/e2e/README.md) for the full manual test
scenario covering user registration, deposit sending, event verification, and
balance checks.

**Lightweight alternative** (no Docker):
```bash
cargo test e2e_multichain -- --ignored --nocapture
```
Spawns Anvil, Solana test validator, and Bitcoin regtest as child processes,
runs Pano in-process, sends native + token transfers on all three chains, and
asserts every deposit detected + confirmed.

Prerequisites: `anvil`, `solana-test-validator`, `bitcoind`, `bitcoin-cli`,
`cast`, `spl-token`, `solana`, `solana-keygen` on `$PATH`.

---

## Configuration

Pano is configured through a single TOML file. Sensitive values (RPC keys, passwords) can be injected via `${ENV_VAR}` placeholders. **Unset `${VAR}` references produce a config load error** — they are not silently replaced with empty strings.

Commented starting point: see [Config.example.toml](Config.example.toml) in the repository root.

### `[server]`

| Field | Default | Description |
|-------|---------|-------------|
| `enabled` | `false` | Enable the HTTP server. |
| `bind` | `"0.0.0.0"` | Bind address. |
| `port` | `3210` | Listen port. |
| `prefix` | `"v1"` | URL prefix for API routes. |
| `api_key` | `""` | When non-empty, all routes require `Authorization: Bearer <key>` or `X-Pano-API-Key: <key>`. |
| `dashboard` | `""` | Static dashboard directory served via HTTP at `/{prefix}/{dashboard}`. |
| `dashboard_export` | `false` | When `true`, export masked `config.json` and `addresses.json` into the dashboard directory. |
| `shutdown_timeout_secs` | `1` | Seconds to wait for background tasks during graceful shutdown. |

### `[detector]`

| Field | Default | Description |
|-------|---------|-------------|
| `dedup_window_size` | `100000` | Maximum recent deposit event keys retained for in-memory deduplication. Set to `0` for unbounded (no eviction). |
| `delivery_workers` | `8` | Number of async workers used for per-address egress override delivery. |
| `delivery_queue_capacity` | `4096` | Bounded internal queue capacity for per-address egress override delivery. When full, override delivery events are dropped rather than blocking scanning. |
| `command_queue_capacity` | `256` | Bounded command queue capacity between ingress and detector. |
| `stale_event_eviction_multiplier` | `10` | Multiplier (× `confirmed_blocks`) used when evicting stale unconfirmed events. |
| `stale_event_eviction_min_blocks` | `1000` | Minimum block/slot distance before stale unconfirmed events are evicted. |
| `max_decimals` | `30` | Maximum asset decimal places accepted by config and runtime watch overrides. |

### `[[chains]]`

| Field | Required | Description |
|-------|----------|-------------|
| `caip2` | Yes | CAIP-2 chain identifier (e.g. `"eip155:1"`, `"bip122:000000000019d6689c085ae165831e93"`, `"solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp"`). |
| `confirmed_blocks` | Yes | Number of blocks after which a deposit is considered confirmed. Must be ≥ 1. Also overridable per-watch via `ChainEntry.confirmed_blocks`. |
| `rpc` | Yes | List of JSON-RPC endpoint URLs (failover order). Supports `${VAR}` substitution. |
| `start_block` | No | Block to start scanning from. Per-watch `start_block` (from `ChainEntry`) takes priority as minimum across active watches. |
| `end_block` | No | Block to stop at. `0` or omitted = follow the chain tip. Per-watch `end_block` takes priority as minimum across active watches. |
| `rpc_options` | No | Fine-grained RPC behaviour (see below). |

#### `[[chains.assets]]`

| Field | Required | Description |
|-------|----------|-------------|
| `symbol` | Yes | Asset ticker (e.g. `"ETH"`, `"USDC"`). |
| `decimals` | Yes | Decimal places for human-readable display (max 30). |
| `contract` | No | Token contract address. Omit for native chain currency. |
| `token_program` | No | Solana token program for ATA derivation. Omit to scan both classic SPL Token and Token-2022 ATAs; set explicitly when known. |
| `min_amount` | No | Minimum deposit in the smallest unit (e.g. wei). Deposits below this threshold are silently ignored. String of digits without leading zeros. |

#### `[chains.rpc_options]`

| Field | Default | Description |
|-------|---------|-------------|
| `max_concurrent` | `10` | Maximum concurrent RPC requests. |
| `delay_ms` | `0` | Fixed delay between RPC calls (rate limiting). |
| `batch_size` | `200` | Block batch size for EVM log queries; block count per cycle for Bitcoin. |
| `scan_interval_secs` | `5` | Seconds between scan attempts for this chain. |
| `evm_log_address_batching` | `true` | EVM only. Query all actively watched ERC-20 contracts in one `eth_getLogs` request using address-array filters. Set to `false` for local/dev RPCs that reject address arrays. |
| `scan_lookback_blocks` | `50` | Re-scan this many blocks/slots behind the cursor on each cycle (for reorg safety). If `start_block` is omitted, the initial scan starts at current tip minus this value. Solana uses an effective default of `500` slots when this field is omitted because slots are much faster than EVM blocks. |
| `scan_timeout_secs` | `60` | Maximum wall-clock seconds for one chain scan attempt. Slow/rate-limited chains time out without blocking ready results from other chains. |
| `max_native_scan_per_cycle` | `100` | EVM only. Maximum blocks fetched in full for native coin scanning per cycle. |
| `request_timeout_secs` | `15` | Per-request HTTP timeout. |
| `max_retries` | `3` | Number of failover retry rounds across all configured endpoints. |
| `retry_base_ms` | `500` | Base delay for exponential backoff between retry rounds (doubles each round). |
| `solana_max_supported_transaction_version` | `0` | Solana only. `maxSupportedTransactionVersion` for `getTransaction`. |
| `solana_scan_mode` | `"blocks"` | Solana scanning strategy. `"blocks"` (default) uses per-slot `getBlock` with `transactionDetails=full` — zero dependency on RPC signature indexing, same per-cycle cost regardless of watched address count. `"signatures"` uses the legacy per-address `getSignaturesForAddress` + `getTransaction` path. |

### `[override]`

Controls which fields an ingress `WatchSpec` may override. Disallowed fields are rejected with `400 Bad Request`; they are not silently ignored.

#### `[override.chains]`

If `[override.chains]` is present, HTTP/file/queue callers may provide a `chains` array in `WatchSpec` and override per-watch chain settings: `start_block`, `end_block`, and `confirmed_blocks`. RPC endpoints and `rpc_options` are never per-watch overridable.

| Field | Default | Description |
|-------|---------|-------------|
| `assets` | `false` | Allow `ChainEntry.assets`, including asset selection, asset-level address overrides, `min_amount`, and custom token assets with `symbol`, `contract`, and `decimals`. When `false` (default), `assets` must not appear in chain entries. |

To disable all per-watch chain overrides, omit the `[override.chains]` table from the config.

#### `[override.egress]`

Per-address egress overrides are disabled by default and must be explicitly whitelisted per channel.

| Field | Default | Description |
|-------|---------|-------------|
| `webhook` | `false` | Allow `egress.webhook` overrides in `WatchSpec`. |
| `file` | `false` | Allow `egress.file` overrides in `WatchSpec`. |
| `pg` | `false` | Allow `egress.pg` overrides in `WatchSpec`. |
| `sqlite` | `false` | Allow `egress.sqlite` overrides in `WatchSpec`. |
| `queue` | `false` | Allow `egress.queue` overrides in `WatchSpec`. |
| `http` | `false` | Reserved for HTTP egress override permission; SSE/WebSocket are server-level channels. |

Example:

```toml
[override]

[override.chains]
# default: false — explicitly set true to allow per-watch asset overrides
assets = true

[override.egress]
webhook = true
file = false
pg = false
sqlite = false
queue = false
http = false
```

### `[ingress.*]`

Multiple ingress sources can be active simultaneously; they are merged into a single watched-address set.

| Field | Default | Description |
|-------|---------|-------------|
| `command_queue_capacity` | `4096` | Bounded command queue capacity for ingress sources. |

| Source | Config key | Description |
|--------|-----------|-------------|
| HTTP API | `ingress.http` | REST endpoints to add/remove addresses at runtime. |
| File | `ingress.file` | JSON array, JSONL, or CSV file. Hot-reloaded on modification. |
| PostgreSQL | `ingress.pg` | Poll a PostgreSQL table for address records. |
| SQLite | `ingress.sqlite` | Poll a SQLite table for address records. |
| AMQP | `ingress.queue` | Consume Watch/Unwatch messages from an AMQP exchange. |

**Address record format** (JSON / JSONL — a `WatchSpec`):

```json
{
  "address": "0xabc...",
  "chains": [
    { "caip2": "eip155:1", "assets": [{ "symbol": "ETH" }, { "symbol": "USDC" }] }
  ],
  "egress": {
    "webhook": { "url": "https://example.com/hook", "secret": "hmac-secret" }
  }
}
```

`chains` is an array of `ChainEntry` objects keyed by `caip2`. Each may specify `assets`, `start_block`, `end_block`, `confirmed_blocks`. The root `address` serves as fallback for chain entries that omit their own.

**CSV format:** `address,chains_json,egress_json` — the last two columns are optional JSON arrays/objects. There is no header row; every CSV row is treated as data.

#### Ingress file

| Field | Default | Description |
|-------|---------|-------------|
| `enabled` | `false` | Enable file ingress. |
| `path` | `""` | Path to the address file (`.json`, `.jsonl`, `.csv`). |
| `poll_interval_secs` | `5` | How often to check for file modifications. |
| `authoritative` | `true` | When `true`, the file contents replace the entire watched set (`SyncAll`). When `false`, diffs are computed and only changes are applied. |

#### Ingress SQLite

| Field | Default | Description |
|-------|---------|-------------|
| `enabled` | `false` | Enable SQLite ingress. |
| `path` | `""` | Path to SQLite database file. |
| `poll_interval_secs` | `5` | Polling interval. Set to `0` to load once and return; the ingress task finishes while Pano continues scanning. |
| `table.name` | `"watched_addresses"` | SQLite table name. |
| `table.columns.address` | `"address"` | Column for the address (TEXT, part of composite PK). |
| `table.columns.caip2` | `"caip2"` | Column for the CAIP-2 chain ID (TEXT, part of composite PK). |
| `table.columns.symbol` | `"symbol"` | Column for the asset symbol (TEXT, part of composite PK). |
| `table.columns.asset_config` | `"asset_config"` | Column for asset_config JSON (TEXT, nullable). |
| `table.columns.chain_config` | `"chain_config"` | Column for chain_config JSON (TEXT, nullable). |
| `table.columns.egress` | `"egress"` | Column for egress override JSON (TEXT, nullable). |

#### Ingress PostgreSQL

| Field | Default | Description |
|-------|---------|-------------|
| `enabled` | `false` | Enable PostgreSQL ingress. |
| `url` | `""` | Connection URL (`postgres://` or `postgresql://`). |
| `poll_interval_secs` | `5` | Polling interval. Set to `0` to load once and return; the ingress task finishes while Pano continues scanning. |
| `table.name` | `"watched_addresses"` | Table name. |
| `table.columns.address` | `"address"` | Address column (text, part of composite PK). |
| `table.columns.caip2` | `"caip2"` | CAIP-2 chain ID column (text, part of composite PK). |
| `table.columns.symbol` | `"symbol"` | Asset symbol column (text, part of composite PK). |
| `table.columns.asset_config` | `"asset_config"` | Asset config JSON column (text, nullable). |
| `table.columns.chain_config` | `"chain_config"` | Chain config JSON column (text, nullable). |
| `table.columns.egress` | `"egress"` | Egress override JSON column (text, nullable). |

#### Ingress AMQP queue

Consumes messages from a durable topic exchange. Routes with two binding keys:

| Routing key | Payload | Action |
|-------------|---------|--------|
| `watch_routing_key` | `WatchSpec` JSON (`{"address":"...", "chains":[...], "egress":{...}}`) | Add/update watched address |
| `unwatch_routing_key` | `UnwatchAddressRequest` JSON (`{"address":"..."}`) | Remove watched address |

| Field | Default | Description |
|-------|---------|-------------|
| `enabled` | `false` | Enable AMQP ingress. |
| `url` | `""` | AMQP broker URL (e.g. `amqp://host` or `amqps://host`). |
| `username` | `""` | AMQP username (only used if URL has no credentials). |
| `password` | `""` | AMQP password (only used if URL has no credentials). |
| `exchange` | `""` | Topic exchange name. Declared durable. |
| `watch_routing_key` | `""` | Binding key for Watch messages. |
| `unwatch_routing_key` | `""` | Binding key for Unwatch messages. |
| `reconnect_secs` | `5` | Delay before reconnecting on failure. |
| `qos_prefetch` | `100` | AMQP QoS prefetch count. |
| `consumer_tag` | `"pano-ingress"` | Consumer tag for this instance. |

#### Ingress HTTP

| Field | Default | Description |
|-------|---------|-------------|
| `enabled` | `false` | Enable the HTTP address management routes. |
| `addresses` | `"addresses"` | URL path segment under `prefix` for add/remove routes. |

#### Ingress usage examples

HTTP add/remove:

```bash
curl -X POST http://localhost:3210/v1/addresses \
  -H 'Content-Type: application/json' \
  -d '{"address":"0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5"}'

curl -X DELETE http://localhost:3210/v1/addresses/0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5
```

File ingress (`addresses.jsonl`; `.json` is an array, `.csv` is `address,chains_json,egress_json`):

```jsonl
{"address":"0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5","chains":[{"caip2":"eip155:1","assets":[{"symbol":"USDC"}]}]}
{"address":"bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080","chains":[{"caip2":"bip122:000000000019d6689c085ae165831e93"}]}
```

SQLite/PostgreSQL ingress table rows:

```sql
CREATE TABLE watched_addresses (
  address      TEXT NOT NULL,
  caip2        TEXT NOT NULL,
  symbol       TEXT NOT NULL,
  asset_config TEXT,
  chain_config TEXT,
  egress       TEXT,
  PRIMARY KEY (address, caip2, symbol)
);

INSERT INTO watched_addresses (address, caip2, symbol, asset_config) VALUES
('0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5', 'eip155:1', 'USDC', NULL);
```

AMQP ingress publishes `WatchSpec` to `watch_routing_key` and unwatch requests to `unwatch_routing_key`:

```json
{"address":"0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5","chains":[{"caip2":"eip155:1"}]}
```

```json
{"address":"0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5"}
```

### `[egress]`

The top-level `[egress]` section has one shared field:

| Field | Default | Description |
|-------|---------|-------------|
| `broadcast_capacity` | `4096` | Size of the internal broadcast channel. The oldest event is dropped if all receivers are slow. |

#### Egress targets

| Target | Config key | Description |
|--------|-----------|-------------|
| File | `egress.file` | Append events to a file. Format inferred from extension (`.json`, `.jsonl`, `.csv`). |
| PostgreSQL | `egress.pg` | Insert events into a PostgreSQL table. |
| SQLite | `egress.sqlite` | Insert events into a SQLite table. |
| AMQP | `egress.queue` | Publish events to an AMQP topic exchange. |
| Webhook | `egress.webhook` | POST events to an HTTP endpoint, signed with HMAC-SHA256. |
| SSE | `egress.http` → `sse` | Server-Sent Events stream under the `prefix`. |
| WebSocket | `egress.http` → `websocket` | WebSocket stream under the `prefix`. |

#### Egress file

| Field | Default | Description |
|-------|---------|-------------|
| `enabled` | `false` | Enable file egress. |
| `path` | `""` | Destination file path. `.json` writes a full JSON array (rewritten each event), `.jsonl`/`.csv` append atomically. |

CSV egress columns are written in this order: `event_id`, `event`, `version`, `occurred_at`, `tx_id`, `caip2`, `symbol`, `address`, `block_number`, `log_index`, `amount`, `sender`, `confirmations`, `timestamp`. The internal per-address egress override is not included.

#### Egress SQLite

| Field | Default | Description |
|-------|---------|-------------|
| `enabled` | `false` | Enable SQLite egress. |
| `path` | `""` | Path to SQLite database file. |
| `table.name` | `"deposit_events"` | Table name. |
| `table.columns.*` | (see config example) | All 15 column names are overridable (envelope + data). |

#### Egress PostgreSQL

| Field | Default | Description |
|-------|---------|-------------|
| `enabled` | `false` | Enable PostgreSQL egress. |
| `url` | `""` | Connection URL (`postgres://` or `postgresql://`). |
| `table.name` | `"deposit_events"` | Table name. |
| `table.columns.*` | (see config example) | All 15 column names are overridable (envelope + data). The table and deduplication index are auto-created if missing. Inserts use `ON CONFLICT DO NOTHING`. |

#### Egress AMQP queue

Publishes to a durable topic exchange with routing keys that depend on event type:

| Routing key | Event type |
|-------------|------------|
| `detected_routing_key` | `pano.deposit.detected` |
| `confirmed_routing_key` | `pano.deposit.confirmed` |

| Field | Default | Description |
|-------|---------|-------------|
| `enabled` | `false` | Enable AMQP egress. |
| `url` | `""` | AMQP broker URL. |
| `username` | `""` | AMQP username. |
| `password` | `""` | AMQP password. |
| `exchange` | `""` | Topic exchange name. |
| `detected_routing_key` | `"detected"` | Routing key for detected events. |
| `confirmed_routing_key` | `"confirmed"` | Routing key for confirmed events. |
| `reconnect_secs` | `5` | Reconnect delay on failure. |

#### Egress webhook

POSTs each deposit event as JSON with optional HMAC-SHA256 signature headers.

| Field | Default | Description |
|-------|---------|-------------|
| `enabled` | `false` | Enable webhook egress. |
| `url` | `""` | Webhook endpoint URL (http/https). |
| `secret` | `""` | HMAC-SHA256 secret. When set, adds `X-Pano-Signature` (hex) and `X-Pano-Event` headers. |
| `max_retries` | `3` | Number of retry rounds after the initial delivery attempt (4 attempts total). |
| `retry_base_ms` | `250` | Base retry delay in ms, doubled each attempt. |
| `timeout_secs` | `30` | Per-request HTTP timeout in seconds. |

Retry is performed only on server errors (5xx) and `429 Too Many Requests`. Other 4xx responses are treated as permanent failure and not retried.

#### Egress HTTP (SSE / WebSocket)

| Field | Default | Description |
|-------|---------|-------------|
| `enabled` | `false` | Enable HTTP egress routes. |
| `sse` | `"sse"` | URL path segment under `prefix` for the SSE endpoint. |
| `websocket` | `"ws"` | URL path segment under `prefix` for the WebSocket endpoint. |
| `ws_heartbeat_secs` | `30` | WebSocket ping interval. Connection is closed if pong not received before next ping. |
| `sse_keepalive_secs` | `5` | SSE keepalive interval in seconds. |
| `ws_max_message_size` | `65536` | Maximum WebSocket message size in bytes. |
| `ws_max_frame_size` | `65536` | Maximum WebSocket frame size in bytes. |

#### Egress usage examples

File egress format is inferred from extension:

```toml
[egress.file]
enabled = true
path = "events.jsonl" # .json = JSON array, .jsonl = one JSON object per line, .csv = CSV rows
```

SQLite egress auto-creates `deposit_events`. PostgreSQL egress auto-creates the configured table and deduplication index if missing. If you provide your own schema, it must match the expected column types:

```sql
CREATE TABLE deposit_events (
  event_id        TEXT PRIMARY KEY,
  event           TEXT NOT NULL,
  version         INTEGER NOT NULL,
  occurred_at     TEXT NOT NULL,
  tx_id           TEXT NOT NULL,
  caip2           TEXT NOT NULL,
  symbol          TEXT NOT NULL,
  address         TEXT NOT NULL,
  block_number    BIGINT NOT NULL,
  log_index       BIGINT NOT NULL DEFAULT 0,
  amount          TEXT NOT NULL,
  sender          TEXT NOT NULL,
  confirmations   INTEGER NOT NULL,
  timestamp       TEXT NOT NULL
);
```

AMQP egress emits JSON `DepositEvent` payloads to `detected_routing_key` or `confirmed_routing_key`:

```toml
[egress.queue]
enabled = true
url = "amqp://localhost"
exchange = "pano.deposits"
detected_routing_key = "detected"
confirmed_routing_key = "confirmed"
```

Webhook egress sends `POST` requests with `Content-Type: application/json`. When `secret` is set, consumers verify `X-Pano-Signature` as lowercase hex HMAC-SHA256 over the exact JSON request body:

```http
POST /pano HTTP/1.1
Content-Type: application/json
X-Pano-Event: pano.deposit.detected
X-Pano-Signature: <hex-hmac-sha256>

{"event_id":"01HXYZ...","event":"pano.deposit.detected","version":1,"occurred_at":"2025-06-01T12:00:00Z","data":{...}}
```

SSE and WebSocket egress are ordinary HTTP routes under the server prefix:

```bash
curl -N http://localhost:3210/v1/sse
# WebSocket: connect to ws://localhost:3210/v1/ws with any WebSocket client.
```

---

## HTTP API

All routes are prefixed with `/{prefix}` (default `v1`). When `server.api_key` is non-empty, every request must include either `Authorization: Bearer <key>` or `X-Pano-API-Key: <key>`.

### Address Management

When `ingress.http.enabled = true`:

#### Add a watched address

```
POST /v1/addresses
Content-Type: application/json
```

Request body (`WatchSpec`):

```json
{
  "address": "0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5"
}
```

Success responses:

| Status | Condition |
|--------|-----------|
| `201 Created` | Address was accepted (empty body). |

Application error responses (`ApiError` JSON — `{"error":"...","message":"..."}`):

| Status | Error code | Message | Condition |
|--------|-----------|---------|-----------|
| `400 Bad Request` | `"bad_request"` | Various validation errors | Missing address/chains, unknown caip2, invalid address format, confirmed_blocks=0, custom asset missing contract/decimals, override permissions denied. |
| `409 Conflict` | `"conflict"` | `"triad (... , ... , ...) already watched"` | Duplicate (address, caip2, symbol) triad. |
| `405 Method Not Allowed` | `"method_not_allowed"` | `"HTTP address mutations are disabled..."` | Authoritative file ingress is enabled. |

Axum's default rejection: when the request body JSON is malformed or cannot be deserialized, Axum returns its own `400 Bad Request` response **before** the handler runs. The response format differs from the application `ApiError`; it is not guaranteed to use Pano's `{"error":"...","message":"..."}` envelope.

#### Remove a watched address

```
DELETE /v1/addresses/{address}
```

Success:

| Status | Condition |
|--------|-----------|
| `204 No Content` | Address removed (empty body). |

Error:

| Status | Error code | Message | Condition |
|--------|-----------|---------|-----------|
| `404 Not Found` | `"not_found"` | `"address not watched"` | Address not in the watched set. |
| `503 Service Unavailable` | `"unavailable"` | `"detector command channel closed"` | Internal channel failure. |

### Fallback (unknown routes)

```
ANY /v1/nonexistent
```

| Status | Error code | Message |
|--------|-----------|---------|
| `404 Not Found` | `"not_found"` | `"unknown route"` |

### API Key Authentication

When `server.api_key` is set, all routes (including SSE, WebSocket, and dashboard) are protected. Missing or invalid credentials produce:

| Status | Error code | Message |
|--------|-----------|---------|
| `401 Unauthorized` | `"unauthorized"` | `"invalid or missing API key"` |

API key comparison uses constant-time verification.

### SSE stream

```
GET /v1/sse
Accept: text/event-stream
```

Streams `pano.deposit.detected` and `pano.deposit.confirmed` events as Server-Sent Events with a 5-second keep-alive. When the internal broadcast buffer overflows for this client, a `pano.stream.lag` event is emitted instead:

```
event: pano.stream.lag
data: {"missed": 42}
```

SSE events use the event type string as the SSE `event:` field (e.g. `event: pano.deposit.detected`).

### WebSocket stream

```
GET /v1/ws
Upgrade: websocket
```

Streams the same deposit events as JSON text messages. The server sends WebSocket ping frames every `ws_heartbeat_secs` (default 30s). If a pong is not received before the next ping interval, the connection is closed. On broadcast lag, a `pano.stream.lag` text message is sent:

```json
{"event": "pano.stream.lag", "data": {"missed": 42}}
```

---

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

- `event_id` — ULID, sortable and globally unique.
- `caip2` — CAIP-2 chain identifier (e.g. `"eip155:1"`).
- `symbol` — Asset ticker symbol (e.g. `"USDC"`, `"ETH"`).
- `amount` — raw integer in the asset's smallest unit (no decimal point). Divide by `10^decimals` for human-readable value.
- `log_index` — disambiguates multiple transfers in the same transaction (EVM log index, Bitcoin vout `n`, Solana account index).
- `confirmations` — always `1` for detected events; the actual depth at confirmation time for confirmed events.

---

## Per-Address Egress Overrides

Each watched address can include an `egress` object that routes its deposit events to a specific egress target in addition to the global egress broadcast. Supported channels mirror `[egress]` config sections (minus `enabled` flags):

| Channel | Required fields | Description |
|---------|----------------|-------------|
| `webhook` | `url`, optional `secret` | POST the event to the given URL with optional HMAC-SHA256 signature. |
| `file` | `path` | Write the event JSON to the given file (appends or rewrites depending on extension). |
| `sqlite` | `path` | Insert the event into a `deposit_events` table in the given SQLite database (connection pool reused). |
| `pg` | `url` | Insert the event into a `deposit_events` table in the given PostgreSQL database (connection pool reused). |
| `queue` | `url`, `exchange` | Publish the event to an AMQP topic exchange (connection reused). |

Per-address egress overrides are gated by `[override.egress.*]` booleans — each channel must be explicitly whitelisted. Example address with an egress override:

```json
{
  "address": "0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5",
  "chains": [
    { "caip2": "eip155:1", "assets": [{ "symbol": "USDC" }] }
  ],
  "egress": {
    "webhook": { "url": "https://hooks.example.com/pano", "secret": "whsec_abc123" }
  }
}
```

---

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
                 ┌──────────────────────────────┐
                 │           Egress             │
                 │  File / DB / Queue / Webhook │
                 │  SSE / WebSocket             │
                 └──────────────────────────────┘
```

Ingress and egress are fully decoupled. The detector loop is the single point of truth for the watched address set. Chain scanners are stateless functions; all state (last scanned block, unconfirmed events) lives in the detector task.

---

## Supported Chains

| Chain type | CAIP-2 namespace | Notes |
|-----------|-----------------|-------|
| EVM-compatible | `eip155` | Ethereum, Base, Polygon, Arbitrum, etc. ERC-20 via `eth_getLogs`; native ETH via `eth_getBlockByNumber`. |
| Bitcoin | `bip122` | Native BTC via `getblock` (verbosity 2). Supports legacy, P2SH, and bech32 (native SegWit/Taproot) addresses. |
| Solana | `solana` | Native SOL and SPL tokens. Default `solana_scan_mode = "blocks"` uses `getBlock` per slot (no signature-index dependency); legacy `"signatures"` mode uses `getSignaturesForAddress` + `getTransaction`. |

---

## Graceful Shutdown

Pano handles `SIGINT` and `SIGTERM` (Unix) and `Ctrl+C` (Windows). On shutdown, it:

1. Sends a `Shutdown` command to the detector loop.
2. Stops accepting new HTTP connections.
3. Waits up to `server.shutdown_timeout_secs` (default 1) for the detector, ingress, and egress tasks to finish cleanly.

---

## Environment Variables

| Variable | Description |
|----------|-------------|
| `PANO_CONFIG` | Path to the TOML config file. Default: `Config.toml`. Also settable via `--config`/`-c`. |
| `RUST_LOG` | Log filter (e.g. `pano=info`, `pano=debug`). Standard `tracing-subscriber` `EnvFilter` format. |
| Any `${VAR}` in config | Substituted at load time. **Unset variables cause a config load error**, not an empty string. |

---

## License

See `LICENSE` in the repository root.
