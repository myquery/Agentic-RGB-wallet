# Recipient service preflight: blocked before deployment

The requested fresh R402USD asset contains `~`. The existing
`RecipientInvoiceContract::new` asset alphabet permits only ASCII alphanumeric,
colon, hyphen and underscore. Thus this exact asset cannot enter acquisition,
even with successful public HTTPS discovery. No validation rule was changed.
This is a contract-input incompatibility, not a failed live invoice parse.

ngrok is installed and its configuration file exists; a public hostname has not
been supplied or verified. No credentials were displayed or changed.

The smallest deployment option identified is a recipient-only entrypoint in the
existing merchant-server package, exposing WebFinger and one invoice route. It
would forward validated issuance requests to Bob's loopback `/lninvoice` and
return only the invoice. Existing simulated and L402 merchant routes do not
implement this RGB recipient protocol. No new server was added or deployed.

No model call, discovery request, acquisition, invoice issuance, payment plan,
approval, reservation, execution or status poll was performed. Payment submissions
remain zero for this task. Live decoded fields and invoice IDs are unavailable.

To proceed, resolve the asset compatibility separately (explicitly authorize a
reviewed validation correction, or supply a funded asset accepted by the current
contract) and confirm a usable public HTTPS hostname. Do not silently substitute
another asset or relax recipient network rules.

The user subsequently authorized ngrok when localhost is inapplicable. Localhost
is inapplicable under the unchanged public-network policy. No tunnel was started
because the requested asset remains incompatible with contract construction.

Validation: 35 targeted recipient tests passed; cargo fmt --check and strict
workspace/all-target Clippy passed. No frontend files changed.
