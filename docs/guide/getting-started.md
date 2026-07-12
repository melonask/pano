# Getting started

Install Pano and copy the example configuration:

```bash
cargo install pano
cp Config.example.toml Config.toml
pano check --config Config.toml
pano --config Config.toml
```

Configure at least one shared chain and asset, select them in `[pano]`, and provide an HTTP(S) JSON-RPC endpoint. `pano check` validates config and feature requirements without starting scanners.

Enable only the integrations you use:

```bash
cargo install pano --features "server,webhook,postgres,amqp"
```

The default build includes SQLite. `full` enables all optional integrations.
