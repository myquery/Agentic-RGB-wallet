# Recipient Invoice Acquisition v1

This experimental project protocol obtains an invoice candidate and validates it
against explicit application-created intent. It grants no spending authority and
is not an RGB, WebFinger, or Internet standard. There is no interactive-agent
tool, UI, payment preparation, balance query, approval, reservation or submission.

```text
DNS → HTTPS → WebFinger discovery → validated invoice service
    → acquisition POST → local deterministic RGB invoice validation
    → validated candidate → future Harness entry
```

## Inspection and decoder choice

The existing wallet maps the node's `/decodelninvoice` response into
`rgb402_core::wallet::PaymentRequest`: asset ID, integer RGB amount, fixed carrier
msat, payment hash, network and timestamp-plus-expiry. Regtest demo issuance uses
`/lninvoice` with `asset_id`, `asset_amount`, `amt_msat` and `expiry_sec`, returning
`{ "invoice": "..." }`. On-chain RGB invoices are outside this feature.

Acquisition must make no node calls. It therefore uses the same RGB-aware
`lightning-invoice` parser as the pinned node, locally, pinned to RGB-Tools
rust-lightning revision `e2a0b8e24dc919ccef289f103fd1f8e1974fcc1d`. This fork checks
BOLT11 structure, required fields, feature bits, signature and millisatoshi
precision. Its RGB getters select the first field, so the application additionally
requires exactly one RGB asset and one RGB amount field, and rejects duplicate
expiry fields. The wallet's existing decoder and Harness remain unchanged.

The upstream fork pulls in `rgb-lib` and its substantial transitive dependency
graph even for parsing. `Cargo.lock` retains `secp256k1 0.32.0-beta.2` and its
registry checksum from the pinned node lockfile because that required version is
now yanked. Preserve the lockfile for reproducible builds; updating this stack
requires reviewing upstream compatibility, not silently replacing the RGB parser.
The agent and buyer application Rust requirement is raised to 1.88 to reflect
this dependency; the other crates retain their existing source-version metadata;
validation was performed with Rust 1.98.1, not a separate minimum-version run.

## Immutable contract and API

```rust,ignore
use rgb402_agent::recipient::acquisition::{
    RecipientInvoiceContract, acquire_recipient_invoice,
};
let contract = RecipientInvoiceContract::new(
    resolved_recipient, requested_asset, 5, 3_000_000,
)?;
let candidate = acquire_recipient_invoice(&contract).await?;
assert!(candidate.matches_contract(&contract));
```

The constructor accepts a validated discovery descriptor, the existing `AssetId`,
a positive `u64` asset-base-unit amount and a positive explicit carrier-msat
ceiling. Regtest is fixed for v1. The asset string is bounded to 256 ASCII
characters from the current RGB identifier alphabet. This preliminary input
check is not proof of a valid RGB contract; the locally parsed invoice supplies
the canonical ID that must match it exactly.

Private fields and read-only accessors prevent changing the contract. There is
no `Deserialize` constructor for either contracts or successful results.
`contract_digest` is SHA-256 over a fixed-order JSON tuple containing the version
`rgb402-recipient-invoice-v1`, complete discovery descriptor (identifier, subject,
authoritative domain, service kind and URL), asset, amount, carrier ceiling and
network. Changing any of these inputs changes the digest. The digest identifies
intent, not a unique acquisition attempt; repeating the same intent has the same
digest. It does not authorize spending or provide invoice replay protection.

## Wire protocol

POST once to the exact validated service URL. Use `Content-Type: application/json`
and `Accept: application/json`. For example:

```http
POST https://example.com/rgb/invoice/alice
Content-Type: application/json
Accept: application/json

{"subject":"acct:alice@example.com","asset_id":"rgb:...","asset_amount":5}
```

```http
HTTP/1.1 200 OK
Content-Type: application/json

{"invoice":"lnbcrt..."}
```

The request uses the existing node's asset field names and integer base units;
`subject` binds the service request to the discovered account. Service implementations
must parse the JSON amount losslessly as a `u64`, not through floating point. The service chooses
its invoice expiry and carrier amount, but the application validates expiry and
enforces the user's carrier ceiling. The service cannot override intent. Only
HTTP 200 is successful; media-type parameters are accepted. The response must
contain exactly one string `invoice`, with no extra or duplicate fields. Invoice
strings are limited to 12 KiB and complete response bodies to 16 KiB. There are no
query credentials, authentication headers, retries, echoed economic overrides or
remote validation claims. The protocol deliberately adds no representation of
asset precision, floating-point amounts, payment status or approval.

## Transport protections

Discovery and acquisition share `recipient/https.rs`. Every request performs a
new DNS lookup, rejects the entire answer set if any address violates the
existing public-network policy, and pins the checked addresses in the actual
HTTPS client. Hostname/SNI and normal TLS certificate verification remain.
System proxies, redirects and retries are disabled. The discovery descriptor is
not a permanent network-access exemption. L402's distinct loopback policy is
unchanged.

One ten-second deadline covers DNS, TLS, request and response, and validation.
The bounded local parser runs synchronously; elapsed time is checked afterward
so an over-deadline result cannot succeed, although CPU parsing cannot be
preempted midway. Content length and incremental body bytes are bounded. The
shared client has no node, model, wallet or unrelated application credentials.

## Validation and trust boundaries

1. **Domain transport:** DNS locates the host; HTTPS authenticates transport to
   the domain. This is not a real-world identity assertion.
2. **Account discovery:** exact WebFinger subject and supported relation bind the
   identifier to a same-origin service controlled by that domain.
3. **Invoice provenance:** one HTTPS response supplies an invoice from that
   service. Its exact UTF-8 bytes are hashed with SHA-256 into `invoice_id`,
   distinct from its Lightning `payment_hash`. The result retains the complete
   original contract and digest. This is application provenance, not a detached
   cryptographic domain attestation.
4. **Economic intent:** local signature/semantic parsing, one RGB asset/amount,
   exact asset and amount match, positive fixed carrier within the explicit
   ceiling, regtest network, checked expiry arithmetic, unexpired-at-validation
   and no future timestamp are required. Validation uses local UTC after the
   response arrives. Integer amounts and the original invoice enter the existing
   `PaymentRequest` representation.
5. **Economic authorization:** none is granted. The future Harness must recheck
   the invoice, expiry, balance, policy and bound application authorization using
   its unchanged reservation and settlement rules.

BOLT11's signature authenticates its signing key and signed fields; no trusted
mapping between that key and `alice@example.com` exists here. Recipient ownership,
asset authenticity/acceptance, sufficient balance, route availability, prior
payment and settlement are not established by acquisition. The domain may return
an invoice for another node, and this layer cannot prove otherwise.

The signed timestamp and expiry allow deterministic time checks, not proof of
new generation. An unexpired invoice can be replayed, and its issuer chooses the
timestamp. There is no nonce/challenge binding or acquisition replay database.
The model sees `freshness: unexpired_at_validation_only`; it must never be told
that HTTPS acquisition cryptographically proves a fresh invoice.

## Application result and model projection

The successful `ValidatedRecipientInvoice` retains the immutable contract,
SHA-256 invoice ID, decoded `PaymentRequest`, signed issuance time and local
validation time. The raw invoice is stored once inside that request, accessible
only through the application's read-only getter. No automatic serialization or
Debug output exposes it. `matches_contract` compares the complete original contract, including its digest; the
result cannot be rebound through setters or deserialization.

The model projection has a 2 KiB budget, below the existing Harness observation
budget, with bounded input fields and fixed error codes:

```json
{
  "status": "invoice_validated",
  "identifier": "alice@example.com",
  "service_origin": "example.com",
  "asset": "rgb:...",
  "amount": "5",
  "invoice_id": "<sha256>",
  "contract_digest": "<sha256>",
  "contract_match": true,
  "freshness": "unexpired_at_validation_only",
  "payment_authorized": false,
  "next_allowed_actions": [],
  "future_step": "submit_to_payment_harness"
}
```

Neither the raw invoice nor endpoint path, DNS answers, response bodies, TLS
internals, credentials or underlying error strings appear. Fixed failure codes
cover unavailable/security/malformed/unsupported/contract-rejected categories,
including asset/amount/network/carrier mismatch, expiry, future timestamp,
HTTPS failure, body size, media type and total timeout.

## Offline checks

`cargo test -p rgb402-agent recipient --lib` includes real signed RGB invoice
fixtures generated with deterministic test-only keys and parsed by the production
local decoder. It covers valid acquisition, changed intent, malformed/checksum
failure, missing/ambiguous fields, time constraints and replay limitations,
structured HTTP failures, projection bounds and total timeout. Shared DNS tests
exercise new lookup after discovery, mixed safe/unsafe answers, and preservation
of the validated address set supplied to the client override. No real node,
wallet, model or Internet service is required; no spending capability is supplied
to acquisition. These are offline unit/transport-seam checks, not a claim of live
public WebFinger interoperability.

Validation: all **107 workspace Rust tests** (including 11 new acquisition/shared
transport tests), **16 frontend tests**, formatting, strict workspace Clippy,
whitespace checks and the production frontend build pass. All recipient tests
run offline; no live invoice service, node or model request was made.

The subsequent [application-only Harness bridge](recipient-harness-bridge.md)
now accepts this validated result plus the original contract. Acquisition itself
still neither prepares nor authorizes payment. Contract matching compares all
immutable fields, including the digest, rather than accepting digest equality
alone.
