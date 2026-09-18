# Human BTC Lightning payments to recipient addresses

In either wallet's Agent tab, use an explicit satoshi amount, for example:

```text
Pay alice@<current-recipient-domain> 5 sats
```

Use Bob's wallet to pay Alice and Alice's wallet to pay Bob. The application
prepares a BTC plan and presents **Confirm BTC payment**, showing the address,
verified domain, exact sats, available outbound capacity and payment hash.
The user must explicitly approve. The model has no approval tool. Status and
Activity come from the node; a pending or uncertain result is never settlement.
The CLI `agent` binary presents the equivalent application-owned `[y/N]` prompt.

Sats/BTC are never substituted with RGB units. The gateway blocks RGB preparation
when the current request explicitly mentions sats, satoshis, BTC or bitcoin.
The dedicated BTC recipient tool accepts only `identifier` and `amount_sats`.
RGB transfers and the L402 machine-purchase flow keep their existing policies.
The direct invoice Send sheet remains an RGB invoice flow.

## Scope and protocol

This is an extension to the project's experimental WebFinger recipient protocol,
not general LNURL-pay/Lightning Address compatibility. It supports the project's
Alice/Bob public HTTPS service and compatible deployments on regtest only.

Each account advertises an additional relation:
`https://rgb402.example/relations/btc-invoice`, pointing to its same-origin
`/btc/invoice/alice` or `/btc/invoice/bob` endpoint. The POST body is exactly:

```json
{"subject":"acct:alice@example.com","amount_sats":5}
```

The mapped receiving node creates a BTC-only BOLT11 invoice for 5,000 msat with
one-hour expiry. It does not add RGB asset fields or the RGB carrier amount.
Only the invoice is returned. Unknown accounts, mismatched subjects, zero,
fractional, negative, overflowing amounts and extra request fields are rejected.
The public tunnel still exposes only recipient discovery and invoice acquisition.

Discovery preserves exact WebFinger subject checking, HTTPS, public DNS address
validation/pinning, no redirects, no proxy use, fixed deadlines and bounded bodies.
Acquisition locally verifies the BOLT11 signature, exact requested whole-satoshi
amount, regtest currency, freshness and absence of all RGB fields. BTC endpoints
are never inferred from RGB endpoints. No node call occurs during acquisition.
Preparation independently decodes through the wallet node and compares the result.

## Limits, persistence and recovery

The API and agent CLI enable the human BTC service with these independent defaults:

| Environment variable | Default | Meaning |
| --- | --- | --- |
| `MAX_BTC_TRANSFER_SATS` | 100 | Maximum principal per human BTC payment |
| `MAX_BTC_TRANSFER_DAILY_SATS` | 500 | UTC-day reserved principal budget |

Both must be positive integers, and the individual limit must not exceed the
daily limit. Human approval is mandatory even for 1 sat. Limits do not include
node-managed routing fees. The available balance is conservative single-channel
outbound capacity, not total on-chain BTC or receive capacity. Settings displays
these limits separately from RGB policy. L402 automatic thresholds do not apply.

The journal path is `<WALLET_STATE_PATH>.btc.jsonl`, so Alice and Bob get separate
BTC state through the existing launcher. It has a separate lock, mode-0600 file,
append-and-sync reservation before submission, and a contract digest binding the
address/domain/endpoint and complete decoded invoice. Do not delete journals or
locks to retry a payment. Cancelled plans and restarted sessions cannot reuse old
approval. Reservations survive crashes and count against the daily budget even
when submission or status is uncertain. Future-dated reservations count
conservatively if the system clock moves backwards.

Execution re-decodes and checks invoice, expiry, outbound balance, current daily
budget and the exact application authorization. It never repeats a reserved hash.
After an uncertain submission, ask for the original BTC payment status or inspect
Activity; do not request another invoice as an automatic retry. Status queries do
not send payments. Historical RGB and machine journals and digests are unchanged.

## Deploy the update

Build the UI and restart only idle wallet APIs and the recipient service. Preserve
pending reviews and in-flight actions before restarting. Use the same current
`RECIPIENT_DOMAIN`, `RECIPIENT_ASSET_ID`, and existing Alice/Bob node state:

```bash
npm --prefix apps/wallet-ui run build
cargo build -p merchant-server --bin recipient-service -p buyer-agent --bin api
# In separate terminals, after graceful shutdown of the old idle processes:
./scripts/wallet/api.sh alice
./scripts/wallet/api.sh bob
# In the recipient-service terminal with its existing public configuration:
target/debug/recipient-service
```

Keep the ngrok tunnel pointing to port 3050 and refresh both browser tabs. If Alice
uses L402, export `config/l402.env.example` before starting her API, as documented
in [L402 setup](l402.md). No provider key belongs in frontend configuration.

## Validation scope

Offline regressions cover local BTC invoice validation and rail separation,
address relation/origin/subject validation, mandatory human approval, exact plan
binding, changed invoice/balance, cancellation, duplicate execution, journal
failure, uncertain/restart recovery, independent daily limits, model-forged
approval, currency substitution, UI sats/recipient labels and direction.
The existing RGB/L402 tests remain part of the workspace suite. No live human BTC
payment is implied by those tests; live settlement requires a separate explicit
application approval.

## Read-only activation

On September 18 the idle Alice/Bob APIs and recipient service were restarted with
this implementation. Public WebFinger advertised both correct BTC endpoints; both
wallet APIs reported mandatory human approval, 100-sat individual/500-sat daily
limits, and 10,000 sats conservative outbound capacity. Alice's L402 configuration
remained enabled. No BTC transfer invoice or payment was created during activation.
See [sanitized activation evidence](acceptance/btc-recipient-activation.json).

## Small-payment channel minimums

The original Alice-funded RGB channel advertises a 3,000,000-msat minimum for
Bob → Alice. This is negotiated at channel creation, not the human wallet's
100-sat spending limit. The pinned node does not expose an API to lower that
existing minimum. The reverse direction may have a different negotiated minimum.

BTC preparation and execution now check that at least one usable channel supports
the exact amount between its negotiated outbound minimum and limit. A displayed
maximum capacity alone does not establish that a small amount is routable.
BTC execution/status receipts use authoritative application wording; a failed
node result must not be reinterpreted as a missing approval or request to pay again.

A separate Bob-funded BTC-only channel can supply the missing small-payment route.
The existing RGB channel is preserved. Review the exact proposal first:

```bash
python3 scripts/regtest/small_btc_channel.py
```

Only after approving the displayed allocation, run:

```bash
python3 scripts/regtest/small_btc_channel.py --apply
```

This opens a 100,000-sat BTC-only regtest channel from Bob to Alice, pushes zero
sats, pays the node-managed funding fee, and mines six local confirmations when
needed. It does not pay an invoice. A durable opening-attempt record prevents an
uncertain result from triggering another channel open. Existing matching channels
are reused; ambiguous matches fail closed. The final check verifies the actual
Bob → Alice minimum and maximum permit 5 sats. The opening direction matters:
the opener's 3,000-sat receive minimum still applies in the reverse direction.
The existing Alice → Bob channel remains available for small BTC payments there.

Keep the Bitcoin and Electrs services running. Electrs is the project's chain
indexer and is unrelated to an Eclair Lightning node.
