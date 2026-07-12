# Responses

## Deposit event

Detected and confirmed deposits use the same envelope. `event` distinguishes lifecycle state.

```json
{
  "event_id": "01J2V8Q8YQW18Y0AM3QFQZ76A7",
  "event": "pano.deposit.confirmed",
  "version": 1,
  "occurred_at": "2026-07-12T12:00:00Z",
  "data": {
    "tx_id": "0xabc123",
    "caip2": "eip155:1",
    "symbol": "USDC",
    "address": "0x1111111111111111111111111111111111111111",
    "block_number": 21000000,
    "log_index": 4,
    "amount": "1000000",
    "sender": "0x2222222222222222222222222222222222222222",
    "confirmations": 12,
    "timestamp": "2026-07-12T11:59:42Z"
  }
}
```

## Errors

HTTP ingress returns standard API errors. Invalid JSON or an invalid watch produces a client error; disabled routes and unavailable services produce server errors.

```json
{
  "error": "invalid_request",
  "message": "address is required when chains is empty"
}
```

Clients may retry transport failures and `5xx` responses with bounded exponential backoff. Do not retry invalid watches until configuration or request data is corrected.
