# Completed human-recipient payment: reconstructed acceptance

The existing PWA session records the request:

> Pay 5 R402USD to alice@3f0e-102-88-113-62.ngrok-free.app.

The exact payment hash is
`8f14fd52544defc8113a409c39f5068b2eae6a7b5846c0302637ac38275b8562`.
No payment, invoice, plan, approval or reservation was created during reconstruction.

## Authoritative application, journal and node evidence

The PWA session records asset lookup, preparation, application approval, two
pending observations for the same hash, and a model response correctly saying
pending rather than claiming settlement. The journal contains exactly one entry
and that entry is this payment: five units of the fresh R402USD asset, human
authority, immutable recipient/domain provenance and recipient authorization scope.
The economic-action hash, invoice SHA-256 identity and recipient scope were
independently recomputed in memory and match the persisted values. The raw invoice
was neither printed nor copied into evidence.

Both Alice's outbound and Bob's inbound node records report `Succeeded` for this
hash, asset and amount. The existing wallet status endpoint reports `settled`.
This supports a human-readable recipient payment prepared by the agent, authorized
through the application, submitted once through the existing RGB Harness and
ultimately settled. It does not claim a recovered raw provider/HTTP trace.

One reservation establishes one eligible submission through this wallet: the
existing execution path flushes the reservation before its sole send call,
consumes the plan, and rejects another reservation for the same payment hash.
Node success establishes that a submission did occur. Thus the two pending UI
observations are observations of one economic submission under these invariants,
not evidence of two sends. An independent request-level send counter is unavailable.
Status methods have no send capability; the original exact poll count is unavailable.

Preparation's production composition acquires once; execution, approval and status
have no discovery/acquisition calls. One acquired invoice identity is retained.
No request-level access log exists to independently count all historical acquisition
requests. The earlier non-economic preflight invoice is a different invoice and is
not this payment's acquisition.

Effective current API threshold is 1; `amount >= auto_approve_below` requires human
approval. Persisted authority is `human`, consistent with amount 5 and the application
approval event. The approval endpoint uses the pending plan ID and the wallet checks
its stored scope; model prose cannot set human authority. The consumed plan ID and
original policy evaluation trace were not persisted. The recipient/domain displayed
by the sheet are sourced from the plan's immutable provenance in the existing code.

## UI evidence reported by the user

The user reports the application sheet displayed 5 R402USD, the exact recipient
and authoritative domain, 3,000 carrier sats, available balance 500, and manual
approval required; the user approved in the PWA and later saw node-verified settlement.
These observations are corroborated by the application event, journal and node
records above. No screenshot image files were supplied with this attachment, so
this report does not claim independent inspection or preservation of those images.
No screenshots were regenerated.

## Balances

| Node | Earlier outbound RGB snapshot | Current outbound RGB | Current on-chain spendable RGB |
| --- | ---: | ---: | ---: |
| Alice | 500 | 495 | 400 |
| Bob | 100 | 105 | 0 |

The earlier snapshot is the authoritative read-only snapshot from the failed prior
attempt, not a recovered immediate-before sample for the successful run. The user
also reports 500 available on the approval sheet. Current balances were queried
independently and are consistent with the five-unit transfer. Bob's on-chain
settled balance remains zero; the received RGB is Lightning channel balance.

## Not recoverable after the fact

The running API logs to a terminal, not a retained trace file. The session API
retains user-facing events, not raw tool calls or provider requests. Therefore:

- Exact raw tool sequence, total status-query count and independent HTTP submission/
  acquisition counters are unavailable. The inferred sequence is assets → recipient
  preparation → application approval → execute → status.
- Consumed plan ID and complete in-memory Harness trace are unavailable.
- The preparation observation byte size and live wire-level absence of protocol
  internals cannot be measured retrospectively. Source inspection and regression
  tests show the production projection omits raw invoices, JRD, service paths,
  DNS/TLS, contracts/scopes and credentials; this is not a recovered wire capture.
- Provider/model is OpenAI / configured default gpt-4.1-mini, based on application
  configuration and source; no provider response metadata was retained.

The sanitized JSON and event transcript accompany this report. Historical failed
attempts and the non-economic preflight remain separate artifacts.

## Regression validation

36 recipient tests, formatting, strict workspace/all-target Clippy and the frontend
production build passed after read-only reconstruction. No application behavior
was changed.
