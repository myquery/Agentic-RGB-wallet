# RGB402 Bootstrap Protocol

RGB402 is an experimental application protocol for HTTP 402 payment challenges using RGB-denominated payment requests.

It is inspired by L402's machine-payable resource flow, but this bootstrap is **not L402-compatible**. In particular, it does not implement L402 macaroon authorization, preimage verification, or compatibility tests.

## Flow

```text
Agent
  |
  | GET /premium-analysis
  v
Merchant API
  |
  | HTTP 402 Payment Required
  | JSON RGB402 challenge
  v
Agent policy engine
  |
  | validate asset, amount, expiry, and budget
  v
Payment provider
  |
  | simulated RGB settlement in V0
  v
Merchant settlement verifier
  |
  | verify payment identifier against expected request
  v
200 OK
```

## Challenge

Example:

```json
{
  "scheme": "RGB402",
  "payment_request_id": "req-premium-analysis",
  "asset_id": "rgb:boss",
  "amount": {
    "minor_units": 400,
    "precision": 2
  },
  "invoice": "simulated-rgb-ln-invoice:req-premium-analysis",
  "expires_at": {
    "seconds": 4102444800
  },
  "resource": "/premium-analysis"
}
```

Amounts use integer minor units plus precision. The value above represents `4.00`.

## Retry

After payment, the buyer retries the resource with:

```text
x-rgb402-payment-id: sim-rgb-payment-req-premium-analysis
```

The merchant treats this header only as a lookup key. It verifies settlement against the expected payment request before returning `200 OK`.

## Known V0 Simplifications

- Settlement is simulated locally.
- The payment identifier is deterministic for demo clarity.
- There is no macaroon, preimage, caveat, or bearer-token authorization model.
- There is no real RGB state transition, Lightning invoice payment, route finding, HTLC, or node RPC call.
- Payment authentication is intentionally simpler than normal L402.

## Future Direction

The simulated provider should be replaced by an implementation backed by RGB Lightning tooling. For local development, the intended node environment is Polar. The protocol surface should remain stable while the payment provider and settlement verifier gain real node-backed behavior.

