# Approved-plan continuation correction

## Observed failure and limits of the trace

For payment hash `aa756281002be5814e6f30a7f2b07986081b616914484c26fbb78e73cb80dbc5`,
the retained API log records presentation and successful human approval of the
same `hash-1` plan ID. The next model tool is wallet_execute_payment, followed by
an unknown-plan error, with no payment_execution_requested event. The old log did
not record the model's requested plan ID, so its exact value cannot be recovered.

Source tracing rules out approval consuming the wallet registry entry:

- WalletService preparation inserts the immutable plan into `plans`.
- ApprovalSheet sends the plan ID in the existing approval URL and an empty body.
- The endpoint validates that ID against session.pending, marks the session busy
  and clears the presentation slot. It does not remove the WalletService plan.
- The serialized continuation calls confirm_from_human; approve_from_human marks
  human authority and stores the scope, without removing the plan.
- Previously the model was then asked to reproduce the approved plan ID.
- The unknown-plan lookup occurs before execution logging. WalletService removes
  its registry entry only after revalidation, reservation and submission transition.

The evidence therefore points to an execution-ID mismatch at the model handoff,
not a demonstrated premature deletion or two-wallet ownership race. The exact
model argument was not retained. Mock coverage reproduces this failure mode using
a payment hash in place of its `hash-1` plan ID.

## Corrected lifecycle

Human confirmation now retains the executable plan and schedules its exact ID for
the application-owned continuation. On resumption, the gateway dispatches the
existing wallet_execute_payment operation for that stored ID before asking the
model for another response. This uses one of the existing eight steps, the same
wallet policy/approval gate, and the same Harness/reservation/send path. There is
no second sender and no execution in the approval endpoint itself.

The confirmation flag is no longer removed before wallet revalidation. After the
Harness indicates submission may have occurred, the gateway keeps a process-local
plan-ID-to-payment-hash status mapping and removes its execution confirmation.
Repeated execute requests for that ID query status only, including uncertain
submissions. Unknown/stale IDs are not remapped to some other approved plan.
After restart, this convenience mapping is gone; the durable journal still blocks
resubmission and the known payment hash remains the recovery identifier.

The application-generated continuation call is distinguishable by its
`application_execute_` call ID. It must not be described as a fresh model decision
in acceptance evidence. Automatic recipient and machine-payment authorization
semantics, wallet state format, host/origin rules and Alice/Bob isolation are unchanged.

## Validation and deployment

129 workspace tests passed, including the mock one-send continuation regression,
pre-approval rejection, repeated approval rejection, status-only execute replay,
wrong-plan rejection, recipient binding and web lifecycle tests. Formatting and
strict workspace/all-target Clippy passed. No live invoice or payment API was
called during the fix. Neither running wallet API was restarted.

Focused commands:

```bash
cargo test -p rgb402-agent approved_plan_continuation
cargo test -p rgb402-agent approved_pending_settled
cargo test -p rgb402-agent preparation_stops_before_execution
cargo test -p rgb402-agent changed_invoice_and_other_plan
cargo test -p buyer-agent --lib approval
```

Restart each API only after checking it has no unresolved active turn, using the
existing `scripts/wallet/api.sh alice` / `bob` launchers. A restart discards pending
in-memory plans but preserves reservations. The original invoice could technically
be prepared again only if still unexpired and unsubmitted, with fresh validation
and exact-plan approval. Its current expiry was not queried and it was not used.
