# RGB402 Roadmap

Milestone 1 is complete: a real two-node RGB Lightning payment settled through
our wallet with human approval and verified balances. The clean-reset demo,
persistence checks and normal test suite passed. See [regtest.md](regtest.md) and
[the acceptance evidence](acceptance/rgb-lightning-payment.json).

The phases below describe the original bootstrap roadmap. Polar was superseded
by pinned upstream regtest infrastructure. L402 and AI planning remain future work.

## Phase 0: Bootstrap

- Build a compilable Rust workspace.
- Implement a merchant HTTP 402 challenge endpoint.
- Implement an autonomous buyer agent.
- Enforce deterministic spending policy.
- Simulate RGB settlement without external network calls.
- Prove the purchase and rejection flows with integration tests.

## Phase 1: Polar Node Sandbox

- Stand up the local Lightning/RGB development environment in Polar.
- Decide the concrete RGB Lightning implementation target, such as RGB Lightning Node or Bitlight RLN.
- Map provider methods to the selected node API:
  - create or parse RGB Lightning invoices;
  - pay invoices;
  - query payment status;
  - verify settlement for merchant-side access decisions.
- Keep the agent and merchant depending only on `PaymentProvider` and `SettlementVerifier`.

## Phase 2: Stronger Authorization

- Replace the V0 payment-ID retry with a stronger proof or authorization token.
- Evaluate whether L402 compatibility is desirable.
- If claiming L402 compatibility, implement and test macaroon/preimage semantics instead of only describing inspiration.

## Phase 3: AI Planner Boundary

- Add an AI planner that can propose resource purchases.
- Keep deterministic policy as a mandatory safety gate.
- Add audit logs showing proposed action, policy decision, payment, and received value.

