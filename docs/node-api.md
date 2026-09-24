# RGB Lightning node contract and live acceptance

No node version, Docker/regtest deployment or node credentials existed in the
repository at inspection. The Polar provider is an old simulation-era stub.
The new wallet uses a separate narrow client, leaving the old demo intact.

The adapter was checked against RGB-Tools/rgb-lightning-node revision
`e4008278c80495ea8a8514580b899e771feff872`:
[OpenAPI schema](https://github.com/RGB-Tools/rgb-lightning-node/blob/e4008278c80495ea8a8514580b899e771feff872/openapi.yaml).
Deploy this revision or verify compatibility before using another release.
This is a source contract pin, not automatic runtime version negotiation.

| Operation | HTTP API | Mapping |
| --- | --- | --- |
| List fungible demo assets | POST `/listassets` | `filter_asset_schemas: ["Nia"]`, use `nia` array |
| Asset balance | POST `/assetbalance` | `asset_id`; `offchain_outbound` is LN asset liquidity, `spendable` is on-chain |
| Decode RGB Lightning invoice | POST `/decodelninvoice` | `invoice`; require fixed `asset_id`, `asset_amount`, `amt_msat`; expiry = timestamp + expiry_sec |
| Pay RGB Lightning invoice | POST `/sendpayment` | only the unchanged `invoice`, no amount or asset overrides |
| Status | POST `/getpayment` | `payment_hash`; unwrap `payment.status` |

`Pending`, `Succeeded`, `Failed` map to Pending, Settled, Failed.
`payment_id` and `payment_hash` are separate output fields. Secrets/preimages in
responses are ignored. Errors return sanitized status/context, never raw bodies.
The HTTP client is private, has a 30-second timeout, disables redirects and uses
an optional Biscuit bearer token. Username/password are not invented as API auth;
node initialization/unlock and token provisioning are operator responsibilities.

`/decodergbinvoice` and `/sendrgb` are on-chain RGB operations. They are deliberately
outside this milestone, as are ordinary BTC Lightning payments and amountless
invoices. The client does not pretend these are RGB Lightning payments.

## Observed live acceptance

A real 5-unit R402USD payment settled on the pinned implementation through our
WalletService, including human approval and persistent reservation. Alice's
outbound RGB balance changed 500 → 495 and Bob's changed 100 → 105. No adapter
payload or status mapping changes were necessary.

Run `./scripts/regtest/demo.sh` for the reproducible environment and acceptance
flow. See [regtest.md](regtest.md) for prerequisites, exact commands, ports,
recovery behavior and troubleshooting, and
[the recorded evidence](acceptance/rgb-lightning-payment.json) for the transaction.

The setup uses `/init`, `/unlock` with nested `ldk_chain_sync` BlockSync config,
`/address`, `/createutxos`, `/issueassetnia`, `/connectpeer`, `/openchannel`,
`/listchannels`, and `/lninvoice`. Those administrative calls belong to development
scripts, never to model-callable wallet tools. The acceptance payment itself uses
our wallet CLI. The node requires a minimum RGB invoice carrier of 3,000,000 msat,
anchor-enabled RGB channels and six channel confirmations.

Authentication remains supported in the application. Only the localhost-published
regtest containers disable authentication. The scripts discard initialization
mnemonics and use isolated, gitignored data directories.
