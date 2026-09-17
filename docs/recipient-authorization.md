# Recipient authorization binding

The RGB wallet distinguishes three identities:

| Identity | Definition | Purpose |
| --- | --- | --- |
| Economic | Existing Harness hash of the complete `PaymentRequest` | Economic interpretation and immutable request checks |
| Authorization | Economic ID for direct plans; versioned recipient scope below for recipient plans | Exact user/application intent being authorized |
| Execution | Existing Lightning payment hash | Reservation duplicate detection and authoritative payment status |

Alice and Bob may advertise/acquire the same invoice. They still represent the
same economic action and payment hash, but their acquisition contract digests
(and therefore authorization scopes) differ.

## Calculation and approval

`PaymentPlan::authorization_scope()` exposes the immutable scope. For a recipient
plan it is lowercase hexadecimal SHA-256 of the fixed-order JSON tuple:

```text
["rgb402-recipient-authorization-v1", economic_action_id, recipient_contract_digest]
```

This uses the same JSON tuple/SHA-256 conventions as existing Harness hashes.
The acquisition contract already binds human identifier, authoritative domain,
validated service, requested asset/amount, network and carrier ceiling. Changing
any bound field changes the authorization scope. It does not change economic
identity when the underlying invoice remains identical.

Human approval is stored as `plan_id → authorization_scope`, not merely plan-ID
membership. Execution first checks the plan's scope against its immutable
economic action and provenance. Where current policy requires human approval,
the stored approval must match that exact plan's scope. Approval of Alice's plan
cannot authorize Bob's, and vice versa. Plans remain individually consumed;
this is not a reusable authorization token or a scope-wide permission cache.

Automatic authorization uses the identical scope calculation but requires no
additional human prompt. Existing deterministic policy and thresholds decide
whether authorization is automatic or human. The existing Harness still
controls every economic transition and full-contract revalidation.

## Persistence and recovery

Recipient reservation rows add only optional `recipient_authorization_scope`.
Together with the existing fields, the row contains:

- `authority.economic_action_id`: the economic action authorized;
- `authority.source`: human or automatic authorization;
- `recipient_provenance.recipient_contract_digest`: the recipient intent;
- `recipient_authorization_scope`: their versioned binding.

It is appended and fsynced in the existing reservation before submission, for
both automatic and human authorization. No invoice, discovery document or HTTP
response is added. `WalletService::recipient_authorization_scope(payment_hash)`
returns this historical binding for audit; it is not permission to execute.

On open, present scope metadata must agree with the stored authority and
provenance. Inconsistent metadata is rejected. Missing fields remain valid for
legacy direct rows and older recipient rows; recovery does not invent a missing
historical recipient scope. Existing provenance and authority checks also run.
The local journal remains trusted application storage, not a signed attestation.

Recovery restores the existing conservative uncertain state and duplicate
reservation, never approval entries or executable plans. Repreparing after
restart still follows current policy and cannot bypass the payment-hash
reservation. Direct reservations retain their prior shape, with no new scope
field; their plan authorization scope is the economic action ID.

## Regression evidence

The existing bridge fixture adds:

- `same_invoice_distinct_recipient_scopes_require_separate_human_approval`:
  exact same invoice/request and economic/payment identities for Alice, Bob and
  direct input; distinct recipient scopes; deterministic hash formula;
  changed domain/service scopes; approval isolation in both directions; approved
  exact-plan execution; cross-recipient and direct-path duplicates blocked;
  persisted human scope and restart without restored approval.
- `automatic_recipient_scope_is_durable_and_recovery_rejects_mismatch`:
  unchanged automatic policy; automatic source/scope persisted; restart retains
  scope and blocks another recipient's duplicate; tampered scope rejected;
  pre-scope recipient rows remain readable without invented metadata.

The existing bridge legacy-direct regression and all Harness/wallet tests remain
in place. No discovery/acquisition implementation, parsing dependency, policy,
L402 flow, model tool surface, UI or Harness economic state machine is changed.

Validation: **114 workspace Rust tests**, including all seven bridge regressions,
and **16 frontend tests** pass. Formatting, strict workspace/all-target Clippy,
whitespace checks and the production UI build also pass. All new tests are offline;
no live payment or model request was made.
