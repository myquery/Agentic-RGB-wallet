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

The AI agent proposes purchases through typed wallet tools and cannot bypass the
deterministic wallet policy. The CLI and mobile PWA use the same bound-plan
approval mechanism; the PWA's loopback Axum API owns the wallet session and
accepts approval by plan identity only. See [the PWA architecture](pwa.md).

## Real-node Integration

The wallet uses `RgbLightningClient` through the `RgbNode` trait to connect to the pinned `rgb-lightning-node` regtest environment. The original Polar integration remains a stub in the simulated purchase flow; it is not needed by the working wallet path. See [regtest setup](regtest.md) for the two-node environment and payment verification commands.

## Milestone 4: separate BTC machine purchases

`rgb402-core::machine` owns the deterministic satoshi policy.
`rgb402-payment::lightning` adds BTC-only node capabilities using the existing
client; `commerce` owns challenge parsing, origin restrictions, bound machine
plans, a durable reservation journal, settlement proof and resource retry.
`rgb402-merchant::l402` and the `l402-merchant` binary implement the real protected
resource alongside the unchanged simulated RGB merchant.

The agent conditionally exposes one `agent_fetch_resource` tool when commerce is
configured. Automatic policy executes only inside the deterministic service.
Larger plans pause the loop; the existing application-only approval boundary
confirms their stored identity. PWA machine receipts/activity distinguish sats
from RGB units. The six workspace members and eight-step loop remain.
See [L402 setup and acceptance](l402.md).

## Agent Harness v1

Both payment services now enforce explicit contract-bound transitions through
`rgb402-payment::harness`, retaining their existing journals and policies. See
[the harness boundaries, state machines and invariant/test map](agent-harness.md).

## Recipient identity and discovery

The standalone `rgb402-agent::recipient` module resolves `name@domain` through
HTTPS WebFinger into a validated same-origin RGB invoice-service descriptor.
DNS locates the domain; HTTPS authenticates its transport; WebFinger advertises
an account capability. Discovery grants no spending authorization.

The layers are **identity/discovery → future invoice acquisition → economic
Harness v1**. A future acquired invoice must match the user's requested asset
and amount before entering the unchanged Harness. The RGB invoice remains the
payment request; Harness policy, application approval and authoritative evidence
remain the economic authority. Discovery itself acquires no invoice or spending authority; the standalone
acquisition layer below supplies validated candidates. Payment through aliases
is not implemented. See [the discovery profile, API and security policy](recipient-discovery.md).

Recipient invoice acquisition now extends that preflight layer through a single
HTTPS POST and local RGB-aware BOLT11 validation. A private immutable contract
binds identity, service, asset, amount, network and carrier ceiling to the
validated candidate. Domain authentication, discovery, provenance and economic
intent validation confer no economic authorization. No interactive agent tool or
Harness entry is added. See [Recipient Invoice Acquisition v1](recipient-invoice-acquisition.md).

The [recipient-to-Harness bridge](recipient-harness-bridge.md) now adapts a validated
candidate and the original acquisition contract into an ordinary RGB payment
plan. It adds bounded optional provenance to existing plans/reservations while
retaining exact decoded-request comparison, current-time execution checks,
application approval, duplicate protection and authoritative settlement status.
It performs no discovery/acquisition and adds no agent tools.

The interactive agent now exposes one [recipient preparation capability](recipient-agent.md).
It composes the existing preflight and bridge layers, returns a bounded ordinary
plan observation, and reuses the existing execution/status tools and immutable
application approval. Protocol internals remain outside model context.
