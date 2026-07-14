# Configuration

Pano uses universal root profiles plus a `[pano]` namespace. Root profiles can be shared with other services; Pano ignores other package namespaces.

```toml
[chains.ethereum]
family = "evm"
caip2 = "eip155:1"
rpc_urls = ["${ETH_RPC_URL}"]
confirmations = 12
# Optional bounded scan range; environment values are unquoted TOML integers.
start_block = ${PANO_ETH_START_BLOCK}
end_block = ${PANO_ETH_END_BLOCK}

[assets.usdc]
chain = "ethereum"
symbol = "USDC"
contract = "0xA0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
decimals = 6
# Optional for Solana token assets. Omit to scan both classic and Token-2022.
token_program = "${PANO_USDC_TOKEN_PROGRAM}"

[paths.watches]
kind = "file"
path = "data/watches.jsonl"

[paths.events]
kind = "file"
path = "data/events.jsonl"

[pano]
chains = ["ethereum"]
assets = ["usdc"]

[pano.ingress.file]
enabled = true
path_ref = "watches"

[pano.egress.file]
enabled = true
path_ref = "events"
```

Ingress adapters turn watches into detector commands: file, HTTP, SQLite, PostgreSQL, and AMQP. Egress adapters receive `DepositEvent` values: file, SQLite, PostgreSQL, AMQP, webhook, SSE, and WebSocket. An enabled adapter requires its Cargo feature.

Database table and column names are administrative configuration. They are restricted to SQL identifiers before Pano constructs dynamic SQL; do not derive them from request data.

For Solana, `rpc_defaults.batch_size` is the `getSignaturesForAddress` page
limit in `signatures` mode; the detector scans through the requested confirmed
tip in that mode. In `blocks` mode, it instead caps each detector cycle to that
many slots because it issues one `getBlock` request per slot.
