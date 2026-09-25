# Agentic wallet (Milestone 2)

The model-independent implementation lives in `rgb402-agent::wallet_agent`.
It adds no workspace members and leaves the simulated buyer and existing wallet
CLI intact. `AgentModel` is a small asynchronous transport boundary; tests use a
scripted model with the real `WalletService` and a mock `RgbNode`.

## Tools

| Tool | Arguments |
| --- | --- |
| `wallet_get_assets` | `{}` |
| `wallet_get_balance` | `asset_id` |
| `wallet_decode_invoice` | `invoice` |
| `wallet_prepare_payment` | `invoice` |
| `wallet_get_plan` | `plan_id` |
| `wallet_execute_payment` | `plan_id` |
| `wallet_payment_status` | `payment_hash` |

Arguments are JSON objects decoded into Rust structs with unknown fields denied.
Outputs are a tagged Rust enum, including structured errors. No tool accepts an
approval, an endpoint, credentials, or replacement payment details. Asset amounts
are integer base units; asset metadata provides display precision. Outbound RGB
and on-chain spendable RGB are separate. BTC balance is not exposed.

## Application approval

A successful non-denied preparation ends the agent turn with `Prepared(plan)`.
No payment occurs in that turn. The trusted application must display this exact
plan's asset ID, amount, invoice, payment hash, carrier amount and policy, then
capture an affirmative response. It calls `confirm_from_human` for that plan ID
and resumes the agent. Cancellation grants nothing. This method is deliberately
absent from the model's tool registry.

The initial agent interface requires explicit application confirmation for every
payment, including amounts that wallet policy would auto-approve. This makes the
prepare/confirm/execute interaction explicit without changing wallet policy.
Execution consumes the application confirmation and uses the existing wallet
plan registry. WalletService still rechecks invoice integrity, balance and policy,
then durably reserves the payment before submission. Repeated preparation of the
same invoice cannot evade the wallet's duplicate reservation checks.

## Outcomes and limits

Each turn allows at most eight model responses/tool iterations, one tool per
response. Invalid tools and arguments return structured errors. Tool arguments
are limited to 16 KiB. Conversation history is cleared at a turn boundary after
128 messages; authoritative plans and reservations are never reconstructed from
conversation text.

Execution queries authoritative status after submission. Results distinguish
pending, settled, failed and uncertain. `submitted: true` means submission was
acknowledged; `submitted: null` makes no submission claim (status-only lookup or
uncertain error). Failed/uncertain calls retain WalletService reservations.
Model prose is not settlement evidence. The CLI displays structured wallet results
immediately, before requesting the next model response. These results remain
visible even if the provider subsequently fails. Model output is labeled `Agent:`;
authoritative output is labeled `Wallet result:`. Terminal control characters are
escaped before display.

Audit events record request, tool outcome, plan presentation and execution,
without logging conversation text, arguments or provider credentials. Existing
wallet audit events continue to record policy, approval and settlement.

## Validation and remaining work

Run the offline agent regressions:

```bash
cargo test -p rgb402-agent --lib --offline
```

Tests cover reads, preparation without submission, approval and cancellation,
policy denial, forged approval, changed arguments/invoices, cross-plan approval,
unknown/consumed plans, replay after uncertain submission, bounded repeated calls,
node failure and pending/failed/settled status.

## OpenAI transport

`wallet_agent/openai.rs` implements `AgentModel` using the existing `reqwest`
dependency and `https://api.openai.com/v1/chat/completions`. No SDK is required.
The provider reads `OPENAI_API_KEY` from the process environment. Before runtime
startup, the CLI loads missing settings from the ignored `.env` file and then
`.env.example`, preserving every explicitly exported value. The priority is
exported environment, `.env`, then `.env.example`. Missing/blank keys
fail before the CLI opens the wallet or contacts a node. The application never
creates, provisions, or writes credentials; an operator may store the key in the
ignored `.env` file. The provider has no Debug/Serialize
implementation; the authorization header is marked sensitive and never becomes
part of a prompt, tool result or audit event.

`AGENT_MODEL` defaults to `gpt-5.6-terra`. The selected model must support Chat
Completions function calling. Requests set `strict: true`,
`parallel_tool_calls: false`, `store: false`, and a 2,048 completion-token limit.
The provider uses typed message, call and response DTOs. Truncated, refused,
malformed or multiple-call responses are rejected. Unknown tools and malformed
arguments still reach the model-independent dispatcher's structured errors.
Requests have a 45-second timeout and a 256-KiB response-body cap. Redirects and
application retries are disabled. The endpoint cannot be configured by a model
or environment variable; the local mock endpoint exists only in unit tests.
Errors expose a category or HTTP status, never a remote body or credential.

API mapping follows the official [function-calling guide](https://developers.openai.com/api/docs/guides/function-calling)
and [Chat API reference](https://developers.openai.com/api/reference/cli/resources/chat).
The [default model documentation](https://developers.openai.com/api/docs/models/gpt-5.6-terra)
describes its supported endpoints and capabilities; account access is checked
only by the eventual live request.

Provider tests use an explicit fake credential and local HTTP server, never the
process key. They cover authentication placement, message roles and call IDs,
strict schemas, response parsing, rejected parallel/truncated/refused responses,
oversized bodies and sanitized HTTP failures. CLI tests use a scripted model to
check confirmation details, cancellation (including EOF and conversational
assent), and settlement display when the following model call fails. Subprocess
tests verify missing-key startup and credential-free help.

## Interactive CLI and live acceptance

With the user-provided `OPENAI_API_KEY` already exported, load the existing wallet
environment and launch the CLI:

```bash
set -a
source .var/regtest/wallet.env
set +a
cargo run -p buyer-agent --bin agent
```

The binary reads ignored `.env` and then `.env.example` as fallback
configuration automatically. Exported settings win, followed by `.env`, so an
empty example key cannot overwrite your configured key. The loader accepts
literal `NAME=value` assignments, optional single/double quotes and comments; it
does not execute shell expressions or expand variables. It loads only known
wallet/model settings. With the key in `.env` and the remaining settings exported
or present in the fallback files, run the cargo command directly.
For the managed regtest stack, source `wallet.env` as above to override example
node/asset placeholders with the actual setup. Use the same durable wallet journal as
Milestone 1. Exit with `/quit`; as with the wallet CLI, forced termination can
leave a stale lock requiring the documented operator recovery procedure.

First ask `What's in my wallet?`, `What RGB assets do I have?`, and `Decode this
invoice: <invoice>`. Paste each full invoice on the same input line as the request.
The agent does not expose BTC balance and must say it is unavailable.

For acceptance, preserve the existing two-node stack and issue a fresh 5-base-unit
R402USD invoice on Bob. Record both actual outbound balances before payment.
Ask `Pay Bob's invoice: <invoice>`, verify the displayed asset, amount, invoice,
payment hash, carrier and policy, then obtain the user's affirmative approval
at the application prompt. Query the payment hash until the wallet reports
settled (pending or submission acknowledgement alone is insufficient). Record
both resulting balances and verify Alice decreased by 5 and Bob increased by 5.
The prior recorded balances were 495 and 105; do not assume they remain current.
Do not reuse the earlier Milestone-1 approvals for this fresh invoice.

The live acceptance session completed on 2026-09-11 using OpenAI `gpt-4.1-mini`.
The model called decode, assets, balance and prepare tools, then the application
paused for approval. After the user's explicit `yes` was relayed as `y`, the model
executed the bound plan exactly once. It correctly reported pending first; a
subsequent status query returned settled. Independent node checks confirmed
Alice 495 → 490 and Bob 105 → 110 R402USD. No provider issue or application code
change was required during acceptance.

**Milestone 2 is complete.** The [acceptance record](acceptance/agent-rgb-lightning-payment.json),
[exact conversation](acceptance/agent-conversation.txt), and
[wallet audit events](acceptance/agent-wallet-audit.log) contain the sanitized
evidence. Formatting, Clippy and all 48 workspace tests passed after the live run.
The existing regtest environment and Milestone-1 acceptance evidence are preserved.
