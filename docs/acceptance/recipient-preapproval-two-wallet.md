# Human-address preparation in the two-wallet application

No new integration seam was missing. The repository already has the strict
`wallet_prepare_recipient_payment {identifier, asset_id, amount}` tool, production
WebFinger resolver, controlled acquisition and RGB-aware local validation,
recipient bridge, immutable provenance/scope binding, and the existing approval
sheet. The corrected application-owned approved-plan continuation remains intact.
No application code, tool schema or payment architecture was changed for this task.

The actual configured identity is
`alice@3f0e-102-88-113-62.ngrok-free.app`, backed by Bob's node. The alias name does
not identify the sender. The public front door exposes only its existing two
recipient routes, never Bob's application API or node API.

The submitted request was:

> Pay alice@3f0e-102-88-113-62.ngrok-free.app 5 R402USD.

The current application session records asset identification and preparation.
The authoritative pending plan carries the exact canonical asset
`rgb:KigwNgFx-bh7pHa~-Q7gi49D-ncmlxS5-~~44UbJ-g0Ienok`, amount 5, verified recipient
and domain, Regtest network, 3,000,000-msat carrier, available balance 490 and
`require_approval`. The effective threshold is 1, with amount >= threshold
requiring approval. The code path establishes discovery, invoice acquisition,
local validation and exact bridge matching before that plan can exist. Raw model
wire calls were not retained; the report does not invent a captured tool trace.

STOPPED before approval. No send/status operation was initiated for this new
payment. The reservation journal was byte-for-byte unchanged. Independently
queried before/after balances were identical: Alice 490 outbound/400 on-chain,
Bob 110 outbound/0 on-chain. The JSON evidence preserves the plan ID, hash and
expiry without the raw invoice or session credentials.

Existing regression coverage checks subject matching, allowed origins, acquisition,
asset/amount/network/expiry mismatches, provenance/approval binding, no arbitrary
URL tools, duplicate/uncertain handling and the application continuation. The
recipient suite (36 tests) and focused approved-plan continuation regression (one
test) pass; formatting and strict workspace/all-target Clippy pass. No duplicate
implementation/tests were added for capabilities already covered.

## Existing deployment commands

In separate terminals, with the configured funded nodes already running:

```bash
./scripts/wallet/api.sh alice
./scripts/wallet/api.sh bob
```

The public recipient service and tunnel were already running; do not start a
second copy. To restore them if stopped, use separate terminals:

```bash
ngrok http 127.0.0.1:3050 --config "$HOME/.ngrok2/ngrok.yml" --inspect=false
```

Use the actual assigned hostname (which may change) for the service:

```bash
export RECIPIENT_DOMAIN='3f0e-102-88-113-62.ngrok-free.app'
export RECIPIENT_ASSET_ID='rgb:KigwNgFx-bh7pHa~-Q7gi49D-ncmlxS5-~~44UbJ-g0Ienok'
cargo run -p merchant-server --bin recipient-service
```

The existing public demo service intentionally accepts only its configured asset
and five base units. The wallet recipient path itself is asset-ID/amount generic.
The public hostname and current invoice expire independently; a later acceptance
must check freshness. No expired invoice will be silently replaced or retried.
The PWA currently gives a compact preparation event and authoritative approval
sheet, rather than separate streaming discovery/acquisition substep messages.
No UI redesign or additional low-level model tool was needed.

A pending plan now exists in Alice's application. Any economic acceptance requires
separate explicit authorization and the application's approval mechanism. This
report is not payment authorization.
