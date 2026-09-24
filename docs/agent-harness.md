# Agent Harness v1

This is an extraction around the two existing workflows, not a new executor or
agent framework. Policies, tools, node transports, journals, application approval
and the six-member workspace remain. No live payment was requested for this refactor.

## Boundaries and files

| Boundary | Implementation |
| --- | --- |
| Task contract and transitions | New `rgb402-payment::harness::{TaskContract,Action,TaskSnapshot}` |
| RGB execution | Existing `payment::wallet`, now gated by `Action` |
| L402 execution/recovery | Existing `payment::commerce`, now gated by `Action` |
| Authorization | Existing core policies and trusted application confirmation methods |
| Persistence | Existing RGB and machine journals, extended with optional `authority` |
| Model proposals and trace | Existing `agent::wallet_agent` typed dispatcher, plus bounded `GatewayEvent` records |
| Model observation | New `wallet_agent::observation`, applied by the existing OpenAI transport |
| Project context | Root `AGENTS.md`; this document supplies deeper detail |

The full executable contract remains the immutable RGB `PaymentRequest` in its
plan/reservation, or the L402 `(canonical URL, decoded invoice, macaroon)` in its
purchase record. `TaskContract` binds those exact parameters by a SHA256 economic
action ID and declares the completion criterion. No raw conversation is copied
into a contract, reservation, action snapshot or trace. The macaroon contributes
to the digest but never appears in a snapshot or model observation.

L402 review handles now use the full contract digest rather than the payment hash
alone, preventing aliasing when two resource origins present the same invoice.
A plan ID is an application review handle; the economic action ID binds the full
contract; the invoice payment hash identifies the node payment. These roles are
separate. The durable record joins the economic action ID/authorization to its
invoice, hash and, for L402, URL and challenge.

## State machines

```text
RGB:
Requested → Decoded → Validated → Prepared
  → Authorized (automatic policy), or
  → AwaitingAuthorization → Authorized (application human approval), or Denied
Authorized → [journal flush] Reserved → Submitted
Submitted/Uncertain → Submitted (pending) / Settled / Failed
AwaitingAuthorization/Authorized → Cancelled

L402 paid resource:
Requested → Challenged → Validated → Prepared
  → Authorized (automatic policy), or
  → AwaitingAuthorization → Authorized (application human approval), or Denied
Authorized → [journal flush] Reserved → Submitted
Submitted/Uncertain → Submitted (pending) / Settled / Failed
Settled → ProofReady → [authenticated retry] Complete
AwaitingAuthorization/Authorized → Cancelled

Restart with any reservation:
RecoveredReservation → Uncertain → query status only
  → Settled → verify proof → retry resource (L402)
```

RGB completes at authoritative `Settled`. L402 completes only after settlement,
SHA256 verification of the node preimage, and an authenticated HTTP 200 response
with a valid bounded JSON body. A free-resource HTTP 200 remains the existing
non-economic path and does not create a payment action or authorization.

`Action` has private state and no `Deserialize` implementation. Transition and
authorization methods are crate-private. Calling submit before reservation,
submitting twice, unlocking without proof, or granting approval after cancellation
fails in application code. Recovery cannot restore the submit transition.

Repeated status/proof/resource reads are allowed; they do not reopen authorization.
A contradictory terminal node status fails closed. A resource transport or body
failure leaves the paid action recoverable with its existing proof. A pending or
uncertain result never establishes completion.

## Policy and execution

`WalletPolicy` and `MachinePolicy` are unchanged. Machine purchases of 3 sats are
automatically authorized under the configured inclusive 10-sat threshold; 50 sats
requires application approval. Denied actions cannot reach reservation/submission.
The agent's RGB confirmation gate remains in addition to wallet policy.

At execution, services still re-decode and compare the complete invoice, recheck
balance and budget, and enforce existing approval requirements. The harness then
compares the entire economic contract with the authorized digest. Neither amount,
asset, invoice/recipient hash, carrier, network, expiry, origin nor challenge can
be silently substituted. Only after the existing journal flush succeeds can the
harness enter `Reserved` and consume its single submission transition.

RGB cancellation now marks the wallet action cancelled and removes authorization.
The plan remains readable to preserve existing gateway error behavior, but cannot
be approved or executed again. L402 cancellation removes its pending plan and any
approval. A later newly prepared action still needs fresh policy authorization.

## Durable state and compatibility

New reservation entries have an optional field:

```json
{"authority":{"economic_action_id":"<contract digest>","source":"human"}}
```

The source is `human` or `automatic`. Existing files lacking that field load as
`legacy_unknown`; recovery does not fabricate historical human approval. A present
but mismatched contract digest fails closed. Journals retain their existing locks,
append/flush behavior, daily accounting and duplicate keys. No second journal or
conversation persistence was introduced, and existing acceptance records are not
rewritten by this refactor.

A reservation survives the ambiguous interval between disk flush and the network
call. On restart it means **submission may have occurred**, not that it certainly
did. `observed_submission_attempts` counts calls observed in the current process
only (0 after recovery, at most 1 for a live action); it is not a fabricated lifetime
counter. Both services consult persisted reservations before any new submission.
The node remains authoritative for settlement, and L402 re-fetches its preimage
from the node rather than persisting it.

Transient prepared plans and unspent human approvals do not survive restart. This
is intentional: they must be prepared/reviewed again. Reservation authorization
metadata is durable for audit and cannot itself authorize another submission.

## Evidence and trace

`WalletService::task(payment_hash)`, `WalletService::plan_task(plan_id)` and
`CommerceService::task(url)` return read-only
snapshots. They expose action identity, rail, state, authorization source, observed
submission count, uncertainty and a maximum of 32 typed transition events. Equal
consecutive polling events coalesce. The corresponding journal supplies the exact
contract when needed. No capability can be reconstructed from a snapshot.

`WalletAgent::trace()` retains at most 32 gateway records: known tool name,
normalized outcome and relevant task snapshot, plus explicit stop reasons for
application authorization, model explanation, model failure and the eight-step
limit. Unknown model-supplied tool names are normalized to `unavailable`; raw
arguments and full log output are excluded. The trace is diagnostic, never an
execution input. `model_explanation_only` cannot change task completion.

Traces are process-local and intentionally bounded. Existing audit logging and
reservation metadata support historical inspection; full transition histories are
not replayed from the journal. After a crash, reconstruct reservation/authorization
and query the node instead of inventing missing events.

## Model context and progressive disclosure

The application retains its existing full typed results for confirmation and UI.
Only the provider-facing projection changes. Each tool observation is at most
8 KiB of serialized JSON; oversized results produce a small structured observation
error with explicit instructions not to repeat economic execution. L402 resource
content over 4 KiB is omitted from the model observation and remains in the existing
application result. Failed observations include bounded retryability information.
Each economic tool result also carries its application-owned task snapshot. The
provider discloses only action ID, kind, state, authorization and submission
uncertainty, omits the trace, and derives allowed next actions from that snapshot.

For an RGB payment the model needs the user intent/invoice once, asset display
metadata, relevant balance, decoded economic fields, plan ID/policy, application
confirmation, and authoritative payment status. Decoded and prepared observations
omit the repeated full invoice. The measured regression fixture reduced prepared
plan JSON from **6,312 to 567 bytes (91.0%)**. This is a byte measurement using a
6,000-character synthetic invoice, not a tokenizer or full-session cost claim.

For an L402 fetch the model needs the URL, cost, policy, approval requirement,
normalized payment/resource status and bounded purchased content. It does not need
402 headers, macaroon, preimage, node responses, logs, polling events or the entire
harness trace. Pending/paid-but-unavailable observations suggest `fetch_same_url`;
plan observations require application authorization. These are guidance, not grants
of permission: the services enforce every transition independently.

The existing bounded conversation and eight-step loop remain. No automatic summary
or conversation-derived state is used for economic recovery. Start with `AGENTS.md`,
then this/subsystem documentation, the exact task snapshot, and the corresponding
journal/node evidence only when needed.

## Invariants and tests

All prior tests remain. New or extended coverage is marked **new**.

| Required invariant | Tests / enforcement |
| --- | --- |
| 1. Model cannot authorize | `invalid_and_forged_arguments_are_rejected`, `machine_tool_preserves_application_boundary_and_rejects_model_approval` |
| 2. Deterministic policy | Core `policy_denials`, `independent_machine_limits_and_inclusive_threshold` |
| 3. Authorization before execution | `prepare_approve_execute_status_and_replay`; **new** `authorization_binding_and_at_most_once_transitions` |
| 4–5. Exact authorization binding | `approval_is_bound_to_one_plan`, `changed_invoice_or_balance_cannot_use_approval`, `approval_revalidates_balance_and_invoice`; **new** `rgb_contract_binds_asset_recipient_invoice_amount_and_carrier` and `approval_for_same_invoice_at_one_origin_cannot_authorize_another` |
| 6. At-most-once submission | `automatic_payment_and_duplicate_hash`, `auto_purchase_unlocks_and_duplicate_and_restart_do_not_repay`; **new** transition guards |
| 7. Resource retries reuse payment | `resource_timeout_after_settlement_recovers_proof_without_second_payment` |
| 8. Polling never submits | `prepare_approve_execute_status_and_replay`, `pending_failed_uncertain_and_bad_proof_never_unlock_or_repay`; **new** recovered action cannot submit |
| 9. Delayed settlement/retry | `delayed_settlement_unlocks_in_the_original_fetch_without_another_prompt` |
| 10. Cancellation means no payment | Existing agent/web cancellation tests; **new** `cancellation_consumes_wallet_plan_without_submission`, `cancelled_machine_plan_cannot_reuse_approval` |
| 11. Origin restrictions | `amount_mismatch_and_denied_urls_do_not_pay` (unchanged transport restrictions) |
| 12. Completion requires evidence | Existing proof and failure tests; **new** invalid proof/unlock transitions and `model_narration_is_not_economic_evidence` |
| 13. Recovery outside conversation | Existing restart tests; **extended** `journal_survives_restart_and_locks` checks authority metadata; **new** `legacy_reservation_recovers_without_inventing_authorization`, `recovery_never_restores_execution_authority` |
| 14. Bounded structured failures | Existing sanitized failures; **new** `oversized_and_failed_observations_are_bounded_and_structured`, `gateway_trace_is_bounded_and_does_not_copy_untrusted_arguments` |
| Context reduction | **new** `invoice_is_not_duplicated_in_model_plan_context`; provider contract still checks roles/tool-call IDs |

## Remaining limits

The known locked regtest-node environment is separate from harness correctness;
this refactor uses mock nodes and no new live settlement. The prior live evidence
remains under `docs/acceptance/`.

At-most-once submission intentionally trades availability for safety after ambiguous
failure; operator inspection may be needed. Local journals are trusted application
files, not tamper-proof audit storage. Future contract serialization changes require
an explicit migration for existing digests. No cross-process distributed scheduler,
mainnet fee-budget change, merchant feature or UI change is included. Model prose
may still be mistaken; it never establishes economic evidence or completion.

## Validation result

All **84 Rust tests** and **16 frontend tests** pass. `cargo fmt --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, `git diff --check`, and
`npm --prefix apps/wallet-ui run build` pass. Tests use deterministic fake models
and local HTTP mock nodes; no API key, real payment or unlocked regtest node was
required. The new tests extend the existing 72-test Rust baseline by 12 tests.
