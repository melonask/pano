---
name: pano
description: Use when configuring, operating, integrating, troubleshooting, testing, or deploying Pano’s EVM, Bitcoin, or Solana deposit detector, including watches, scanners, normalized deposit events, ingress/egress adapters, RPC behavior, and its internal HTTP API.
---

# Pano operations and integration guide

## Purpose and boundaries

Pano converts watches into normalized deposit lifecycle events through `ingress → detector → egress`. It observes EVM, Bitcoin, and Solana deposits; it is not a public payment API, wallet/key manager, signer, broadcaster, balance ledger, identity system, or finality guarantee. Keep public authentication, authorization, rate limits, customer policy, and payment effects in the owning gateway/application.

## Command selection

| Need | Command and use |
| --- | --- |
| Validate config/features | `pano --config Config.toml check`; validates locally and does not contact RPC. Run after every config, secret, or feature change. |
| Start service | `pano --config Config.toml`; start only after validation succeeds. It starts enabled ingress, detector, egress, and optionally the internal server. |
| Check liveness | `pano --config Config.toml healthcheck --timeout-secs 3`; use only against a running instance with `pano.server.enabled = true`. It is not end-to-end readiness. |

## Prerequisites and features

- Build with Rust 1.97+; configure HTTP(S) JSON-RPC URLs for every selected chain.
- SQLite is default. `server`, `webhook`, `postgres`, and `amqp` must be compiled before their integrations are enabled; `full` enables all optional integrations.
- Select root `[chains.<id>]` and `[assets.<id>]` through `[pano].chains` and `[pano].assets`. Chains need `caip2`, `rpc_urls`, and positive confirmations; assets need chain, symbol, and decimals; token assets also need contract/mint identity.
- `${VAR}` is mandatory environment substitution; `${VAR:-default}` is optional substitution. Never place credentials/secrets in watches.

## Safe workflow

```bash
cp Config.example.toml Config.toml
pano --config Config.toml check
pano --config Config.toml
```

Use the checked-in file paths only from the intended working directory, replace its RPC endpoint for deployment, and verify writes to the configured durable sink. Run check again after enabling an adapter, changing a profile reference, or changing features. Persist received events and deduplicate by `event_id` before business side effects.

## Exact command reference

```text
pano [--config <PATH>] [COMMAND]

COMMAND:
  check
  healthcheck [--timeout-secs <SECONDS>]
```

| Command | Defaults and output | Result |
| --- | --- | --- |
| `pano` | `--config Config.toml`; `PANO_CONFIG` also supplies it | Runs until shutdown; errors exit non-zero |
| `pano check` | Prints `configuration is valid` | Validates config/profile/feature/SQL identifier constraints without scanners/RPC; 0 on success |
| `pano healthcheck` | `--timeout-secs 3`; prints `healthy` | GETs local `/healthz`, adding configured API key; succeeds only on 204; timeout must be >0 |

`--config` works before or after a subcommand; prefer top-level-first: `pano --config Config.toml check`. `pano check --config Config.toml` is also accepted.

## Configuration and override gates

Keep shared profiles at root: `[chains]`, `[assets]`, `[paths]`, `[stores]`, and `[transports]`; Pano behavior is under `[pano]`. `path_ref`, `store`, and `transport` must resolve. SQL table/column identifiers are administrator-only input and are restricted before dynamic SQL; never derive them from request data.

`[pano.rpc_defaults]` controls concurrency, delay, batch size, EVM log batching, lookback, interval, request/scan timeouts, native cap, retry count/base delay, and Solana mode/version. Bound all values to provider capacity. Detector command and persistent egress queues are bounded and apply backpressure. Per-watch override delivery has a bounded queue but drops that override delivery when full or closed. `dedup_window_size = 0` disables eviction and can grow memory without bound.

An explicit `WatchSpec.chains` requires `[pano.overrides.chain].assets = true`; that same setting permits an `assets` array. Each egress override needs its exact matching gate: `webhook`, `file`, `pg`, `sqlite`, `queue`, or `http`. Rejected watches must not change active watches. Do not enable a gate merely to pass unreviewed input.

## WatchSpec schema and ingress

```json
{"address":"0x1111111111111111111111111111111111111111"}
```

| Field | Contract |
| --- | --- |
| `address` | Optional root shorthand, expanded across compatible configured chains/assets; fallback for nested entries |
| `chains[]` | Required `caip2`; optional `address`, `start_block`, `end_block` (`0` follows tip), `confirmed_blocks`, `assets` |
| `assets[]` | Required `symbol`; optional address, contract, token program, decimals, positive base-unit `min_amount` |
| `egress` | Optional, gated members: `webhook` (`url`, optional `secret`); `file` (`path`); `pg` (`url`, optional `table.name`); `sqlite` (`path`, optional `table.name`); `queue` (`url`, optional `exchange`, `username`, `password`); `http` (optional `sse`, `websocket`) |

Unknown fields are rejected. Address resolution is asset → chain → root. EVM and Bech32 Bitcoin addresses normalize for matching; Base58 Bitcoin and Solana retain case. EVM addresses must be `0x` plus 40 hex characters; Solana is validated as base58; Bitcoin supports accepted Base58 and Bech32 forms. Custom assets require both contract and decimals. Amount thresholds and emitted amounts are positive digit strings in smallest units: no decimal point, zero, or leading zeros.

Ingress may be file, internal HTTP, SQLite, PostgreSQL, or AMQP. Their required path/store/transport profile and build feature must be available. Authoritative file ingress prevents HTTP watch mutations.

## Detector chain semantics

- **EVM:** native recipient transfers and ERC-20 `Transfer` logs; removed logs, malformed entries, and zero values are skipped. Native scans are block-by-block and capped; ERC-20 log batches can fall back to one contract at a time.
- **Bitcoin:** scans block outputs for watched addresses, reports satoshis, uses `vout` as `log_index`, and uses the first input address as sender when available.
- **Solana:** detects confirmed SOL/SPL balance increases. `blocks` mode makes one `getBlock` per slot and limits each cycle to `batch_size`; `signatures` mode pages signatures (page limit no greater than 1000) and fetches transactions. Failed transactions, missing blocks, and skipped slots emit nothing. A pruned signature cursor falls back to a block-style rescan. Omitted token program scans classic and Token-2022 associated token accounts; an explicit program limits it.

`scan_lookback_blocks` deliberately re-scans recent ranges. Scan cursor, unconfirmed deposits, and dedup state are in-memory and discarded on shutdown. With no start block and zero lookback, first scan starts at the current tip. Configure start/lookback and reconciliation for provider gaps, reorgs, restart loss, and pruning.

## Events, egress, and HTTP contracts

All adapters receive `DepositEvent`:

```json
{"event_id":"01J2V8Q8YQW18Y0AM3QFQZ76A7","event":"pano.deposit.detected","version":1,"occurred_at":"2026-07-12T12:00:00Z","data":{"tx_id":"0xabc123","caip2":"eip155:1","symbol":"ETH","address":"0x1111111111111111111111111111111111111111","block_number":21000000,"log_index":0,"amount":"1","sender":"0x2222222222222222222222222222222222222222","confirmations":1,"timestamp":"2026-07-12T11:59:42Z"}}
```

`event` is exactly `pano.deposit.detected` or `pano.deposit.confirmed`; they have distinct ULIDs and are independently deduplicated. `tx_id` is EVM/Bitcoin hex or Solana base58. `log_index` is EVM log index, Bitcoin output index, or Solana account index. Sender may be empty. `internal_egress` is never serialized.

File, SQLite, PostgreSQL, AMQP, and webhook are persistent/external egress paths. AMQP uses configured detected/confirmed keys. Webhooks POST JSON, send `X-Pano-Event`, and sign the exact JSON body with lowercase-hex HMAC-SHA256 in the configured header when secret is non-empty; retry network failures, 429, and 5xx with configured bounded exponential backoff.

SSE and WebSocket require `server` plus stream egress. They are broadcast streams, not durable queues: lag generates `pano.stream.lag` with `{"missed":n}` and no replay. SSE uses the deposit event type as its SSE event name. Reconcile loss-intolerant consumers from durable egress.

With server enabled, routes are `POST /<prefix>/<ingress.path>`, `DELETE` on that path plus `/{address}`, `GET /healthz`, and enabled SSE/WS routes. The example defaults are `/v1/watch`, `/v1/events`, and `/v1/ws`. POST returns 201; successful DELETE returns 204. Health is 204 only while server and detector command loop are live, otherwise 503. A non-empty `api_key` protects **every** route and accepts either matching `Authorization: Bearer <key>` or matching `X-Pano-API-Key: <key>`; both may be present.

## Errors and recovery

Pano application error JSON is `{"error":"…","message":"…"}`. Exact values/statuses: `unauthorized` 401, `bad_request` 400, `conflict` 409, `not_found` 404, `method_not_allowed` 405, and `unavailable` 503. Unknown route is `not_found`. Framework rejections are malformed JSON 400, unknown WatchSpec fields 422, and absent JSON content type 415. Duplicate watch triads are 409; unknown DELETE addresses are 404; authoritative file ingress rejects HTTP mutations with 405.

Correct 4xx request, authorization, configuration, or feature errors before retrying. Retry only transient transport/5xx failures with bounded backoff. For missing events: check config, selected chain/asset/address, range/confirmation settings, RPC reachability, queue pressure, and durable sink writes; do not compensate by unbounded queues or retries.

## Reliability, security, and prohibited actions

- Treat detected events as observations, not settlement; confirmations reduce but do not erase chain risk.
- Persist then apply effects; event IDs make consumers idempotent, not exactly-once delivery.
- Do not publish the internal server. Use a protected gateway; API key is defense in depth only.
- Keep RPC credentials, broker/database passwords, webhook secrets, and API keys in secret management. Restrict file and database permissions.
- Stop ingress first and allow `shutdown_timeout_secs`; remaining tasks can be aborted after timeout.
- Do not claim stream replay, scanner persistence, complete historical coverage, or exactly-once guarantees. Do not invent flags, routes, config fields, or adapter behavior.

## Verification checklist

- [ ] Required features match every enabled integration; `pano --config Config.toml check` prints `configuration is valid`.
- [ ] Root profile references, CAIP-2 IDs, RPC URLs, scan bounds, confirmations, and asset decimals/minimums are correct.
- [ ] Watch schema and override gates are reviewed; no secrets or SQL identifiers originate from requests.
- [ ] Durable egress exists where loss matters; consumers persist and deduplicate both lifecycle events by `event_id`.
- [ ] Internal server is private; authentication is tested; `/healthz` returns 204 only after startup.
- [ ] RPC/provider limits, bounded queues/retries, stream lag, scanner errors, and durable delivery failures are monitored.
- [ ] Formatting passes: `cargo fmt --all -- --check`.
- [ ] Linting passes: `cargo clippy --all-targets --all-features --locked -- -D warnings`.
- [ ] Tests pass: `cargo test --all-targets --all-features --locked`.
