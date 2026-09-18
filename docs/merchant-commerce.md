# Luma wallet merchant capability (Carol)

Carol is a normal wallet/node with independent state, plus an optional store
capability. Alice/Bob remain ordinary wallets. This adds no custody, settlement
engine or payment rail. Coffee costs 5 and Sandwich 8 base units of the existing
precision-zero R402USD contract configured by `RECIPIENT_ASSET_ID`.

`rgb402-core::merchant` defines profile, product and order data. The
`rgb402-merchant::store` module creates invoices through the existing `RgbNode`
adapter. It computes price × quantity with checked arithmetic (quantity 1–100),
rejects extra buyer fields, decodes each invoice, and verifies asset, amount,
network, expiry, carrier and ownership in Carol's inbound node history. Orders
are append-and-sync persisted in `.var/regtest/carol-orders.jsonl`; the exclusive
lock prevents two store processes. Status uses the same node payment status API.
Only node settlement marks an order paid. A status failure is an error, not paid.

Public endpoints, proxied through the existing recipient service on 3050:

* GET `/commerce/v1/merchant`: profile, enabled/public flags, accepted assets,
  catalog and order URLs. Disabled stores return 404.
* GET `/commerce/v1/catalog`: available server-priced products; requires both
  merchant mode and public catalog.
* POST `/commerce/v1/orders`: only `product_id` and integer `quantity`; creates
  a fresh invoice/order, never sends a payment.
* GET `/commerce/v1/orders/{id}`: commercial receipt and authoritative status.

Carol's WebFinger advertises ordinary RGB/BTC invoice endpoints and the
`https://rgb402.example/relations/commerce` descriptor relation when active.
Catalog content is not embedded in WebFinger. Alice/Bob discovery is unchanged.
The public proxy never exposes owner settings/order-list endpoints, node
APIs or wallet approval routes. An unguessable order ID is a receipt capability;
keep it private if the order's contents are sensitive. This regtest demo is not
a production authenticated store.

Agent tools: `merchant_catalog(recipient)`,
`merchant_create_order(recipient, product_id, quantity)`, and
`merchant_order_status(order_id)`. `Carol` resolves using the configured wallet
recipient domain; an explicit `carol@domain` also works. The client reuses public
DNS pinning, HTTPS, same-origin discovery, no redirects/proxy and bounded
responses/deadlines. Catalog names are untrusted data. The app re-fetches the
catalog when ordering, independently checks product/quantity/price/asset against
the returned order and verifies the invoice through its existing node decoder.

The order becomes a normal direct-invoice `WalletService` plan; this preserves
the existing Harness, balance/policy checks, immutable invoice, durable spend
reservation and duplicate protection. It always uses application confirmation,
including below-threshold policy outcomes; no model approval is added. Product,
merchant and order ID are displayed with the plan. Execution and authoritative
status are unchanged. The order receipt is separately checked against the
sender's node settlement. The eight-step model limit remains unchanged.

The order service persists receipts across restart. An uncertain invoice-create
response may leave an unpaid invoice/order; it cannot itself spend. A wallet
restart invalidates approvals, and the agent's in-memory order lookup is lost;
the merchant retains the receipt and the wallet retains its ordinary payment
reservation. Never create replacement payments to recover an uncertain payment.
Owner Merchant UI lives in each wallet's Settings section; only the owner UI can
change settings, never model tools.

## Setup with preserved Alice/Bob state

```bash
python3 scripts/regtest/carol.py          # review allocation
python3 scripts/regtest/carol.py --apply  # add isolated node and channel
cargo build -p merchant-server --bin carol-store --bin recipient-service -p buyer-agent --bin api
npm --prefix apps/wallet-ui run build
```

Carol uses node 3103, peer 19737, wallet 3032, store 3051. The optional Compose
override `.var/regtest/carol-compose.json` joins the existing regtest network.
Provisioning funds Carol with 1 mined regtest BTC, allocates 100 existing on-chain
R402USD and 100,000 sats from Alice to a new RGB channel, pushes zero RGB and
10,000 sats. Existing Alice/Bob channels are preserved. Attempt markers stop
uncertain funding/channel opens from being repeated. No new RGB asset is issued.
Bob's purchase can route through Alice; fees/minimums still apply.

## Owner setup and restart

Each wallet now hosts its optional store in the wallet API process. In Settings →
Your store, turn on merchant mode, name the store, select accepted wallet assets,
add products with integer base-unit prices, choose Publish catalog, and Save store.
Alice/Bob start disabled with no products; Carol retains her seeded demo catalog.
Saved settings override these defaults on restart. No payment or model call occurs
when saving settings. Disabling prevents new orders/discovery but does not cancel
already issued invoices or erase receipts.

Build and start from the repository root, in separate terminals:

```bash
cargo build -p buyer-agent --bin api -p merchant-server --bin recipient-service
npm --prefix apps/wallet-ui run build
./scripts/wallet/api.sh alice
# separate terminal
./scripts/wallet/api.sh bob
# separate terminal
./scripts/wallet/api.sh carol
```

Open Alice at 3030, Bob at 3031, Carol at 3032 on http://127.0.0.1.
The scripts configure public-only loopback merchant listeners at 3053, 3052 and
3051 respectively. A configured recipient domain and a healthy wallet node with
an RGB asset are needed at startup. Keep the existing recipient service on 3050
and ngrok forwarding to 3050. Restart the recipient service with its existing
environment to load the new routing (including CAROL_WALLET_ENABLED=true).

**Upgrade from the standalone Carol demo:** gracefully stop the idle
`carol-store` process with Ctrl-C before starting the upgraded Carol API. Both
use the same order journal and port; never run them together or delete live locks.
The standalone binary is retained for historical demo compatibility, but owner
editing requires the integrated wallet API. Stop/restart idle wallet APIs with
Ctrl-C, preserving all wallet and order files. Do not rerun provisioning/reset.

Owner writes use `/api/merchant` POST behind the existing wallet Host, Origin and
session CSRF guard. No owner endpoint exists on the public merchant listener or
recipient proxy. Identity, node, endpoint URLs and order/payment records are not
editable fields. Accepted assets must exist on that wallet's node; product IDs
must be unique and prices positive u64 integers. Settings are atomically persisted
and synced alongside each isolated order journal. Existing orders retain their
product/price snapshots after edits and remain queryable when the store is off.

Public Bob routes are `/commerce/v1/wallets/bob/{merchant,catalog,orders}`;
Alice uses `alice` in that path. Carol's original `/commerce/v1/...` URLs are
preserved. WebFinger advertises only that account's enabled store. Agent aliases
Alice, Bob and Carol resolve on the configured wallet domain, with strict
account-scoped descriptor/catalog/order validation. Ask another wallet “What does
Bob sell?” then request a published product. Buying still requires the unchanged
bound application approval and node settlement path.

This update was tested offline for owner-CSRF enforcement, absence of public
management routes, settings persistence, disabled-store behavior, unchanged old
orders, asset/price validation and UI setup without model/payment calls. No new
live merchant payment is implied by these tests.

## Acceptance

See [merchant acceptance](acceptance/carol-commerce.md). Offline tests do not
establish real payment completion; the required purchases need separate explicit
application approvals and real node settlement. Do not mark this milestone
complete from a mocked transfer or mere order creation.
