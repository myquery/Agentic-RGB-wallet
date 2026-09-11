# Agentic RGB Wallet Architecture

## Current wallet path

`wallet` CLI → `WalletService` → deterministic `WalletPolicy` → private approved
payment capability → `RgbNode` → typed HTTP adapter → rgb-lightning-node.

`rgb402-core::wallet` holds the wallet domain and policy, independent of HTTP,
agent logic and the old RGB402 challenge protocol. `rgb402-payment::{config,rgb,wallet}`
contains configuration, infrastructure and orchestration. The journal uses a local
append-only file and exclusive process lock; no database or new crate is needed.
Amounts are exact integer asset base units, with independent per-asset budgets.

Prepared plans expose read-only getters and serialize for display, but cannot be
deserialized into executable authority. Execution accepts only an ID found in the
service's private plan registry. Human approval is a trusted interface operation;
it must never become a model-callable tool. The approved node capability can only
be created after execution-time revalidation. The original invoice is submitted
without overrides. Audit events omit node secrets and raw response bodies.

There is no model integration yet. Future structured tools should wrap assets,
balance, decode, prepare, execute-by-plan-ID and status. They must not expose human
approval, node configuration, the raw client, or arbitrary HTTP endpoints.

See [node API and live acceptance](node-api.md) for the contract pin and limitations.
The older simulated HTTP 402 flow below remains as historical demo functionality.

## Bootstrap Shape

The bootstrap architecture has four layers:

1. `rgb402-core` defines the protocol vocabulary: assets, amounts, payment requests, challenges, receipts, payment status, and spending policy decisions.
2. `rgb402-payment` defines the payment boundary with `PaymentProvider` and `SettlementVerifier`.
3. `rgb402-merchant` exposes an Axum HTTP API that returns `402 Payment Required` until settlement is verified.
4. `rgb402-agent` implements the deterministic buyer loop: request, parse challenge, enforce policy, pay, retry.

The design keeps the merchant and buyer independent from the eventual RGB Lightning implementation.

## Payment Boundary

The buyer depends on:

```rust
PaymentProvider::pay(request) -> PaymentReceipt
PaymentProvider::status(payment_id) -> PaymentStatus
```

The merchant depends on:

```rust
SettlementVerifier::is_settled(request, payment_id) -> bool
```

This prevents the merchant from unlocking content just because a client sends a claim such as `paid=true`. The client may provide a payment identifier, but the merchant decides access by checking settlement state.

## Simulated Settlement

`SimulatedRgbPaymentProvider` supports two local modes:

- in-memory ledger for deterministic tests;
- JSON-file ledger for the two-process demo.

Both modes create deterministic payment identifiers from the payment request ID. No external network or node calls are made.

## Spending Policy

The spending policy is a hard safety boundary. It checks:

- the protocol scheme is `RGB402`;
- the challenge has not expired;
- the requested asset is allowed;
- the amount is within the maximum single-payment limit;
- the projected session spend is within budget.

Future AI planning can propose purchases, but it must not bypass this deterministic policy layer.

## Real-node Integration

The wallet uses `RgbLightningClient` through the `RgbNode` trait to connect to the pinned `rgb-lightning-node` regtest environment. The original Polar integration remains a stub in the simulated purchase flow; it is not needed by the working wallet path. See [regtest setup](regtest.md) for the two-node environment and payment verification commands.
