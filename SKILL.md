---
name: pano
description: Use when configuring, operating, integrating, troubleshooting, testing, or deploying Pano’s EVM, Bitcoin, or Solana deposit-detection service, including watches, scanners, normalized deposit events, ingress/egress adapters, RPC behavior, and its internal HTTP API.
---

# Pano operations and integration guide

## Purpose and boundary

Pano detects deposits on configured EVM, Bitcoin, and Solana chains. It accepts
address watches through ingress adapters, resolves and scans those watches in
the detector, then publishes normalized lifecycle events through egress
adapters:

`ingress → detector → egress`

Use Pano for deposit observation and delivery only. It is not a public payment
API, wallet/key manager, transaction broadcaster, balance ledger, finality
guarantee, identity system, or gateway. Keep public authentication,
authorization, rate limiting, payment policy, and customer-facing APIs in the
owning gateway/application. Do not collapse or bypass the pipeline boundary.

## Build and configuration prerequisites

- Configure at least one shared chain and asset, select both in `[pano]`, and
  provide an HTTP(S) JSON-RPC endpoint for every selected chain.
- SQLite is available in the default build. Build with the matching Cargo
  features before enabling `server`, `webhook`, `postgres`, or `amqp`
  integrations; `full` enables all optional integrations.
- Use `${VAR}` for required environment values and `${VAR:-default}` only when
  an explicit default is safe. An unset required variable is a configuration
  load failure.
- Keep shared definitions at the root: `[chains.<id>]`, `[assets.<id>]`,
  `[paths.<id>]`, `[stores.<id>]`, and `[transports.<kind>.<id>]`. Put Pano
  selection and behavior under `[pano]`. In a merged configuration, Pano
  ignores other package namespaces.
- A chain profile supplies CAIP-2 identity, RPC URLs, and confirmations; an
  asset profile supplies chain, symbol, decimals, and, for tokens, contract
  identity. Select profile IDs with `[pano].chains` and `[pano].assets`.
- Reference paths with `path_ref`, database profiles with `store`, and AMQP or
  webhook connection profiles with `transport`. Package-local transport values
  take precedence over the referenced transport profile.
- Treat configured database table and column identifiers as administrator
  input. Never derive SQL identifiers from a watch request.

## Check, run, and healthcheck

After every configuration or feature change, run:

```bash
pano check --config Config.toml
pano --config Config.toml
```

`check` validates root-profile references, Pano configuration, SQL identifier
constraints, and enabled feature gates without starting scanners or contacting
chain RPCs. Start only after a successful check.

When `[pano.server].enabled = true`, use the running instance’s internal
health endpoint for liveness:

```bash
pano healthcheck --config Config.toml --timeout-secs 3
```

It requests `/healthz` and succeeds only on `204 No Content`; it requires the
internal server. Do not use a healthcheck as evidence that every RPC provider,
scanner, or downstream consumer is healthy.

## Pano configuration and detector behavior

`[pano.rpc_defaults]` applies scanner controls across chains: concurrency,
delay, batch size, scan lookback and interval, scan/request timeouts, retry
count and base delay, native-scan limit, EVM log-address batching, and Solana
scan mode/version support. Size those values to provider limits and expected
block or slot throughput; do not remove bounds to compensate for overload.

The detector’s command and per-watch delivery queues are bounded. Tune
`command_queue_capacity`, `delivery_queue_capacity`, and `delivery_workers`
for measured load, preserving downstream capacity. Queue saturation is
backpressure: slow the producer, increase capacity only with memory headroom,
or add durable egress. It is not permission to drop or duplicate business
effects.

`dedup_window_size` retains recent event keys in memory. Detected and confirmed
events are independently deduplicated because they are distinct lifecycle
events. A zero window disables eviction and can grow memory without bound.
Unconfirmed events are retained until their configured confirmation threshold
is reached; stale-event eviction is controlled by the multiplier and minimum
block distance. Set confirmation counts for the chain’s finality risk; a
detected event is observation, not final settlement.

### Scanner semantics by chain

- **EVM:** detects native transfers and ERC-20 `Transfer` logs for watched
  recipients. Removed logs and malformed/zero-value transfers are skipped.
  Log index distinguishes transfers in one transaction.
- **Bitcoin:** scans blocks and outputs for watched addresses, emitting the
  native asset in satoshis. `log_index` is the output (`vout`) index. Sender is
  taken from the first input when available.
- **Solana:** detects native SOL and SPL-token activity for watched targets.
  `solana_scan_mode = "blocks"` issues one `getBlock` per slot and is capped
  to `batch_size` slots per cycle; `"signatures"` follows address signatures,
  uses `batch_size` only as its page limit, and can fall back to a block rescan
  when a pruned cursor is encountered. Failed transactions, unavailable blocks,
  and skipped slots do not become deposit events. The account index distinguishes
  activity within a transaction. A token program may be specified for an SPL asset.

All emitted amounts are positive, base-unit digit strings with no decimal
point. Transaction IDs are hex for EVM/Bitcoin and base58 for Solana. Preserve
the event’s CAIP-2, block/slot number, timestamp, sender when available, and
network-level index when reconciling deposits.

## Watches and ingress

Ingress adapters normalize all input into the same `WatchSpec` and detector
command path. Supported modes are file polling, internal HTTP, SQLite polling,
PostgreSQL polling, and AMQP consumption. Enable only the adapters required;
their referenced path/store/transport profiles and build features must exist.

File ingress uses `[pano.ingress.file]` with `path_ref` and
`poll_interval_secs`. SQLite and PostgreSQL ingress use `store`, `table`, and
`poll_interval_secs`. AMQP ingress uses `transport`, `exchange`,
`routing_key`, `consumer_tag`, and `qos_prefetch`. HTTP ingress requires the
`server` feature and uses its configured `path` and `max_body_bytes`.

### WatchSpec schema and resolution

A shorthand watch is an address:

```json
{"address":"0x1111111111111111111111111111111111111111"}
```

It expands across compatible configured chains and their selected assets. An
explicit watch can contain `address`, `chains`, and `egress`. Each `chains`
entry has required `caip2` and optional `address`, `start_block`, `end_block`,
`confirmed_blocks`, and `assets`. Each asset entry has `symbol` and optional
`address`, `contract`, `token_program`, `decimals`, and `min_amount`.

Address resolution cascades asset address → chain address → root address.
Addresses are validated for the target chain; EVM and Bech32 Bitcoin forms are
normalized for matching, while case-sensitive Bitcoin Base58 and Solana
addresses retain case. A custom asset requires both `contract` and `decimals`.
`min_amount` is in smallest units. Unknown schema fields and malformed watches
must be rejected rather than silently ignored.

## Override gates

Watch input cannot grant itself additional authority. Explicit chain/asset
overrides require the relevant `[pano.overrides.chain]` permission; per-watch
egress overrides require the matching `[pano.overrides.egress]` gate
(`webhook`, `file`, `pg`, `sqlite`, `queue`, or `http`). Keep every gate false
unless a reviewed integration requires it. A rejected watch must not mutate
the active watch set.

## Egress and event contract

Egress adapters are file, SQLite, PostgreSQL, AMQP, webhook, and the internal
SSE/WebSocket stream. Use file or database egress when durable reconciliation
is required. AMQP uses the configured exchange and distinct detected/confirmed
routing keys. Webhook egress requires the `webhook` feature and uses its
transport profile, secret, and configured signature header. Stream egress
requires both the `server` feature and an enabled internal server; its bounded
broadcast buffer may report consumer lag and is not durable storage.

Every adapter receives the same serialized envelope. `internal_egress` routing
metadata is never serialized. Example:

```json
{
  "event_id":"01J2V8Q8YQW18Y0AM3QFQZ76A7",
  "event":"pano.deposit.confirmed",
  "version":1,
  "occurred_at":"2026-07-12T12:00:00Z",
  "data":{
    "tx_id":"0xabc123",
    "caip2":"eip155:1",
    "symbol":"USDC",
    "address":"0x1111111111111111111111111111111111111111",
    "block_number":21000000,
    "log_index":4,
    "amount":"1000000",
    "sender":"0x2222222222222222222222222222222222222222",
    "confirmations":12,
    "timestamp":"2026-07-12T11:59:42Z"
  }
}
```

`pano.deposit.detected` and `pano.deposit.confirmed` are separate events with
separate IDs. Persist and deduplicate by `event_id` before external side
effects. Consumers that require recovery after outages or stream lag must
reconcile from durable egress, not assume replay from the broadcast stream.

## Internal HTTP API and errors

With `[pano.server].enabled = true`, routes are served below
`/<prefix>` (`/v1` by default):

- `POST /v1/watch` adds or replaces the resolved watches for that request.
- `DELETE /v1/watch/{address}` removes watches for that address.
- `GET /healthz` reports `204` only while the server and detector command loop
  are live.
- `GET /v1/events` and `GET /v1/ws` provide SSE and WebSocket streams when
  stream egress is enabled.

The server can require either `Authorization: Bearer <key>` or
`X-Pano-API-Key: <key>` when `api_key` is non-empty. This is internal-hop
defense in depth, not a substitute for a public gateway. Invalid JSON or an
invalid watch is a client error; unknown watch fields are rejected. Disabled
routes and unavailable services are server errors. Error bodies use the form:

```json
{"error":"invalid_request","message":"address is required when chains is empty"}
```

Retry transport failures and `5xx` only with bounded exponential backoff.
Correct request data, configuration, feature selection, or authorization before
retrying client errors.

## Reliability, security, and deployment

- Bound retries for RPC, webhooks, and downstream transport writes using the
  configured retry count and base delay. Do not retry permanent validation,
  configuration, disabled-feature, or authentication failures.
- Set RPC concurrency, batching, delay, timeout, and scan interval below
  provider limits. Investigate persistent RPC errors, missed ranges, or queue
  pressure instead of increasing retries indefinitely.
- Persist events before side effects and use a durable egress/database for
  recovery and reconciliation. In-memory deduplication and broadcast streams
  alone do not provide durable exactly-once delivery.
- Keep RPC credentials, AMQP passwords, webhook secrets, and internal API keys
  in environment-backed secret management; never place them in watch payloads,
  logs, dashboards, or public clients. Limit file permissions and database/
  broker credentials to the minimum required scope.
- Do not publish the internal listener. Bind and firewall it for trusted
  internal callers, place the owning gateway in front of any public surface,
  and expose dashboard exports only on an approved internal path.
- On deployment, run `pano check`, verify feature/config alignment and secret
  injection, start Pano, then verify `/healthz`. Alert on scanner/RPC failures,
  queue saturation, egress failures, stream lag, and durable-store errors.
- For shutdown, stop ingress producers first, allow the configured
  `shutdown_timeout_secs` for background work, then stop the process. Preserve
  durable egress and scan state needed for reconciliation; do not treat a
  process stop as delivery completion.

## Troubleshooting and verification

1. Run `pano check --config Config.toml`; resolve every profile reference,
   identifier, feature-gate, and environment substitution error first.
2. Confirm selected chains/assets, CAIP-2 values, RPC reachability, start/end
   boundaries, confirmation counts, and address/asset compatibility.
3. Confirm the ingress adapter is enabled, its file/store/transport profile is
   correct, and the submitted watch conforms to `WatchSpec` and override gates.
4. Check detector queue pressure and RPC timeout/retry behavior. Reduce scan
   work or provision capacity rather than accepting unbounded buffering.
5. Verify durable egress writes and downstream idempotency by `event_id`; use
   the durable sink to reconcile detected and confirmed lifecycle events.
6. For streams, verify stream egress plus the server feature/server setting;
   investigate lag with durable storage, not a presumed replay.

Run focused tests for changed behavior and the complete suite before release:

```bash
cargo test
```

For end-to-end coverage, use the repository e2e stack only with its required
Docker/Compose environment, local Anvil, Solana test validator, Bitcoin Core
regtest, and command-line prerequisites. Exercise native and token deposits,
watch creation/removal, confirmation events, durable egress, and shutdown;
tear down containers and separately stop local chain processes afterward.

## Prohibited actions

- Do not use Pano as a public gateway or expose its internal server directly.
- Do not put secrets, RPC URLs with credentials, or SQL identifiers in watch
  requests or event consumers’ logs.
- Do not enable a config section whose build feature is absent.
- Do not enable override gates merely to make a request pass.
- Do not treat detected events as confirmed, stream delivery as durable, or
  in-memory deduplication as an exactly-once guarantee.
- Do not retry malformed input, invalid configuration, disabled features, or
  permanent downstream failures without correcting the cause.
- Do not invent configuration keys, API routes, adapter behavior, or recovery
  guarantees; validate changes with `pano check` and tests.

## Final checklist

- [ ] Root chains, assets, paths, stores, and transports resolve; `[pano]`
      selects the intended chain and asset profiles.
- [ ] Enabled adapters match installed features and approved ingress/egress
      profiles; SQL identifiers are administrator-configured.
- [ ] RPC, confirmation, scan, timeout, retry, queue, and dedup settings are
      sized for the deployment.
- [ ] Watch schema, address/asset validation, and override gates are correct.
- [ ] Consumers persist and deduplicate `event_id`; durable reconciliation is
      available where loss is unacceptable.
- [ ] Internal server is protected, secrets are injected safely, and the public
      gateway owns public security controls.
- [ ] `pano check` passes, the service starts, `/healthz` returns `204` when
      enabled, and relevant tests/e2e scenarios pass.
