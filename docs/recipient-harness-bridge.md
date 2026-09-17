# Recipient invoice → RGB Harness bridge

Application entry point:

```rust,ignore
rgb402_agent::recipient::bridge::prepare_recipient_payment(
    &mut wallet, &validated_candidate, &original_acquisition_contract,
).await
```

The result is an ordinary `rgb402_payment::wallet::PaymentPlan`. Discovery and
acquisition are never invoked by the bridge, preparation, approval, execution or
recovery. There is no new model tool, economic state machine or payment workflow.
The application subsequently uses the existing `approve_from_human` (when
required), `execute_payment` and authoritative `payment_status` methods.

## Mapping and defensive validation

The bridge first compares the candidate's **complete immutable acquisition
contract**, including its digest, with the original application intent. Contract
matching no longer relies on digest equality alone. Neither type can be
constructed through deserialization; a forged digest cannot substitute changed
identity, service or economic fields.

`WalletService::prepare_recipient_invoice` then enters the same private
preparation function as `prepare_payment`. It decodes the exact raw invoice once
and compares the complete decoded `PaymentRequest` with the acquired request
before evaluating balance/policy or creating a plan. This preparation-time decode
is deliberate boundary revalidation, not another acquisition. There is no second
decode within preparation.

Both paths pass the same request to `Action::new(TaskKind::RgbPayment, &request)`.
The immutable economic contract still consists of asset ID, amount, invoice,
payment hash, expiry, network and carrier msat. Therefore equivalent inputs have
identical economic action IDs, balance interpretation and policy decisions.
Recipient provenance does not participate in the economic action ID, approval
threshold, payment identity or duplicate key.

Execution still re-decodes and compares the full request, reads current balance,
computes current UTC/day spend and evaluates existing policy. Acquisition-time
expiry validation never overrides those checks. An invoice expiring while the
application waits for human approval is denied before reservation/submission.
No changes were made to the Harness state machine, policy or L402.

## Bounded provenance and persistence

`RecipientProvenance` retains:

- `identifier`: the canonical human account, at most 318 ASCII bytes;
- `authoritative_domain`: at most 253 ASCII bytes;
- `recipient_contract_digest`: 64 lowercase hexadecimal characters;
- `invoice_id`: SHA-256 of the exact invoice bytes, 64 lowercase hex characters;
- `payment_hash`: exactly equal to the economic request's Lightning hash.

The wallet validates bounds, identity/domain consistency, invoice hash and payment
hash before attaching this metadata. The low-level wallet adapter treats it as
trusted-application audit metadata, not a cryptographic account assertion. Only
the higher-level validated-candidate bridge establishes acquisition provenance;
neither API grants authorization.

The plan exposes metadata through `PaymentPlan::recipient_provenance()`. The
existing reservation row adds one optional `recipient_provenance` field, written
and fsynced with the same request and authorization before submission. No new
journal or replay database is introduced. The existing `authority` field binds
the row to its economic action ID. Payment results/status join to it by payment
hash via `WalletService::recipient_provenance(hash)` and `task(hash)`.

No raw invoice is added to provenance. The existing `PaymentRequest.invoice`
remains the sole invoice field in each plan/reservation; the reservation stores
it once. The caller's acquisition result and ordinary wallet plan may coexist in
memory, as the bridge borrows its input and the existing service owns its plan.
There is no additional raw-invoice field or serialized copy in the provenance,
model observations or trace. No JRD, HTTP body, endpoint path or credential is
persisted in the new metadata.

The field defaults to `None` during deserialization and is omitted for direct
payments. Older rows lacking provenance, including rows without Harness
authority metadata, still recover under the existing conservative semantics.
Present provenance is checked against the stored request on open. Recovery
retains metadata but restores no plan or submission permission. Failed/uncertain
submissions retain their reservation and provenance. Duplicate detection remains
solely the existing payment-hash check, regardless of acquisition or direct entry.

## Focused regression map

Tests live beside the signed acquisition fixtures in
`recipient/acquisition/bridge_tests.rs`.

| Invariant | Test |
| --- | --- |
| Same full request, asset/amount, balance, policy and action ID as direct input; provenance grants no approval; result/journal provenance retained; cross-path duplicate rejected | `bridge_equals_direct_economics_and_preserves_provenance_without_authority` |
| Uncertain submission retains provenance; restart yields uncertain state, no restored plan/approval, and no second send; status remains authoritative | `uncertain_submission_restart_keeps_provenance_and_cannot_resubmit` |
| Expired acquired input denied; changed node decode rejected both at bridge preparation and after approval | `expired_acquisition_and_changed_decode_cannot_execute` |
| Changed intent and forged digest rejected before node decode; legacy rows without provenance readable and still block duplicates | `mismatched_or_forged_digest_stops_before_harness_and_legacy_stays_readable` |
| Actual UTC time passes after approval; unchanged invoice is denied before reservation/submission | `invoice_expiring_after_approval_is_rejected_at_execution` |

These use offline invoice-service fixtures, real locally signed/parsed invoices
and a mock RGB node. No model request, live discovery, live acquisition or payment
is needed. Existing wallet/Harness tests continue to cover daily limits, changed
balance, cancellation, exact approval binding and journal locking.

Validation completed: **112 workspace Rust tests**, including the five focused
bridge regressions, and **16 frontend tests** pass. `cargo fmt --check`, strict
workspace/all-target Clippy, `git diff --check` and the production frontend build
also pass. The final provenance bound check was additionally verified by rerunning
all five bridge tests. No live payment or model request was made.

Recipient provenance additionally binds the separate plan authorization scope;
it still does not change economic action IDs or payment-hash duplicate detection.
See [recipient authorization binding](recipient-authorization.md) for calculation,
approval checks, durable metadata and recovery compatibility.
