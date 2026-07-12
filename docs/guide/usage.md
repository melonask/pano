# Usage

Run the detector after checking configuration:

```bash
pano check --config Config.toml
pano --config Config.toml
```

With `server` enabled, submit a watch to the configured ingress path (the default is `/v1/watch`):

```bash
curl -X POST http://127.0.0.1:3210/v1/watch \
  -H 'content-type: application/json' \
  -d '{"address":"0x1111111111111111111111111111111111111111"}'
```

The detector expands a compatible address across configured chains and assets. To provide chain or asset overrides, enable the corresponding `[pano.overrides]` gate first. Duplicate detected and confirmed events are independently deduplicated; confirmed events are emitted after the watch's required confirmation count.

Use `GET /healthz` or `pano healthcheck --config Config.toml` for liveness. Healthcheck requires the internal server to be enabled and returns success only for HTTP `204 No Content`.
