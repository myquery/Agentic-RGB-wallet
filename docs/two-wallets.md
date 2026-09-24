# Two instances of the RGB wallet

Alice and Bob use the same `buyer-agent --bin api` executable and PWA build.
The existing sender WalletService, policy, Harness, approvals and journal format
are unchanged. Each process has its own agent, pending plans, session token and
journal lock. No account switcher or second payment implementation is involved.

## Launch

Keep the existing funded regtest nodes running. Build the frontend once:

```bash
npm --prefix apps/wallet-ui ci
npm --prefix apps/wallet-ui run build
```

In separate terminals, from the repository root:

```bash
./scripts/wallet/api.sh alice
```

```bash
./scripts/wallet/api.sh bob
```

Open Alice at http://127.0.0.1:3030 and Bob at http://127.0.0.1:3031.
If an old API/CLI holds a journal, exit it gracefully first. The journal lock is
kernel-managed: its file remains for audit, while its exclusion is released when
the owning process exits or the host reboots. Do not delete journals to retry.
Each API retains the existing OpenAI credential configuration requirement, even
when only using Receive/history; these operations do not make model requests.
Credentials remain server-side and are not added to any example or frontend file.

The launchers load the fresh asset/policy from `.var/regtest/wallet.env`, then select:

| Setting | Alice | Bob |
| --- | --- | --- |
| WALLET_API_BIND | 127.0.0.1:3030 | 127.0.0.1:3031 |
| WALLET_NAME | Alice Wallet | Bob Wallet |
| RGB_NODE_URL | http://127.0.0.1:3101 | http://127.0.0.1:3102 |
| WALLET_STATE_PATH | .var/regtest/wallet-state.json | .var/regtest/bob-wallet-state.json |
| MACHINE_STATE_PATH | .var/regtest/machine-state.jsonl | .var/regtest/bob-machine-state.jsonl |

All paths are exported as absolute paths. Direct API launches can use these same
variables with independent policy settings. Never point two wallets at one journal.
A duplicate journal is refused by the existing kernel-held lock rather than silently shared.
API binds are restricted to IPv4 loopback and nonzero ports. Host validation and
same-origin checks use the configured port; Bob rejects Alice's origin and vice
versa. The existing Alice Vite origins on port 5173 remain supported only by port
3030. Use production builds for the two-instance demo; no Vite changes are required.
The API returns the display name in `/api/wallet`; it is not a wallet identity proof.

## Receive and history

`RgbNode` now has typed `create_invoice` and `list_payments` capabilities. The real
client wraps POST `/lninvoice` and GET `/listpayments`. Narrow request/result/history
types live in `rgb.rs`; history deliberately does not retain preimages, descriptions
or peer identities. Existing mock/custom nodes can report unsupported capabilities.

POST `/api/invoice` requires the existing session CSRF token and accepts only:

```json
{"asset_id":"rgb:...","amount":"5"}
```

The amount is an unsigned positive integer **base-unit string**, preserving u64
precision through JavaScript. The asset must appear in this node's asset list.
The application uses a fixed demo carrier of 3,000,000 msat and expiry of 3,600
seconds. The shared adapter additionally supports optional description/hash fields;
the minimal UI does not expose them. Invoice creation does not prepare, approve,
reserve or send. Failed requests are not retried automatically.

Receive lets the user select an asset, enter base units and generate/copy the invoice.
It does not implement QR codes. Node-created invoices may appear in history as
pending before any payment arrives; pending is not evidence of receipt.

GET `/api/activity` preserves outgoing journal-backed entries and their uncertain
status handling, then merges native RGB history by hash. It adds native sent and
received entries and leaves BTC machine-purchase activity on its existing path.
Node-history errors are reported, not disguised as empty history. Received rows
show a plus sign and no invented sender identity. The node's payee key is not a
payer identity. Timestamps are creation times, not asserted settlement times.

Home labels local channel RGB allocation “Lightning balance” and shows on-chain
spendable RGB separately. It does not sum overlapping accounting fields or promise
routing availability. No inbound-capacity or combined-total display was added.

The public recipient-service remains a separate restricted front door connected
to Bob's node. Do not tunnel port 3031 or 3102. Its protocol and demo restrictions
are unchanged; this milestone does not migrate that service into the wallet API.

## Acceptance A: existing settled payment, no new economic action

1. Open both wallet origins and confirm their distinct wallet names.
2. In Activity, find hash
   `8f14fd52544defc8113a409c39f5068b2eae6a7b5846c0302637ac38275b8562`.
3. Alice shows Sent, 5 R402USD, Settled; Bob shows Received, 5 R402USD, Settled.
4. Confirm one entry per wallet and current balances through each Home screen.

The live API check passed with Alice 495 outbound/400 on-chain and Bob 105
outbound/0 on-chain. Distinct CSRF tokens and cross-wallet origin rejection were
verified without saving tokens. See `acceptance/two-wallet-history.json`.
No invoice or payment was created for this acceptance.

## Acceptance B: only after a separate fresh-payment request

1. Record fresh balances on both sides.
2. Bob: Receive → R402USD → amount 5 → Create invoice → Copy invoice.
3. Alice: Send opens the existing agent composer; paste Bob's invoice with a
   request to pay it. Review exact asset, amount, carrier and policy.
4. Approve once through Alice's existing application sheet if required.
5. Wait for node-verified settlement; if uncertain, query status rather than
   creating another invoice/plan/payment.
6. Verify one same-hash sent/received entry across both wallets and independently
   query the corresponding five-unit change in Lightning balances.

This fresh payment was not performed during implementation. Deterministic
non-agent Send remains a follow-up; no second sender pipeline was introduced.

## Receiving addresses

Receive now offers Copy address above the manual invoice form. The launchers use
`RECIPIENT_DOMAIN` or the public `.var/regtest/recipient-domain` file to configure
`alice@<domain>` and `bob@<domain>` independently. The restricted public service
maps each address to its own node. No automatic Lightning-peer/contact discovery
is implied; see [recipient service configuration](recipient-service.md).
