# Pano

Pano is a multi-chain deposit detector for EVM, Bitcoin, and Solana. It turns address watches into normalized detected and confirmed deposit events for files, databases, queues, webhooks, SSE, and WebSockets.

The runtime pipeline is deliberately simple: `ingress → detector → egress`.

## Start

```bash
cargo install pano
cp Config.example.toml Config.toml
pano check --config Config.toml
pano --config Config.toml
```

SQLite is enabled by default. Add integrations at build time:

```bash
cargo install pano --features "server,webhook,postgres,amqp"
```

`pano check` validates configuration and enabled feature gates without starting scanners. When the internal server is enabled, use `pano healthcheck --config Config.toml` for container and orchestrator health checks.

## Documentation

Read the full guide and API reference at [melonask.github.io/pano](https://melonask.github.io/pano/). The repository is [github.com/melonask/pano](https://github.com/melonask/pano).

## License

MIT OR Apache-2.0. See [LICENSE](LICENSE).
