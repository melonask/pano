# Operations

## CLI

```bash
pano --config Config.toml check
pano --config Config.toml healthcheck --timeout-secs 3
pano --config Config.toml
```

`--config` works before or after a subcommand; examples use the preferred top-level-first form. `check` validates universal references, package configuration, SQL identifier constraints, and enabled feature gates. `healthcheck` requests the configured internal `/healthz` endpoint; it is appropriate for container health checks.

## Internal HTTP API

When `[pano.server].enabled = true`, routes are served below `/<prefix>` (`/v1` by default). HTTP ingress is internal integration traffic, not a public payment API. Put it behind the owning gateway and use `api_key` only as defense in depth.

| Operation | Purpose |
| --- | --- |
| `POST /v1/watch` | Add or replace resolved watches for a request |
| `DELETE /v1/watch/{address}` | Remove watches for an address |
| `GET /healthz` | Return `204` while the server and detector command loop are live |
| `GET /v1/events` | SSE stream when stream egress is enabled |
| `GET /v1/ws` | WebSocket stream when stream egress is enabled |

For retries, preserve the same watch request only when the caller can tolerate replacement semantics. Consumers must deduplicate by `event_id`, persist events before side effects, and treat detected and confirmed as separate lifecycle events. Bound retries for RPC, webhooks, and downstream writes; alert instead of retrying permanent configuration or validation failures.
