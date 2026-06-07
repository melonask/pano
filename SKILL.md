---
name: pano
description: Use when the application needs blockchain deposit detection — tracking user deposits/top-ups on EVM chains, Bitcoin, or Solana. Use for cryptocurrency exchanges, payment gateways, merchant checkout flows, or any service that generates addresses and needs to know when funds arrive. Pano watches addresses across multiple chains and delivers standardized deposit events via files, databases, webhooks, message queues, SSE, or WebSockets.
---

# Pano — Multi-chain Deposit Detector

## When to Use

Use Pano when your application needs to detect incoming blockchain deposits in real time. Common use cases:

- **Crypto exchange**: users deposit ETH/USDC/BTC/SOL to top up their balances
- **Payment gateway / merchant**: generate a unique address per order and listen for the payment
- **On-chain event monitoring**: any service that generates addresses for users and needs to react to incoming transfers

Pano supports EVM-compatible chains (Ethereum, Base, Polygon, Arbitrum, etc.), Bitcoin, and Solana — all from a single process.

Do NOT use Pano to build a blockchain indexer, to scan historical state, or for general transaction monitoring. Pano is purpose-built for deposit detection: watching a specific set of addresses and emitting events when transfers arrive.

## Quick Start

### Docker (recommended)

```bash
docker run --rm --name pano -p 3210:3210 \
  -v "$(pwd)/Config.toml:/etc/pano/Config.toml:ro" \
  ghcr.io/melonask/pano:latest
```

### Cargo

```bash
cargo install pano
cp Config.example.toml Config.toml   # edit this file
pano --config Config.toml
```

Config path defaults to `./Config.toml`, overridable via `--config` flag or `PANO_CONFIG` env var.

## Configuration

Pano is driven entirely by a single TOML config file. Sensitive values (RPC keys, passwords) use `${ENV_VAR}` substitution — **unset variables produce a load error**, never silently become empty strings.

### Minimal config (Ethereum mainnet, HTTP API + webhook)

```toml
[server]
enabled = true
port = 3210

[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["https://eth.llamarpc.com"]
[[chains.assets]]
symbol = "ETH"
decimals = 18

[ingress.http]
enabled = true

[egress.webhook]
enabled = true
url = "https://your-app.example.com/pano-webhook"
secret = "your-hmac-secret"
```

### Chain configuration

Each chain is declared as a `[[chains]]` array entry. Key fields:

| Field | Required | Description |
|-------|----------|-------------|
| `caip2` | Yes | `"eip155:1"` (Ethereum), `"eip155:8453"` (Base), `"bip122:000000000019d6689c085ae165831e93"` (Bitcoin), `"solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp"` (Solana) |
| `confirmed_blocks` | Yes | Blocks after which a deposit is considered confirmed (≥ 1). EVM: 12, Bitcoin: 6, Solana: 32 are common values |
| `rpc` | Yes | Array of JSON-RPC URLs (failover order) |
| `start_block` | No | Block to start scanning from. Omit to start from recent blocks |
| `rpc_options.scan_interval_secs` | No | Seconds between scans (default 5) |

Each chain declares its assets via `[[chains.assets]]`:

| Field | Required | Description |
|-------|----------|-------------|
| `symbol` | Yes | Ticker, e.g. `"ETH"`, `"USDC"`, `"USDT"` |
| `decimals` | Yes | Decimal places (ETH = 18, USDC = 6, BTC = 8) |
| `contract` | No | Token contract address. Omit for native currency (ETH/BTC/SOL) |
| `min_amount` | No | Minimum deposit in smallest unit (e.g. `"1000000"` for 1 USDC). Deposits below this are silently ignored |

**Multi-chain example:**

```toml
[[chains]]
caip2 = "eip155:1"
confirmed_blocks = 12
rpc = ["https://eth.llamarpc.com"]
[[chains.assets]]
symbol = "ETH"
decimals = 18
[[chains.assets]]
symbol = "USDC"
contract = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
decimals = 6
[[chains.assets]]
symbol = "USDT"
contract = "0xdAC17F958D2ee523a2206206994597C13D831ec7"
decimals = 6

[[chains]]
caip2 = "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp"
confirmed_blocks = 32
rpc = ["https://api.mainnet-beta.solana.com"]
[[chains.assets]]
symbol = "SOL"
decimals = 9
[[chains.assets]]
symbol = "USDC"
contract = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
decimals = 6

[[chains]]
caip2 = "bip122:000000000019d6689c085ae165831e93"
confirmed_blocks = 6
rpc = ["http://btc-node:8332"]
[[chains.assets]]
symbol = "BTC"
decimals = 8
```

## Sending Addresses to Watch (Ingress)

Pano starts with zero watched addresses. You feed addresses in through one or more ingress sources. All sources can be active simultaneously — they merge into one watched set.

### HTTP API (recommended for dynamic use)

Enable `[ingress.http]`, then use REST endpoints:

**Add an address** (expand to all configured chains/assets):
```bash
curl -X POST http://localhost:3210/v1/addresses \
  -H 'Content-Type: application/json' \
  -d '{"address":"0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5"}'
```
Response: `201 Created` (empty body)

**Add with specific chain/asset selection:**
```bash
curl -X POST http://localhost:3210/v1/addresses \
  -H 'Content-Type: application/json' \
  -d '{
    "address": "0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5",
    "chains": [
      { "caip2": "eip155:1", "assets": [{ "symbol": "USDC" }, { "symbol": "USDT" }] }
    ]
  }'
```

**Remove an address:**
```bash
curl -X DELETE http://localhost:3210/v1/addresses/0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5
```
Response: `204 No Content`

**Error responses** use the envelope `{"error":"...","message":"..."}`:
- `400` — validation error (invalid address, unknown chain, etc.)
- `401` — missing/wrong API key (if `server.api_key` is set)
- `409` — duplicate (address, chain, symbol) triad already watched
- `404` — address not found on DELETE

### File Ingress (for static/bulk address lists)

Point Pano at a JSON, JSONL, or CSV file. Pano hot-reloads the file on modification.

```toml
[ingress.file]
enabled = true
path = "addresses.jsonl"
poll_interval_secs = 5
authoritative = true   # true = file IS the watched set; false = diff changes only
```

**JSONL format** (one `WatchSpec` per line):
```jsonl
{"address":"0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5"}
{"address":"bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080","chains":[{"caip2":"bip122:000000000019d6689c085ae165831e93"}]}
```

**JSON format** (array):
```json
[
  {"address":"0x9522..."},
  {"address":"0xabcd..."}
]
```

**CSV format** (no header row): `address,chains_json,egress_json`

### Database Ingress (SQLite / PostgreSQL)

Pano polls a table for watch records. Set `poll_interval_secs = 0` for one-shot load.

```toml
[ingress.pg]
enabled = true
url = "postgres://user:pass@localhost:5432/mydb"
poll_interval_secs = 5
```

Required table schema:
```sql
CREATE TABLE watched_addresses (
  address      TEXT NOT NULL,
  caip2        TEXT NOT NULL,
  symbol       TEXT NOT NULL,
  asset_config TEXT,       -- nullable JSON
  chain_config TEXT,       -- nullable JSON
  egress       TEXT,       -- nullable JSON
  PRIMARY KEY (address, caip2, symbol)
);
```

### AMQP Queue Ingress

Consume `WatchSpec` and `Unwatch` messages from an AMQP exchange.

```toml
[ingress.queue]
enabled = true
url = "amqp://localhost"
exchange = "pano.watch"
watch_routing_key = "watch"
unwatch_routing_key = "unwatch"
```

Publish a `WatchSpec` JSON to add, or `{"address":"..."}` to remove.

## Receiving Deposit Events (Egress)

Events are broadcast to all enabled egress targets simultaneously. Choose the one(s) that fit your architecture.

### Webhook (simplest HTTP integration)

Pano POSTs each event as JSON to your endpoint with optional HMAC-SHA256 signing.

```toml
[egress.webhook]
enabled = true
url = "https://your-app.example.com/pano-webhook"
secret = "your-hmac-secret"
max_retries = 3
retry_base_ms = 250
timeout_secs = 30
```

Your endpoint receives:
```http
POST /pano-webhook HTTP/1.1
Content-Type: application/json
X-Pano-Event: pano.deposit.detected
X-Pano-Signature: <hex-hmac-sha256>

{
  "event_id": "01HXYZABC...",
  "event": "pano.deposit.detected",
  "version": 1,
  "occurred_at": "2025-06-01T12:00:00Z",
  "data": {
    "tx_id": "0xabc123...",
    "caip2": "eip155:1",
    "symbol": "USDC",
    "address": "0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5",
    "block_number": 20000000,
    "log_index": 3,
    "amount": "1000000",
    "sender": "0xdef456...",
    "confirmations": 1,
    "timestamp": "2025-06-01T11:59:58Z"
  }
}
```

**Verifying the signature** (HMAC-SHA256 over the raw request body):
```python
import hmac, hashlib
expected = hmac.new(secret.encode(), request_body, hashlib.sha256).hexdigest()
# compare with request.headers["X-Pano-Signature"]
```

### PostgreSQL / SQLite Egress

Events are inserted into a table. Pano auto-creates the table and a dedup index on first use.

```toml
[egress.pg]
enabled = true
url = "postgres://user:pass@localhost:5432/mydb"
```

Schema: columns include `event_id`, `event`, `version`, `occurred_at`, `tx_id`, `caip2`, `symbol`, `address`, `block_number`, `log_index`, `amount`, `sender`, `confirmations`, `timestamp`. Inserts use `ON CONFLICT DO NOTHING` for idempotency.

Query deposits for a user:
```sql
SELECT symbol, amount, block_number, confirmations
FROM deposit_events
WHERE address = '0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5'
ORDER BY event_id;
```

### File Egress

Append events to a file. `.jsonl` (one JSON object per line) is recommended.

```toml
[egress.file]
enabled = true
path = "/var/log/pano/events.jsonl"
```

### AMQP Egress

Publish events to a topic exchange with separate routing keys for detected vs confirmed.

```toml
[egress.queue]
enabled = true
url = "amqp://localhost"
exchange = "pano.deposits"
detected_routing_key = "detected"
confirmed_routing_key = "confirmed"
```

### SSE / WebSocket (real-time streaming)

```toml
[egress.http]
enabled = true
sse = "sse"
websocket = "ws"
```

```bash
curl -N http://localhost:3210/v1/sse       # Server-Sent Events
# or connect to ws://localhost:3210/v1/ws   # WebSocket
```

## Event Schema

All deposit events follow the same schema. There are two event types:

| Event type | When it fires |
|------------|---------------|
| `pano.deposit.detected` | First observation of a transfer |
| `pano.deposit.confirmed` | After `confirmed_blocks` have passed |

Fields in `data`:

| Field | Type | Description |
|-------|------|-------------|
| `tx_id` | string | Transaction hash (hex for EVM/BTC, base58 for Solana) |
| `caip2` | string | Chain identifier, e.g. `"eip155:1"` |
| `symbol` | string | Asset ticker, e.g. `"USDC"` |
| `address` | string | The watched address that received the deposit |
| `block_number` | u64 | Block/slot containing the transaction |
| `log_index` | u64 | Position within the transaction (0 for simple transfers) |
| `amount` | string | Raw amount in smallest unit (wei, satoshi, lamport). **Divide by 10^decimals** for human-readable value |
| `sender` | string | Sender address (empty string if unknown) |
| `confirmations` | u32 | Always 1 for detected events; actual depth for confirmed |
| `timestamp` | string | Block timestamp in ISO 8601 |

## Per-Address Egress Overrides

Route events for specific addresses to different egress targets. Useful for routing high-value addresses to a separate webhook, or for merchant-specific delivery.

First, enable the channels you want to allow in the config:
```toml
[override.egress]
webhook = true
file = false
pg = false
sqlite = false
queue = false
```

Then include an `egress` block when adding the address:
```json
{
  "address": "0x95222290dd7278aa3ddd389cc1e1d165cc4bafe5",
  "chains": [{ "caip2": "eip155:1", "assets": [{ "symbol": "USDC" }] }],
  "egress": {
    "webhook": { "url": "https://hooks.example.com/pano", "secret": "whsec_abc123" }
  }
}
```

Per-address overrides deliver in addition to the global egress broadcast — the event goes everywhere.

## API Authentication

When `server.api_key` is set in the config, all HTTP routes (API, SSE, WebSocket, dashboard) require authentication:

```toml
[server]
api_key = "your-secret-key"
```

Clients must include either:
- `Authorization: Bearer your-secret-key`
- `X-Pano-API-Key: your-secret-key`

Uses constant-time comparison to prevent timing attacks.

## Integration Patterns

### Exchange Deposit Flow

```
1. User requests a deposit address for ETH
2. Your app generates/assigns an address, stores (user_id, address) mapping
3. Your app POSTs the address to Pano via HTTP API
4. Pano scans the chain, detects a deposit
5. Pano POSTs a webhook to your app with the DepositEvent
6. Your app reads event.data.address → looks up user_id → credits their balance
```

### Merchant Checkout Flow

```
1. Customer initiates checkout, selects USDC on Base
2. Your backend generates a unique address for this order
3. POST to Pano with per-address webhook override pointing to your order callback
4. Pano detects the USDC transfer and fires a webhook to your callback URL
5. Your callback marks the order as paid
6. After TTL/cleanup, DELETE the address from Pano
```

### Cold Storage Monitoring

```
1. Add your treasury addresses via file ingress (authoritative mode)
2. Use PostgreSQL egress to persist all events
3. Query the deposit_events table periodically or trigger on new rows
4. Pano handles reorg safety with configurable scan_lookback_blocks
```

## Environment Variables

| Variable | Purpose |
|----------|---------|
| `PANO_CONFIG` | Path to config file (default: `Config.toml`) |
| `RUST_LOG` | Log level: `pano=info`, `pano=debug` |
| Any `${VAR}` in config | Substituted at load — unset vars cause an error |

## Important Notes

- **Amounts are raw integers**: `"1000000"` means 1 USDC (6 decimals), `"1000000000000000000"` means 1 ETH (18 decimals). Always divide by `10^decimals` for display.
- **Address normalization**: EVM addresses are lowercased, bech32 Bitcoin addresses are lowercased, Solana and legacy Bitcoin addresses preserve case. Use the address as returned in events for lookups.
- **Dedup is in-memory**: restarts lose the dedup window. If you restart Pano, you may receive duplicate events. Your downstream consumer should be idempotent (check `event_id` or the `(tx_id, caip2, symbol, address, amount, log_index)` composite key).
- **No historical scanning**: Pano scans forward from the configured `start_block`. It does not backfill deposits that occurred before it started watching.
- **`confirmed_blocks` must be ≥ 1**: detected events fire immediately; confirmed events fire after the configured depth. For instant-only detection, use the detected event and ignore confirmed.
- **File egress JSON format** rewrites the entire array on each event — use JSONL for append-only behavior.
