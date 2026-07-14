# Pano

<img align="right" src="https://raw.githubusercontent.com/melonask/pano/refs/heads/main/logo.svg" alt="Pano monitors blockchain addresses across multiple chains (EVM, Bitcoin, Solana) and emits standardised deposit events the moment a transfer is detected" width="200" />

> **Argos Panoptes sees all** — a multi-chain, real-time deposit detector.

Pano is a multi-chain deposit detector for EVM, Bitcoin, and Solana. It turns address watches into normalized detected and confirmed deposit events for files, databases, queues, webhooks, SSE, and WebSockets. Its runtime pipeline is deliberately simple: `ingress → detector → egress`.

[Documentation](https://melonask.github.io/pano/) · [Getting started](https://melonask.github.io/pano/guide/getting-started) · [Configuration](https://melonask.github.io/pano/guide/configuration) · [Repository](https://github.com/melonask/pano)

## At a glance

| Area | What Pano provides |
| --- | --- |
| Chains | EVM native transfers and ERC-20 logs; Bitcoin block outputs; Solana SOL and SPL token balance increases |
| Input | File, HTTP, SQLite, PostgreSQL, and AMQP watch ingress |
| Output | File, SQLite, PostgreSQL, AMQP, webhook, SSE, and WebSocket event egress |
| Lifecycle | A detected event followed by a separately identified confirmed event after the configured threshold |
| Identity | CAIP-2 chains, base-unit amounts, ULID event IDs, and a transaction-local `log_index` |

Pano observes deposits; it is not a wallet, signer, transaction broadcaster, ledger, payment API, identity system, or finality guarantee.

## Requirements, installation, features, and containers

Pano 0.3.0 requires Rust 1.97 or newer to build. Runtime chain access requires HTTP(S) JSON-RPC endpoints. SQLite is enabled by default.

```bash
cargo install pano
# Optional integrations are compile-time features.
cargo install pano --features "server,webhook,postgres,amqp"
# Or build every optional integration.
cargo install pano --features full
```

| Feature | Enables |
| --- | --- |
| `sqlite` (default) | SQLite ingress and egress |
| `server` | Internal HTTP ingress, health endpoint, SSE, WebSocket, dashboard serving |
| `webhook` | Signed webhook egress |
| `postgres` | PostgreSQL ingress and egress |
| `amqp` | AMQP ingress and egress |
| `full` | All of the above |

For a container, bake the required features into the image, mount a configuration file (and writable configured file/database paths), inject secrets as environment variables, then run `pano check` before the long-running `pano` process. `healthcheck` is suitable for container liveness only when the internal server is enabled.

## Quick start

The checked-in example already selects Ethereum and file ingress/egress. Run it from the repository root so its relative `data/pano` paths resolve as intended.

```bash
cp Config.example.toml Config.toml
pano --config Config.toml check
pano --config Config.toml
```

Add one JSON object per line to `data/pano/addresses.jsonl`:

```json
{"address":"0x1111111111111111111111111111111111111111"}
```

Events are appended as JSON lines to `data/pano/deposits.jsonl`. Replace the example RPC endpoint and choose production confirmation and scan settings before deployment.

## CLI reference

```text
pano [--config <PATH>] [COMMAND]

COMMAND:
  check
  healthcheck [--timeout-secs <SECONDS>]
```

| Mode | Action | Success output | Exit behavior |
| --- | --- | --- | --- |
| run (no subcommand) | Loads config and runs ingress, detector, and egress until shutdown | Structured logs | Non-zero on startup/runtime error |
| `check` | Loads and validates configuration, profile references, SQL identifiers, and compiled feature gates; does not start scanners or contact RPC | `configuration is valid` | 0 on valid config; non-zero otherwise |
| `healthcheck` | GETs the local configured `/healthz` endpoint | `healthy` | 0 only for HTTP 204; non-zero for disabled server, timeout, connection error, or another status |

`--config` is a top-level option: place it before the subcommand, for example `pano --config Config.toml check` or `pano --config Config.toml healthcheck --timeout-secs 3`. It defaults to `Config.toml` and is also set by `PANO_CONFIG`. `--timeout-secs` is available only to `healthcheck`, defaults to `3`, and must be greater than zero. Pano loads `.env` when present. `${VAR}` requires an environment value; `${VAR:-default}` supplies a literal default. Configuration, validation, and command errors are reported on stderr and result in a non-zero exit.

## Configuration model

Root profiles are reusable; Pano consumes `[chains]`, `[assets]`, `[paths]`, `[stores]`, `[transports]`, and `[pano]`, and ignores other package namespaces in a merged file. Unknown fields in these Pano-owned structures are rejected.

| Profile | Required/useful fields | Referenced by |
| --- | --- | --- |
| `[chains.<id>]` | `caip2`, `rpc_urls`, `confirmations`; optional `start_block`, `end_block` | `[pano].chains` |
| `[assets.<id>]` | `chain`, `symbol`, `decimals`; optional `enabled`, `contract`, `token_program` | `[pano].assets` |
| `[paths.<id>]` | `kind = "file"`, `path` | file `path_ref` fields |
| `[stores.<id>]` | `driver`, `url` | SQLite/PostgreSQL `store` fields |
| `[transports.amqp.<id>]` | URL, optional credentials/reconnect/QoS | AMQP `transport` |
| `[transports.webhook.<id>]` | URL, timeout, retry controls | webhook `transport` |

`[pano]` selects chain and asset profile IDs. An asset belongs to its selected chain; native assets omit `contract`, while token assets identify a contract/mint. `end_block = 0` means follow the tip. Decimal and `min_amount` values are base-unit integers; the configured maximum decimals defaults to 30.

### Runtime parameters

| Section | Parameters |
| --- | --- |
| `[pano.server]` | `enabled`, `bind`, `port`, `prefix`, `api_key`, `dashboard_path_ref`, `dashboard_export`, `shutdown_timeout_secs` |
| `[pano.detector]` | `dedup_window_size`, `delivery_workers`, `delivery_queue_capacity`, `command_queue_capacity`, stale-event eviction multiplier/minimum, `max_decimals` |
| `[pano.rpc_defaults]` | concurrency, delay, batch size, EVM log batching, lookback, interval/timeouts, native scan cap, retries/backoff, Solana mode/version |
| `[pano.ingress.file]` | `enabled`, `path_ref`, `poll_interval_secs` |
| `[pano.ingress.http]` | `enabled`, `path`, `max_body_bytes` |
| `[pano.ingress.sqlite]`, `[pano.ingress.pg]` | `enabled`, `store`, `table`, `poll_interval_secs` |
| `[pano.ingress.amqp]` | `enabled`, `transport`, `exchange`, `routing_key`, `consumer_tag`, `qos_prefetch` |
| `[pano.egress.file]` | `enabled`, `path_ref` |
| `[pano.egress.sqlite]`, `[pano.egress.pg]` | `enabled`, `store`, `table` |
| `[pano.egress.amqp]` | `enabled`, `transport`, `exchange`, `detected_routing_key`, `confirmed_routing_key` |
| `[pano.egress.webhook]` | `enabled`, `transport`, `secret`, `signature_header` |
| `[pano.egress]` / `[pano.egress.stream]` | persistent queue capacity; stream `enabled`, SSE/WS paths, keepalive/heartbeat, broadcast capacity |

Database table/column identifiers are administrator configuration and must be conservative SQL identifiers; never accept them from a watch request. Persistent sink queues are bounded and apply backpressure. Per-watch override delivery uses a bounded queue and drops that override delivery when full or closed. Stream broadcast is intentionally lossy for lagging clients.

### Detector and RPC behavior

`scan_lookback_blocks` re-scans recent ranges, while the detector owns cursors, recent-event deduplication, and unconfirmed events in memory. `dedup_window_size = 0` disables eviction and can grow memory indefinitely. State is discarded at shutdown; use durable egress and reconciliation where recovery matters. Detected and confirmed lifecycle events are independently deduplicated.

Set confirmations for the chain’s reorganization/finality risk: a detected event is an observation, not settlement. Bound concurrency, request/scan timeout, retries, delay, and batch size to provider capacity. An initial chain with neither `start_block` nor lookback begins at its current tip and does not scan earlier history.

## Watches, ingress, and override gates

All ingress forms normalize to this `WatchSpec`; unknown fields are rejected.

```json
{
  "address": "0x1111111111111111111111111111111111111111",
  "chains": [
    {
      "caip2": "eip155:1",
      "address": "0x1111111111111111111111111111111111111111",
      "start_block": 21000000,
      "end_block": 0,
      "confirmed_blocks": 12,
      "assets": [{"symbol":"USDC","contract":"0x1111111111111111111111111111111111111111","decimals":6,"min_amount":"1000000"}]
    }
  ],
  "egress": {"file":{"path":"data/pano/customer-events.jsonl"}}
}
```

| Field | Rule |
| --- | --- |
| root `address` | Shorthand: expands to compatible configured chains/assets; fallback for entries without an address |
| `chains[]` | `caip2` required; optional address, scan bounds, confirmations, and assets |
| `assets[]` | `symbol` required; optional address, contract, token program, decimals, and positive base-unit `min_amount` |
| `egress` | Optional members: `webhook` (`url`, optional `secret`); `file` (`path`); `pg` (`url`, optional `table.name`); `sqlite` (`path`, optional `table.name`); `queue` (`url`, optional `exchange`, `username`, `password`); `http` (optional `sse`, `websocket`) |
| address cascade | asset address → chain address → root address |
| custom asset | requires both `contract` and `decimals` |
| addresses | EVM and Bech32 Bitcoin are normalized for matching; Base58 Bitcoin and Solana remain case-sensitive |

An explicit `chains` array requires `[pano.overrides.chain].assets = true`; that same setting permits an explicit `assets` array. Per-watch `egress` members require their matching `[pano.overrides.egress]` boolean (`webhook`, `file`, `pg`, `sqlite`, `queue`, or `http`). Keep gates false unless reviewed: a watch must not grant itself new routing or asset authority.

## Internal HTTP API

Requires the `server` feature and `[pano.server].enabled = true`. Routes are nested under `/<prefix>` (default `/v1`); health is always `/healthz`. The configured ingress path defaults to `watch`, so the checked-in example uses `/v1/watch`.

| Request | Success | Contract |
| --- | --- | --- |
| `POST /v1/watch` | `201 Created`, empty body | `Content-Type: application/json`; body is `WatchSpec`; synchronously validates/resolves then queues it |
| `DELETE /v1/watch/{address}` | `204 No Content` | Removes all watched chain/asset triads for normalized address |
| `GET /healthz` | `204 No Content`, empty body | Server accepts work and detector command channel is live; `503` when it is closed |
| `GET /v1/events` | `200` SSE | Available only when stream egress is enabled |
| `GET /v1/ws` | WebSocket upgrade | Available only when stream egress is enabled |

When `api_key` is non-empty, **all** routes including health accept either a matching `Authorization: Bearer <key>` or matching `X-Pano-API-Key: <key>` header; both headers may be present. It is internal-hop defense in depth, not public authentication.

Pano-generated API errors are JSON: `{"error":"<value>","message":"<detail>"}`. Exact application values are `unauthorized` (401), `bad_request` (400), `conflict` (409), `not_found` (404), `method_not_allowed` (405), and `unavailable` (503). Unknown routes produce `not_found`. Framework request rejections also occur: malformed JSON is 400, unknown WatchSpec fields are 422, and missing JSON content type is 415. HTTP mutation is 405 while authoritative file ingress is enabled; a duplicate resolved triad is 409; a missing watched address on DELETE is 404. Correct 4xx input/configuration/authentication problems; retry transport failures and 5xx with bounded backoff.

## Event contract and delivery

Every delivery adapter serializes this exact normalized shape; `internal_egress` is routing-only and never serialized.

```json
{
  "event_id":"01J2V8Q8YQW18Y0AM3QFQZ76A7",
  "event":"pano.deposit.confirmed",
  "version":1,
  "occurred_at":"2026-07-12T12:00:00Z",
  "data":{"tx_id":"0xabc123","caip2":"eip155:1","symbol":"USDC","address":"0x1111111111111111111111111111111111111111","block_number":21000000,"log_index":4,"amount":"1000000","sender":"0x2222222222222222222222222222222222222222","confirmations":12,"timestamp":"2026-07-12T11:59:42Z"}
}
```

`event` is exactly `pano.deposit.detected` or `pano.deposit.confirmed`; each has a newly generated ULID `event_id`. `amount` is a non-empty, positive ASCII digit string without a decimal point or leading zero. `tx_id` is hex for EVM/Bitcoin and base58 for Solana. `log_index` is EVM log index, Bitcoin `vout`, or Solana account index. `sender` may be empty when unavailable.

File/database/AMQP/webhook adapters receive the envelope. AMQP uses configured detected/confirmed routing keys. Webhooks POST JSON, set `X-Pano-Event`, and, when secret is non-empty, send a lowercase hex HMAC-SHA256 of the JSON body in the configured signature header; retries cover transport errors, 429, and 5xx. SSE sends event name `pano.deposit.*` with JSON data; WebSocket sends JSON text. Both send `pano.stream.lag` with `{"missed":n}` to lagging consumers and offer no replay. Persist and deduplicate `event_id` before external effects; reconcile from durable sinks.

## Public Rust API

The crate exposes `pano::run(AppConfig)`, configuration types in `pano::config`, models such as `WatchSpec`, `DepositEvent`, `DepositData`, and `ResolvedWatch` in `pano::model`, and detector/chain/ingress/egress modules for embedding. The binary remains the supported operational boundary.

## Scanner semantics and caveats

| Chain | Scanner behavior and constraints |
| --- | --- |
| EVM | Scans native transaction recipients and ERC-20 `Transfer` logs. Removed logs, malformed data, and zero amounts are skipped. Native scanning is block-by-block and capped by `max_native_scan_per_cycle`; token logs use `batch_size`, with per-contract fallback if address-array log filtering fails. |
| Bitcoin | Scans blocks and outputs, emits native amounts in satoshis, and uses `vout` as `log_index`. The sender is the first input address when RPC data provides it. |
| Solana | Detects confirmed SOL/SPL increases. `blocks` mode calls `getBlock` per slot and caps a cycle to `batch_size`; `signatures` mode pages `getSignaturesForAddress` (maximum page limit 1000) and fetches transactions. A pruned signature cursor triggers a block-style rescan. Failed transactions, unavailable blocks, and skipped slots produce no event. Omitting a token program derives/scans classic and Token-2022 associated token accounts; specifying one limits scanning to it. Sender inference is best effort. |

RPC gaps, provider pruning, reorgs, process restarts, bounded dedup eviction, and lagging streams mean Pano does not promise exactly-once or complete historical settlement. Use an adequate lookback/start range, durable egress, idempotent consumers, and chain-specific reconciliation.

## Deployment, security, and troubleshooting

- Do not expose the internal listener publicly. Put a gateway with authentication, authorization, rate limiting, and payment policy in front of public traffic.
- Keep RPC URLs, database/broker credentials, webhook secrets, and API keys in secret-managed environment variables; do not put them in watches or logs.
- Stop ingress producers first during shutdown and allow `shutdown_timeout_secs`; task timeout can abort remaining background work.
- Alert on RPC/scan failures, queue pressure, egress failures, stream lag, and durable-store errors. A 204 health result does not prove RPC or downstream health.
- If no events arrive, run `pano --config Config.toml check`, verify selected CAIP-2/asset/address compatibility and scan bounds, then inspect RPC reachability and durable sink writes. Do not increase retries or buffers without finding the bottleneck.

## Development and tests

```bash
cargo test
cargo test --features full
```

Integration coverage exercises configuration, scanners, ingress, egress, delivery, and HTTP behavior. End-to-end scenarios require their local chain tooling (Anvil, Solana test validator, Bitcoin Core regtest) and Docker/Compose prerequisites where applicable. Test native and token deposits, confirmation transitions, watch add/remove, durable egress, and graceful shutdown.

## License

MIT OR Apache-2.0. See [LICENSE](LICENSE).
