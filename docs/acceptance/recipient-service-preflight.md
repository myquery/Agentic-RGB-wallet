# Live recipient service preflight

Recipient: `alice@3f0e-102-88-113-62.ngrok-free.app`.
Public origin: `https://3f0e-102-88-113-62.ngrok-free.app`.

1. Started ngrok with the existing legacy configuration and inspection disabled.
   The initial Snap default configuration path was missing; specifying the
   existing config fixed startup. No credentials were changed or printed.
2. Started the recipient-only merchant binary on loopback port 3050. The public
   tunnel exposes its WebFinger and invoice routes; Bob's API remains on loopback.
3. Ran `recipient-preflight` with the recipient and fresh RGB asset in its
   environment. No model or wallet service was started by this command.
4. Production resolution succeeded, establishing exact WebFinger subject,
   one supported relation and same-origin HTTPS endpoint. Production acquisition
   made one invoice request, then the existing local RGB-aware parser validated
   the exact requested asset, five base units, regtest, expiry and carrier ceiling.
5. Stopped at validation. No preparation, approval, reservation, execution or
   payment-status polling was performed. Payment submissions: **0**.

The JSON record contains the timestamp, requested/decoded values, invoice ID,
payment hash, expiry and carrier. No raw invoice, JRD, DNS IPs or credentials are
included. The acquired invoice is only validated at the recorded time and will
expire; it is not stored as a payment plan or approval for a later task.

The original asset alphabet blocker was corrected solely by accepting `~`, with
a signed-invoice regression using the exact node-issued asset. No other recipient
validation or wallet/Harness/tool policy was changed.

Remaining requirements for a later live agent payment: keep the tunnel, service
and nodes healthy; verify current balances; acquire a fresh invoice via the agent
and obtain any required application approval for its actual immutable plan.

Post-run validation: 36 recipient tests and one server validation test pass;
formatting, strict workspace/all-target Clippy, both binary builds and diff checks
pass. No frontend files changed.
