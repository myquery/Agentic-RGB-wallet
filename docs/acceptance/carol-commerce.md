# Carol commerce acceptance — settled

## Observed implementation run

Carol's isolated node/channel and wallet were provisioned without replacing the
Alice/Bob channel. Carol uses the current R402USD contract. Electrs had stopped;
it was restarted before funding. Starting balances are recorded in
[carol-before.json](carol-before.json): Alice 540 outbound/300 on-chain, Bob 160
outbound/0 on-chain, Carol 0/0 (Alice's new channel allocation is included).

Real OpenAI agent catalog discovery returned Coffee 5 and Sandwich 8. The first
invoice attempt exposed a pinned-node integration detail: pending inbound history
has null RGB fields. The store now checks ownership via the inbound payment hash
and carrier, while checking RGB asset/amount through the decoded invoice. The
regression test models these null history fields. Diagnosis created unpaid
invoices and one extra unpaid Coffee order; none were submitted as payments.

The first Bob request did not produce an order. Explicit catalog-first product
selection subsequently succeeded. Unknown product IDs now return a specific
correction message rather than a generic invalid-payment message. The server
still requires the exact authoritative product ID. No price substitution occurs.

Bound real-node plans reached application approval:

* Alice Coffee, 5 R402USD: hash
  `741dfc8db484370eb77f08236e93facdfafc09f5564f8925de999545ca2aa3ab`;
  [preparation transcript/plan](carol-coffee-awaiting-approval.json).
* Bob Sandwich, 8 R402USD: hash
  `e69d17632ef535be7780e5a71de48f8b5e551f828319830d08452c2cbabafd51`;
  [preparation transcript/plan](carol-sandwich-awaiting-approval.json).

Both use 3,000-sat regtest carriers and node-managed fees. The user approved each
in its application. Both sender nodes and Carol independently report Succeeded
with the expected RGB amount; both orders are paid / Settled. See
[Coffee result](carol-coffee-result.json), [Sandwich result](carol-sandwich-result.json)
and [agent receipts](carol-receipts.json). The Coffee policy snapshot records
require_approval; both durable reservations record Human authority. Each payment
has exactly one reservation and one submission event in the
[execution audit](carol-execution-audit.json), including UTC tool timestamps.
Plan IDs are the payment hash followed by `-1`; order IDs are `ord_` plus the hash.

The Sandwich approval snapshot raced the user's approval: it contains the
post-approval transcript, not a pending plan. Its capture note clarifies this.
Earlier failed preparation snapshots remain recorded.

Measured outbound R402USD balances after both purchases:

| Wallet | Before | After | On-chain before / after |
| --- | ---: | ---: | ---: |
| Alice | 540 | 535 | 300 / 300 |
| Bob | 160 | 152 | 0 / 0 |
| Carol | 0 | 13 | 0 / 0 |

No intermediate Carol=5 balance snapshot was taken. Individual authoritative
incoming records verify the 5- and 8-unit credits. Final balances are captured in
the Sandwich result above. Read-only receipt queries did not create new orders.

Successful preparation prompts, using the real OpenAI provider:

> Buy one Coffee from carol@3f0e-102-88-113-62.ngrok-free.app. Prepare the merchant order for application review now; do not ask for conversational confirmation.

> Read Carol’s catalog first, then prepare one Sandwich using its exact catalog product ID sandwich and quantity 1. Show application approval; do not pay yet.

Tool sequence: merchant_catalog → merchant_create_order → application approval →
wallet_execute_payment → merchant_order_status. Order creation internally uses
invoice validation, balance checks and the existing wallet prepare/policy path.
The final unknown-product diagnostic wording was compiled/tested after the live
purchases; the successful payment flow used the preceding running binary.

The full workspace test suite, Clippy, formatting, 29 frontend tests and UI build
passed during implementation. Added regressions cover changed price/asset/
merchant/quantity, overflow, inbound ownership, duplicate hash, exclusive order
storage, restart recovery and node-only paid transitions.

The merchant implementation uses the existing real `RgbLightningClient` and
`WalletService`/Harness. Settlement was verified on 2026-09-18 UTC.

Existing wallet-to-wallet behavior is covered by regression tests and
[historical human-recipient acceptance](recipient-agent-completed.md). No fresh
wallet-to-wallet payment was submitted in this merchant run. Historical L402
evidence also remains separate.

Reproduction steps (the wallet-to-wallet step was not rerun here):

1. Alice: `Send bob@<domain> 5 R402USD.` Verify the unchanged recipient payment
   route, application approval, same-hash Alice/Bob settlement and balances.
2. Alice: `What does Carol sell?` Expect Coffee — 5 R402USD and Sandwich — 8
   R402USD, discovered via Carol's WebFinger commerce relation.
3. Alice: `Buy the coffee from Carol.` Capture order ID, product/quantity, exact
   existing asset ID, 5-unit invoice/hash, plan ID, policy and explicit approval.
   After approving, verify one node-settled submission, Carol's +5 RGB delta,
   paid order and receipt. Query `merchant_order_status` if the conversation
   ends before returning the receipt; never request a replacement purchase.
4. Bob independently: `Buy the sandwich from Carol.` Capture the same evidence
   for 8 units; verify Bob −8 and Carol +8 after node settlement.

Capture UTC times, provider/model, exact requests/tool sequence, order/plan/hash,
policy/approval, before/after node balances and receipt status. Omit all keys,
tokens, CSRF values and payment proofs. Prior wallet-to-wallet/L402 evidence in
this directory remains historical and unchanged. No historical payment may be
relabeled as one of these merchant purchases.
