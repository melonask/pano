# Pano e2e — End-to-end integration stack

Minimal Docker Compose stack that runs Pano (multi-chain deposit detector) with
Ladon (address-pool daemon) and a tiny Python API against local blockchain nodes.

## Tables

Only 3 app-level tables:

| Table | Schema | Owner | Purpose |
|-------|--------|-------|---------|
| `address_pool` | public | ladon | HD-derived addresses, app marks rows `is_used` |
| `watched_addresses` | public | app | App inserts; Pano polls via ingress |
| `deposit_events` | public | pano | Pano writes; app reads live for balances |
| `users` | app | app | id, email, per-chain addresses, `watching_since` |

## Prerequisites

- **Docker** and **Docker Compose v2**
- Three **local blockchain nodes** on the host:
  - [Anvil](https://book.getfoundry.sh/anvil/) on `localhost:8545`
  - [Solana test validator](https://docs.anza.xyz/cli/examples/test-validator/) on `localhost:8899`
  - [Bitcoin Core](https://bitcoincore.org/) regtest on `localhost:18443`
- `cast`, `solana`, `spl-token`, `bitcoin-cli`, `curl`, `jq`
- A **BIP-39 mnemonic** — Ladon derives all addresses from it

## Quick start

### 1. Start local chains

```bash
# Terminal 1 — Anvil
anvil --host 0.0.0.0 -b 1

# Terminal 2 — Solana test validator
solana-test-validator --reset --account-index spl-token-owner

# Terminal 3 — Bitcoin Core regtest
bitcoind -regtest -txindex -rpcuser=rpcuser -rpcpassword=rpcpass \
  -rpcallowip=0.0.0.0/0 -rpcbind=0.0.0.0 -server -fallbackfee=0.00001
```

### 2. Create test tokens

```bash
# ERC-20 USDT on Anvil
mkdir -p /tmp/pano-e2e-token/src
cat >/tmp/pano-e2e-token/src/Token.sol <<'SOL'
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;
contract Token {
    string public name; string public symbol; uint8 public immutable decimals;
    mapping(address => uint256) public balanceOf;
    event Transfer(address indexed from, address indexed to, uint256 value);
    constructor(string memory n, string memory s, uint8 d, uint256 supply) {
        name = n; symbol = s; decimals = d; balanceOf[msg.sender] = supply;
        emit Transfer(address(0), msg.sender, supply);
    }
    function transfer(address to, uint256 value) external returns (bool) {
        require(balanceOf[msg.sender] >= value, "balance");
        balanceOf[msg.sender] -= value; balanceOf[to] += value;
        emit Transfer(msg.sender, to, value); return true;
    }
}
SOL
forge create /tmp/pano-e2e-token/src/Token.sol:Token \
  --rpc-url http://localhost:8545 \
  --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --broadcast \
  --constructor-args "Tether USD" "USDT" 6 1000000000000 | tee /tmp/pano-usdt.deploy
export USDT_CONTRACT=$(awk '/Deployed to:/ {print $3}' /tmp/pano-usdt.deploy)

# SPL USDC on Solana
solana config set --url http://localhost:8899
export USDC_MINT=$(spl-token create-token --decimals 6 | awk '/Creating token/ {print $3}')
spl-token create-account "$USDC_MINT"
spl-token mint "$USDC_MINT" 1000
```

### 3. Export environment variables

```bash
export LADON_MNEMONIC="test test test test test test test test test test test junk"

export SOLANA_GENESIS_HASH="solana:$(curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getGenesisHash"}' \
  http://localhost:8899 | jq -r '.result')"

export ANVIL_START_BLOCK=$(cast block-number --rpc-url http://localhost:8545)
export SOLANA_START_BLOCK=$(curl -s -X POST -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"getSlot","params":[]}' \
  http://localhost:8899 | jq -r '.result')
export BTC_START_BLOCK=$(bitcoin-cli -regtest -rpcuser=rpcuser -rpcpassword=rpcpass getblockcount)
```

### 4. Launch the stack

```bash
docker compose -f tests/e2e/docker-compose.e2e.yml up -d
```

Wait for all services to become healthy:

```bash
curl -s http://localhost:8080/health | jq
docker compose -f tests/e2e/docker-compose.e2e.yml ps
```

## Manual scenario

### 1. Register a user

```bash
RESULT=$(curl -s -X POST http://localhost:8080/register \
  -H 'Content-Type: application/json' \
  -d '{"email": "test@example.com"}')
echo "$RESULT" | jq

USER_ID=$(echo "$RESULT" | jq -r '.id')
EVM_ADDR=$(echo "$RESULT" | jq -r '.addresses.evm')
SOL_ADDR=$(echo "$RESULT" | jq -r '.addresses.solana')
BTC_ADDR=$(echo "$RESULT" | jq -r '.addresses.btc')
```

Addresses are assigned but **not yet watched** by Pano.

### 2. View profile — start watching

Viewing `/users/{user_id}` sets `watching_since` and inserts addresses into
`watched_addresses` for `APP_ADDRESS_TTL_SECS` (default 30 seconds):

```bash
curl -s "http://localhost:8080/users/$USER_ID" | jq
```

### 3. Send deposits on all blockchains

Send within the 30-second window:

**EVM — ETH:**
```bash
cast send --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --value 1ether "$EVM_ADDR" --rpc-url http://localhost:8545
```

**EVM — USDT:**
```bash
cast send "$USDT_CONTRACT" "transfer(address,uint256)" "$EVM_ADDR" 100000000 \
  --private-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --rpc-url http://localhost:8545
```

**Solana — SOL:**
```bash
solana transfer "$SOL_ADDR" 1 --allow-unfunded-recipient --url http://localhost:8899
```

**Solana — USDC:**
```bash
spl-token transfer "$USDC_MINT" 500 "$SOL_ADDR" \
  --url http://localhost:8899 --allow-unfunded-recipient --fund-recipient
```

**Bitcoin — BTC:**
```bash
bitcoin-cli -regtest -rpcuser=rpcuser -rpcpassword=rpcpass createwallet testwallet 2>/dev/null || true
bitcoin-cli -regtest -rpcuser=rpcuser -rpcpassword=rpcpass -generate 101 > /dev/null 2>&1
BTC_TXID=$(bitcoin-cli -regtest -rpcuser=rpcuser -rpcpassword=rpcpass sendtoaddress "$BTC_ADDR" 1.0)
bitcoin-cli -regtest -rpcuser=rpcuser -rpcpassword=rpcpass -generate 1
```

### 4. Verify deposits on-chain

```bash
cast balance "$EVM_ADDR" --rpc-url http://localhost:8545
spl-token balance "$USDC_MINT" --owner "$SOL_ADDR" --url http://localhost:8899
bitcoin-cli -regtest -rpcuser=rpcuser -rpcpassword=rpcpass \
  getrawtransaction "$BTC_TXID" true | jq --arg addr "$BTC_ADDR" '.vout[] | select(.scriptPubKey.address == $addr)'
```

### 5. Verify deposit_events and user balance

```bash
docker compose -f tests/e2e/docker-compose.e2e.yml exec -e PGPASSWORD=postgres postgres \
  psql -U postgres -d pano -c \
  "SELECT event, symbol, address, amount FROM public.deposit_events ORDER BY event_id;"

curl -s "http://localhost:8080/users/$USER_ID" | jq '.balances'
```

### 6. Verify cleanup after TTL

After 30+ seconds the `cleanup` task removes expired entries:

```bash
docker compose -f tests/e2e/docker-compose.e2e.yml exec -e PGPASSWORD=postgres postgres \
  psql -U postgres -d pano -c "SELECT count(*) FROM public.watched_addresses;"
```

Should return 0. `watching_since` on the user row is set to NULL.

### 7. Re-view profile — watching restarts

```bash
curl -s "http://localhost:8080/users/$USER_ID" | jq '.watching_since'
# => non-null timestamp — watching is active again

docker compose -f tests/e2e/docker-compose.e2e.yml exec -e PGPASSWORD=postgres postgres \
  psql -U postgres -d pano -c "SELECT count(*) FROM public.watched_addresses;"
# => 5 (ETH, USDT, SOL, USDC, BTC)
```

## Teardown

```bash
env \
  LADON_MNEMONIC="test test test test test test test test test test test junk" \
  USDT_CONTRACT="0x0000000000000000000000000000000000000000" \
  USDC_MINT="11111111111111111111111111111111" \
  SOLANA_GENESIS_HASH="solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG" \
  docker compose -f tests/e2e/docker-compose.e2e.yml down -v
```

> **IMPORTANT — Stop local blockchains**
>
> The Docker Compose stack only tears down the application containers.
> The three local blockchain nodes (Anvil, Solana test validator, Bitcoin
> regtest) run as host processes and must be stopped separately:
>
> ```bash
> # Kill all local blockchain processes
> pkill -f anvil
> pkill -f solana-test-validator
> pkill -f bitcoind
> ```
>
> You can verify they are all gone with:
> ```bash
> pgrep -af 'anvil|solana-test-validator|bitcoind'
> ```
> These processes consume CPU, memory, and disk I/O while idle. Forgetting to
> stop them will degrade host performance until the machine is rebooted.
