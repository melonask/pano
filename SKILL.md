---
name: pano
description: Operate Pano to detect EVM, Bitcoin, and Solana deposits through its ingress-to-detector-to-egress pipeline.
---

# Pano AI operations guide

Pano accepts watches through ingress adapters, scans configured chains in the detector, and emits detected and confirmed events through egress adapters. Preserve that `ingress → detector → egress` boundary when operating or integrating it.

## Validate before starting

```bash
pano check --config Config.toml
pano --config Config.toml
pano healthcheck --config Config.toml --timeout-secs 3
```

Run `check` after every configuration or feature change. It validates shared profile references, Pano namespace fields, database identifier constraints, and feature gates without contacting chain RPCs. `healthcheck` probes the live internal `/healthz` endpoint and requires the internal server to be enabled.

## Feature and configuration guardrails

- Default builds include SQLite only; enable `server`, `webhook`, `postgres`, or `amqp` before enabling their config sections.
- Keep chains and assets in shared root profiles, then select them under `[pano]`.
- Treat database table and column names as administrator-controlled configuration; never source them from requests.
- Keep the HTTP listener internal. Put public authentication, authorization, and rate limiting at the owning gateway.
- Enable watch or egress overrides only when the required `[pano.overrides]` gate is explicitly configured.

## Event semantics and retries

- `pano.deposit.detected` means a transfer was observed; `pano.deposit.confirmed` is a separate event emitted after the configured confirmation threshold.
- Process events idempotently by `event_id` and persist before external side effects.
- Retry bounded transient RPC, webhook, and downstream transport failures with backoff. Do not retry malformed watches, disabled features, invalid configuration, or permanent HTTP failures without correction.
- A broadcast stream can report consumer lag; reconcile from durable egress storage when event loss is unacceptable.
