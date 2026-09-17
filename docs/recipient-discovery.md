# Recipient discovery

`rgb402_agent::recipient::resolve_recipient("alice@example.com").await` is a
standalone, non-economic preflight API. It does not call the wallet, node, model,
invoice endpoint or Harness, and does not persist JRD documents. No new tool is
advertised to the interactive agent yet. `DiscoveryResult::model_observation()`
provides the bounded projection for subsequent gateway integration.

```text
human identifier → WebFinger → validated service descriptor
                              ↓ future invoice acquisition
                         fresh RGB Lightning invoice
                              ↓ validate requested asset/amount
                         unchanged economic Harness v1
```

## Application WebFinger profile

The transport follows [WebFinger RFC 7033](https://www.rfc-editor.org/rfc/rfc7033.html):
HTTPS GET to the domain's `/.well-known/webfinger`, one URL-encoded `resource`
parameter, and `Accept: application/jrd+json`.

```http
GET https://example.com/.well-known/webfinger?resource=acct%3Aalice%40example.com
Accept: application/jrd+json
```

```json
{
  "subject": "acct:alice@example.com",
  "links": [{
    "rel": "https://rgb402.example/relations/rgb-invoice",
    "href": "https://example.com/rgb/invoice/alice"
  }]
}
```

`https://rgb402.example/relations/rgb-invoice` is our experimental application
relation URI, using the reserved `.example` namespace. It is not a registered RGB
or WebFinger standard, deployed website, or endpoint the resolver fetches. A
compatible service must advertise this exact relation. Stabilizing a public
relation namespace and the invoice acquisition protocol is future work.

This deliberately narrow profile accepts ASCII `name@domain` only. Names are
case-sensitive, at most 64 characters, using letters, digits, dot, underscore,
plus and hyphen; leading/trailing/consecutive dots are rejected. DNS names are
case-insensitive and normalized to lowercase, with ordinary label/length rules.
Unicode names, IP literals, single-label names and local-use suffixes are not
supported. ASCII punycode DNS names can be used. Inputs are not silently trimmed.

Successful responses must be HTTP 200 with `application/jrd+json` (optional
parameters allowed), an exact canonical `acct:name@domain` subject, and exactly
one supported relation with an absolute HTTPS `href`. Unknown JRD members and
unrelated relations are ignored. RFC 7033 permits broader subject aliasing; this
application deliberately rejects it to preserve explicit account binding.
Duplicate supported relations are rejected even when their URLs match.

The advertised service must use the same DNS host and port 443. Userinfo, query,
fragment, whitespace, control characters and backslashes are rejected; URLs are
limited to 2,048 bytes. Endpoint paths are retained only in the application
result, not exposed in model observations. Discovery does not fetch the endpoint
or define its invoice-request method/payload.

## Network policy and trust

The fixed v1 policy is public-network HTTPS only. DNS answers are checked before
any connection, and the entire result is rejected if any address is disallowed.
Private, loopback, link-local, multicast, documentation and special-use ranges
are denied; IPv6 is conservatively restricted to native global unicast excluding
transition and special-purpose ranges. Validated addresses are pinned in the
per-request client while preserving hostname/SNI and certificate verification,
preventing a second DNS lookup from redirecting the connection to a private IP.
Address exclusions were checked against the [IANA IPv4](https://www.iana.org/assignments/iana-ipv4-special-registry/)
and [IPv6](https://www.iana.org/assignments/iana-ipv6-special-registry/) special-purpose registries; keep this conservative policy reviewed as allocations change.
System proxies are disabled. All redirects are rejected, including same-origin
redirects. There is no HTTP fallback or application retry.

One ten-second deadline covers DNS, connection, TLS and body reading. Bodies are
limited to 16 KiB both by advertised content length and accumulated chunks. The
resolver uses its own credential-free client; it never reuses node bearer tokens.
These protections reuse patterns from existing clients, but L402's configured
loopback-origin policy and payment clients remain unchanged.

DNS locates the domain. HTTPS authenticates the domain transport, not the human's
real-world identity or the service's honesty. WebFinger advertises that domain's
account capability. The application independently validates the subject,
relation and service origin. A domain controller can still advertise a malicious
service or change a descriptor; successful discovery grants **no payment
authorization** and is not a permanent mapping to an invoice.

Future invoice acquisition must reapply DNS pinning and all HTTP protections on
every request: this descriptor is not a permanent network-access exemption.
It must bind the human identifier, authoritative domain and service to the user’s
requested asset and amount. A service response cannot replace that economic
contract. The RGB invoice remains the authoritative RGB payment request, subject
to validation against user intent. Harness v1 remains authoritative for policy,
bound approval, reservations, execution and settlement evidence.

## Results and observations

The application result has `status: resolved` and a `recipient` containing
`identifier`, `subject`, `authoritative_domain`, and `service: {type, url}`.
Read-only accessors expose those validated fields. Descriptors cannot be
constructed from deserialized external JSON.

Failures have `status: failed`, a `category` and a fixed `code`. Categories are
`malformed`, `not_found` (HTTP 404), `unsupported`, `unavailable`, and
`security_rejected`. TLS/HTTPS errors fail closed as `https_failure`; detailed
HTTP error strings, response bodies and credentials are never returned.

The model receives at most 1 KiB, for example:

```json
{
  "status": "resolved",
  "identifier": "alice@example.com",
  "service_kind": "rgb_invoice",
  "service_origin": "example.com",
  "next_allowed_actions": [],
  "future_step": "request_rgb_invoice",
  "payment_authorized": false
}
```

The future step is descriptive only: invoice acquisition is not implemented or
advertised as an executable tool. The existing eight-step gateway, approval
mechanisms, economic state machines, journals and observation budgets are unchanged.

## Offline validation

`cargo test -p rgb402-agent recipient --lib` exercises valid discovery, unrelated
relations, unknown accounts, malformed inputs/JRD, missing/ambiguous relations,
subject mismatch, unsafe service endpoints, mixed public/private DNS answers,
HTTPS failure, redirects, response size/media type, total deadline, and bounded
model observations. The private test transport cannot enable plaintext HTTP or
private-network access in the production API. Tests do not require an API key,
Internet access, node environment or payment. TLS errors are injected; this
suite does not claim live interoperability with an external WebFinger service.

## Invoice acquisition follow-on

[Recipient Invoice Acquisition v1](recipient-invoice-acquisition.md) now implements
the standalone HTTPS acquisition and local validation layer. Discovery remains
non-economic; gateway wiring and Harness entry remain future work. The transport
is shared in `recipient/https.rs` without changing the discovery profile.
