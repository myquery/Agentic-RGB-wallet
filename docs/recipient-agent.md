# Human-recipient preparation in the interactive agent

The gateway adds one tool, `wallet_prepare_recipient_payment`, with the strict
schema below. Its amount is a positive integer in asset base units, decoded as
Rust `u64`; unknown fields, fractions, negatives and overflow are rejected.

```json
{
  "type": "object",
  "properties": {
    "identifier": {"type": "string", "maxLength": 318},
    "asset_id": {"type": "string", "maxLength": 256},
    "amount": {"type": "integer", "minimum": 1, "maximum": 18446744073709551615}
  },
  "required": ["identifier", "asset_id", "amount"],
  "additionalProperties": false
}
```

No discovery/acquisition tool, network destination, raw invoice, digest, scope,
payment hash, approval argument or transport setting is accepted in this input.
The existing wallet policy's configured `max_carrier_msat` supplies the acquisition
ceiling through a read-only getter. There is no new configuration or credential.

## Composition and existing execution

`wallet_agent/recipient.rs` calls the existing layers in order:

1. `resolve_recipient(identifier)`;
2. `RecipientInvoiceContract::new` with the resolved descriptor, explicit asset
   and integer amount, and application-owned carrier ceiling;
3. `acquire_recipient_invoice(&contract)` including existing local validation;
4. `prepare_recipient_payment(wallet, &candidate, &contract)`;
5. the existing wallet preparation/revalidation and Harness policy.

No protocol validation, decoding, authorization hashing, state transition,
reservation or execution logic is duplicated. A private service seam supplies
fixtures in tests; production always uses the existing public-HTTPS components.

After preparation, the model uses the unchanged `wallet_execute_payment` input
`{plan_id}` and `wallet_payment_status` input `{payment_hash}`. The former still
calls the ordinary wallet execution/status methods. Execution does not resolve
or acquire again, and rejects any extra recipient/asset/amount/invoice/approval
arguments. Changed intent requires a new preparation operation.

## Policy and authoritative approval

Approval-required recipient plans return `TurnOutcome::Prepared` and pause the
loop. The CLI and existing PWA confirmation sheet obtain recipient identifier
and authoritative domain from `PlanView::from(&stored_plan)`, copied only from
its immutable provenance. Asset, amount and policy likewise come from the stored
plan. Model prose is never used to fill confirmation details or grant authority.
The existing plan-ID-only application approval reaches `approve_from_human`,
which binds that plan's actual recipient authorization scope.

For recipient plans whose existing Harness policy returns `Allow`, preparation
requires no human prompt. The gateway permits the existing execute tool to reach
wallet execution only for such stored recipient plans or an application-confirmed
plan. It does not mint human approval. The wallet rechecks policy, request,
balance, expiry and authorization before reserving/submitting. Direct-invoice
plans retain the existing gateway behavior of application confirmation even for
small RGB payments. Machine/L402 policy and behavior remain unchanged.

Denied plans have no allowed next action or approval prompt. Recipient plan
observations include `policy_denied` or `insufficient_balance` where appropriate.
Preparation alone never submits a payment, even when policy allows automatic
execution. Economic identity, authorization scopes, duplicate detection,
recovery and authoritative settlement remain unchanged.

## Bounded observations

Recipient `PlanView` carries only the identifier/domain as added context; it has
no contract digest, authorization scope or service path. Its raw invoice field is
cleared even in the typed gateway result. The wallet privately retains the exact
invoice in its existing bound plan. Subsequent `wallet_get_plan` uses the same
projection, so it cannot reveal the acquired invoice through that tool.

The provider-facing observation is an ordinary plan result with a compact
recipient summary:

```json
{
  "type": "plan",
  "status": "prepared",
  "plan": {
    "plan_id": "<existing bound plan ID>",
    "recipient": {"identifier": "alice@example.com", "authoritative_domain": "example.com"},
    "asset_id": "rgb:...",
    "amount": "5",
    "available_balance": "100",
    "policy": {"decision": "require_approval", "reason": "..."},
    "application_confirmation_required": true
  },
  "payment_authorized": false,
  "next_allowed_actions": ["await_application_authorization"]
}
```

The existing compact Harness task summary is attached. Automatically authorized
recipient plans report `payment_authorized: true` (policy authorization, not human
approval) and `execute_bound_plan`; denied plans report `status: denied` and no
allowed next action. Provider serialization still enforces the existing 8 KiB
observation budget.

The representative fixture measures **748 UTF-8 JSON bytes including the Harness
summary**, with **zero protocol-internal bytes**: no raw invoice, WebFinger/JRD,
service endpoint, DNS/TLS output, authorization scope, contract or response body.
This is measured in `recipient_tool_enters_harness_and_returns_bounded_protocol_free_context`.

Failures use fixed codes and short recovery guidance: `invalid_recipient`,
`recipient_not_found`, `recipient_unsupported`, `discovery_unavailable`,
`discovery_security_rejected`, `acquisition_unavailable`,
`acquisition_security_rejected`, `invoice_invalid`, `asset_mismatch`,
`amount_mismatch`, `invoice_expired`, and existing wallet/gateway codes.
No underlying parser/node/HTTP error text is forwarded; automatic retry is not
recommended. A failure before bridging creates no economic plan or reservation.

## Agent instructions and tests

The always-loaded instructions add only direct-invoice versus recipient tool
selection, explicit asset/integer amount, returned next actions, application
approval and existing execute/status tools. Protocol details remain here, outside
model context. The eight-step loop limit remains unchanged.

Five deterministic agent-level tests in
`recipient/acquisition/agent_tests.rs` compose actual discovery response validation,
local signed-invoice parsing and bridge preparation using fake transport/services,
a scripted model and a mock node. They cover:

- matching the direct path's economic action, bounded observations, exact approval,
  model-written assent rejection, existing execution/status and no reacquisition;
- unchanged automatic policy and cross-recipient duplicate protection;
- another recipient's approval failing to authorize a plan, plus mutation rejection;
- fixed discovery/acquisition failures before wallet decoding or submission;
- strict preparation schema, forbidden internal fields, and newly requested
  asset/amount failing when the acquired invoice does not match.

Existing direct-invoice, Harness, bridge, authorization, eight-step and provider
contract tests remain in the suite. A PWA regression verifies recipient/domain
and policy in the existing approval sheet. No live Internet, model API, node or
payment is required for the regression suite.

Validation: **119 workspace Rust tests** and **17 frontend tests** pass, including
the five new agent-level regressions and recipient approval view. Formatting,
strict workspace/all-target Clippy, whitespace checks and the production frontend
build pass. No live model, public recipient service or payment was used.
