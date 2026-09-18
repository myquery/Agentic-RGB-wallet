# Public recipient demo service

The existing merchant-server package now has a `recipient-service` binary with
only WebFinger and account-specific `/rgb/invoice/alice` and `/rgb/invoice/bob` routes. It binds to `127.0.0.1:3050`.
The public identities are explicitly mapped: `alice@<domain>` to Alice on port
3101 and `bob@<domain>` to Bob on port 3102. The alias names the receiving demo wallet, not a claim about a person's identity.

The service accepts only the configured asset and positive integer base
units (u64), with an exact subject match and no extra request fields. It requests a
regtest RGB Lightning invoice from the mapped node's loopback `/lninvoice`, using 3,000,000
carrier msat and a 3,600-second expiry. Responses contain only the invoice;
upstream failures become generic HTTP 502 responses. Request bodies are limited
to 2 KiB, node calls have an eight-second timeout and no redirects or proxy use.
This deployment uses the existing local node with authentication disabled; it
neither reads nor forwards node credentials. Do not point the tunnel at port 3102.

Build from the repository root:

```bash
cargo build -p merchant-server --bin recipient-service -p buyer-agent --bin recipient-preflight
```

Start the HTTPS tunnel in a separate terminal. The installed Snap's default
configuration path was missing; the existing legacy config was valid:

```bash
ngrok http 127.0.0.1:3050 --config "$HOME/.ngrok2/ngrok.yml" --inspect=false
```

Use the actual assigned HTTPS hostname, without a scheme or path, below:

```bash
export RECIPIENT_DOMAIN='<assigned-ngrok-hostname>'
export RECIPIENT_ASSET_ID='rgb:KigwNgFx-bh7pHa~-Q7gi49D-ncmlxS5-~~44UbJ-g0Ienok'
target/debug/recipient-service
```

Run the non-economic preflight in another terminal with the same two variables:

```bash
export RECIPIENT_IDENTIFIER="bob@$RECIPIENT_DOMAIN"
target/debug/recipient-preflight
```

This command calls production `resolve_recipient`, constructs the immutable
acquisition contract, then calls production `acquire_recipient_invoice`. It uses
the normal public DNS pinning, HTTPS certificate validation, no-redirect policy
and local RGB-aware invoice parser. It never opens wallet state, prepares a plan,
authorizes, reserves, executes or polls a payment. Each invocation requests a new
invoice; its output omits the raw invoice and protocol response bodies.

The service and tunnel must remain running. An assigned tunnel hostname may
change on restart: update RECIPIENT_DOMAIN and RECIPIENT_IDENTIFIER together.
This is a narrowly scoped public regtest demo, not a general invoice hosting
service. The invoice service does not grant spending authority; sender wallet policy and
channel/routing constraints still govern payment.

The only recipient validation correction adds `~` to the existing preliminary
asset alphabet. A regression builds and locally validates a signed invoice with
the actual fresh asset ID. Exact decoded asset matching and all other checks
remain in force.

See [live preflight evidence](acceptance/recipient-service-preflight.json).

## Copyable wallet addresses

The wallet API accepts optional `WALLET_RECIPIENT_ADDRESS` and exposes it as
`recipient_address` in `/api/wallet`. Receive displays this configured identity
with Copy address; absent configuration shows the invoice fallback. The display
is not a claim of automatic peer discovery or independently authenticated human
identity. Production sender discovery still verifies WebFinger on every new
recipient preparation.

The Alice/Bob launch script derives each account from its role and reads the
public hostname from `RECIPIENT_DOMAIN`, or `.var/regtest/recipient-domain` when
that variable is unset. That file contains only the hostname, never credentials.
If ngrok changes domains, update this file, restart the recipient service with
that same domain and restart idle wallet APIs. Never rewrite historical plans.

Both address mappings were checked over public HTTPS without issuing invoices;
see [address verification](acceptance/wallet-addresses.json). Historical evidence
from when the demo alias `alice@…` pointed to Bob remains unchanged. From this
configuration onward, `alice@…` points to Alice, and `bob@…` points to Bob.
The service accepts positive integer base-unit amounts of the configured demo asset.
It forwards the exact requested amount without rounding or substitution. Sender
spending limits and human approval requirements remain unchanged.

## BTC recipient transfers

The same account mappings now also advertise a separate BTC invoice relation and
`/btc/invoice/:account` endpoint. See [human BTC recipient payments](btc-recipient-payments.md)
for protocol, mandatory approval, separate limits and restart instructions. RGB
invoice requests and their carrier amount remain unchanged.
