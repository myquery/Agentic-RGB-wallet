# Human-operated final UI acceptance

Only the human operator executes these flows. Follow the [runbook](final-demo-runbook.md)
and save a preflight/snapshot first. Confirm Alice 3030 and Bob 3031 names and
addresses, regtest, actual asset ID, and policy in Settings. Use the current
configured domain, not a hostname copied from historical evidence.

## Evidence procedure for every scenario

Save `<scenario>-before.json` and `<scenario>-after.json` with the snapshot helper.
Use [the evidence template](acceptance/final/scenario-template.md) for exact request,
recipient/resource, discovery, asset/amount, bound plan ID, invoice hash/expiry,
policy decision, required/actual approval, Alice result, Bob result, balances,
activity and authoritative final status. Capture the app's approval/result screen.
Record provider/model and UTC time. Raw RGB/BTC invoices may be captured from the
UI if needed; never copy raw L402 challenges, headers, macaroon, preimage, session
CSRF, bearer tokens, keys or seed material. Omit invoice data unavailable in the UI
and say so rather than guessing. Capture discovery from preflight and the agent's
recipient observations; that is not proof of general Lightning Address support.

For exactly-one evidence compare the before/after payment-hash sets in both
activities and match the same settled hash. Record the number of application
submission events from sanitized audit output if available. One hash alone
proves one observed payment, not the number of HTTP attempts; if submission
events were not captured, label exactly-one-submission evidence incomplete.
Never export entire journals: L402 state contains sensitive authorization material.
During pending/uncertain results query the original status; do not repeat a new
payment request. Never substitute an agent's prose for node settlement.

## A — RGB recipient payment

In Alice → Agent:

```text
Pay bob@<configured-domain> 5 R402USD.
```

Expect WebFinger discovery and invoice acquisition, exact 5-unit current R402USD
decode, balance/policy check and an immutable plan. At the application approval
sheet confirm Bob's address, asset, amount and 3,000-sat carrier. Approve once.
If current policy differs from the expected manual boundary, stop and document it.
Wait for node-confirmed Settled. Alice RGB outbound decreases by 5, Bob increases
by 5; BTC carrier/fees are separate. Both activities identify the same hash.

## B — native BTC recipient payment

In Alice → Agent:

```text
Pay bob@<configured-domain> 10 sats.
```

Expect BTC discovery, BTC-only invoice, 10 sats, independent BTC policy and
**Confirm BTC payment**. Approve once. Expect authoritative settlement and
matching Alice sent/Bob received activity. RGB units must not move. Capture
channel balances; principal is 10 sats and node-managed fees may affect debit.
If refused before submission, capture the exact policy/route failure and stop.

## C — low-value machine commerce

In Alice → Agent:

```text
Get the premium report.
```

Expected resource: `http://127.0.0.1:3040/premium/report`, 3 sats. Under the shipped
inclusive 10-sat machine automatic threshold, expect automatic policy approval,
settled BTC and verified authenticated HTTP 200 resource acquisition. No human
approval prompt should appear. Record the receipt's policy, payment hash and
resource status; Bob is the merchant receiving node.

**Existing state caveat:** if this URL is already reserved/purchased, this request
must reuse its original payment, not submit a fresh one. Mark C as recovery/reuse
with zero new submissions, and reference historical fresh-purchase evidence.
If credentials have expired, `paid_resource_unavailable` is the honest result;
mark this scenario blocked. Do not delete state, change the URL, or pay again to
make the demo green. A new-purchase requirement on this already-used URL needs
an explicitly agreed separate environment, outside this freeze task.

## D — repeat report

Repeat exactly `Get the premium report.` in the same Alice wallet. Expect reuse
of C's stored payment and the same hash, resource acquisition if credentials
remain valid, no new approval and zero new payment/reservation. Compare activities
and balances; unrelated activity must be excluded explicitly. If credentials have
expired, no new payment is still required, but resource-success acceptance fails.

## E — reject above-threshold report

```text
Get the extended premium report at http://127.0.0.1:3040/premium/extended.
```

Expected price 50 sats; above automatic 10 but within maximum 100. Expect
application approval, then choose **Cancel**. Do not approve. Capture cancellation,
no new outgoing payment hash, unchanged balances and no completed purchase.
An invoice/plan may exist on Bob without any paid transfer; distinguish incoming
unpaid invoices from outgoing economic submissions. A cancelled plan must not
produce a new spend reservation. If a durable reservation already exists for this
URL, mark E blocked rather than erasing it. Proving no new reservation requires a
sanitized operator comparison of reservation counts, not merely an empty UI.

Conclude only with scenarios actually observed. Historical evidence does not
turn an unexecuted final acceptance into a pass.
