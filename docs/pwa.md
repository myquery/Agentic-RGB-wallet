For two independent Alice/Bob instances and the current Receive/history flow, see
[two-wallet setup](two-wallets.md).

# Mobile wallet PWA (Milestone 3)

The React/TypeScript UI lives in `apps/wallet-ui`. Vite builds a static app that the
new `buyer-agent` `api` binary serves on **http://127.0.0.1:3030**. The existing six
Rust workspace members, wallet policy, plan registry, approval capability and
reservation journal remain intact. Milestone 3 added read-only accessors for balances, assets, history and payment
status. Milestone 4 adds a separate optional BTC machine-purchase service; see
[L402 setup and policy](l402.md).

## Run locally

Use Node 20 and npm, Rust, and the existing regtest prerequisites. From the repo:

```bash
./scripts/regtest/start.sh
./scripts/regtest/bootstrap.sh
npm --prefix apps/wallet-ui ci
npm --prefix apps/wallet-ui run build
set -a
source .var/regtest/wallet.env
set +a
cargo run -p buyer-agent --bin api
```

Open http://127.0.0.1:3030. Port 3030 avoids another local service already using
3003. The API binds only to loopback. The same wallet journal is used by the CLI
and web API: exit other wallet processes before starting the API. Ctrl-C shuts
it down gracefully; do not delete a journal or a live process's lock to retry.

The existing `OPENAI_API_KEY` and optional `AGENT_MODEL` remain server-side. The
API uses the same ignored `.env` then `.env.example` fallback loader as the CLI;
exported variables win. Keep all credentials out of the frontend and its environment.
Do not put secrets in examples, documentation, screenshots or acceptance artifacts.

For frontend development, keep the API running and in another terminal run:

```bash
npm --prefix apps/wallet-ui run dev
```

Open http://127.0.0.1:5173. Vite proxies `/api` to loopback port 3030. It uses an
isolated environment directory and does not load the repository's environment
files. PWA registration runs in the production build, not the Vite development
server. The backend must be running for real data; no mock balances replace it.

## Interaction

Home shows outbound RGB balance, separate on-chain RGB, and recent activity.
The Home sats total remains explicitly unavailable. The machine-purchase service
separately checks conservative BTC Lightning outbound capacity for its plans. Send opens a direct RGB Lightning invoice sheet: paste, review the decoded plan,
and approve. It makes no model request. Natural-language payments remain available
in the Agent tab. Agent tool outputs become wallet-action messages rather than raw
JSON. Model explanations, user messages, wallet actions and approval are labeled
separately. Amount formatting uses integer arithmetic, including large values.

Preparing a plan pauses the agent. The dedicated confirmation sheet shows the
server's exact amount, asset, invoice/hash, carrier, available balance and policy.
The browser posts `{}` to the plan-specific approval/rejection URL. It cannot
supply an amount, asset or destination with approval. Cancel does not execute.
Approval executes the already-bound plan through the existing wallet execution
path. Direct Send does not invoke the model; agent requests resume the agent loop. WalletService rechecks the invoice and policy and
reserves the payment before submission, exactly as in the CLI.

The browser polls session progress every 1.5 seconds. Agent work runs in a
server-owned task, so losing a browser connection does not silently cancel or
retry a submitted payment. While a turn runs, wallet reads return busy and the
browser retains the last view. Once it finishes, activity and balances refresh.
The Agent view also displays a current-status receipt from this authoritative
activity, so historical pending messages do not hide a later settlement.
Activity is reconstructed from the durable reservation journal and statuses are
queried from the node. If status lookup fails it is shown as **uncertain**, never
failed or settled. No browser-local history invents payment outcomes. Conversation
history and pending approval are local to the running API session; reservations
survive API restarts. A restart invalidates pending approval and session tokens.

Receive creates an invoice through the local application API and offers copy.
See [two-wallet setup](two-wallets.md) for asset/base-unit inputs and demo carrier
settings. QR scanning is not required for the paste-based agent send flow.

## HTTP surface and local session

| Method | Endpoint | Purpose |
| --- | --- | --- |
| GET | `/api/session` | Busy state, UI events, pending bound plan and CSRF token |
| GET | `/api/wallet` | Actual RGB holdings and network |
| GET | `/api/assets` | Asset metadata |
| GET | `/api/activity` | Reserved payments with authoritative status |
| GET | `/api/payments/:id` | Authoritative status by payment hash |
| POST | `/api/agent/message` | Typed `{ "message": "..." }`, starts one bounded turn |
| POST | `/api/approvals/:plan_id/approve` | Empty object, confirms that pending plan |
| POST | `/api/approvals/:plan_id/reject` | Empty object, cancels that pending plan |

All mutations require `X-Wallet-CSRF`, obtained from the same-origin session
endpoint. Host and Origin checks restrict access to the configured local UI;
cross-site requests are rejected and no permissive CORS layer is installed.
The token is a browser session safeguard, not a node or OpenAI credential. This
is a single local user session, not production wallet authentication. Do not
expose this service or the Vite server to the public internet. All responses are
no-store, and the built app uses a restrictive Content Security Policy. Static
serving is limited to the built UI directory and approved asset file types.

The model never receives the browser session token or an approval tool. A browser
request is still untrusted: typed parsers reject extra approval fields; a busy,
nonexistent, consumed or cancelled plan cannot be approved again. Amount and
carrier integers are serialized to strings for JavaScript precision. Audit events
continue on stderr without credentials or raw model transport bodies.

## PWA installation

The web manifest declares standalone display, theme/background, scope, start URL,
and 192/512 PNG icons with a maskable safe area. A service worker caches the static
shell and built assets only. `/api` and every non-GET request bypass caching;
there is no background payment queue or offline submission. Offline/reconnection
states disable payment controls.

Install from a supporting browser's app/install menu on localhost. On iOS use
Share → Add to Home Screen when accessing a trusted HTTPS origin. Physical phones
cannot reach a server bound to another machine's loopback; use a trusted secure
local development arrangement rather than exposing the wallet publicly. Chrome
browser checks verify the mobile viewport, service worker and manifest. See the
[installability requirements](https://developer.mozilla.org/en-US/docs/Web/Progressive_web_apps/Guides/Making_PWAs_installable).

## Tests

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm --prefix apps/wallet-ui test
npm --prefix apps/wallet-ui run build
```

The 48 prior Rust tests remain green. Six new API tests cover balances, unknown
plans, altered approval bodies, changed invoices after preparation, replay,
rejection, authoritative activity, CSRF, origin and host checks. The API binary
also runs the existing two config-loader tests. Thirteen frontend tests cover
balance rendering, message submission, approval/rejection, identifier-only
payloads, pending/settled/failed/uncertain statuses, invoice errors, exact large
amounts, in-flight approval disabling, and the current receipt changing from
pending to settled. All tests use scripted models and mock
nodes; none require a real OpenAI key or RGB node. HTTP contract tests bind local
mock sockets.

## Live acceptance

**Milestone 3 is complete.** On 2026-09-11, the real PWA displayed Alice's 490
R402USD balance, submitted a natural-language request containing Bob's fresh
5-unit invoice, and presented the bound payment approval sheet. OpenAI
`gpt-4.1-mini` used decode, assets, balance and preparation tools. Human approval
was captured by the application, followed by one execution and status lookup.
Activity then reported settled from the node, and Home refreshed to 485 R402USD.
An independent node query confirmed Alice 490 → 485 and Bob 110 → 115.

The environment needed the stopped Electrs service restarted without a reset;
an occupied port prompted using 3030. The live review also added the Agent's
current-status receipt alongside its historical messages. The approval-click
automation timed out after the approval sheet had already been consumed; audit
evidence confirmed one approval and one submission. No financial action was
retried. Final screenshots were recovered through read-only browser checks.

Formatting, Clippy, 56 Rust tests, 13 frontend tests and the production build
passed after the run. The browser reported no JavaScript errors; mobile layout,
manifest and active service worker were checked. The final bundle and evidence
were scanned for configured credentials without exposing them.

See the [acceptance record](acceptance/pwa-rgb-lightning-payment.json),
[sanitized browser events](acceptance/pwa-browser-events.json), and
[audit trail](acceptance/pwa-wallet-audit.log). Screenshots:
[Home before](acceptance/pwa-home-before.png),
[approval sheet](acceptance/pwa-approval.png),
[settled activity](acceptance/pwa-activity-settled.png),
[Home after](acceptance/pwa-home-after.png),
[Agent receipt](acceptance/pwa-agent-result.png), and
[desktop layout](acceptance/pwa-desktop-home.png).

## Wallet UX and direct invoice Send

Wallet identity remains visible across tabs. Settings displays configured RGB
policy limits in base units per asset (daily accounting uses UTC). Direct invoice
Send always presents application confirmation, including amounts below the automatic
threshold. Denied plans cannot be approved. Duplicate approval is rejected; execution
still revalidates the invoice, policy and balance and persists the reservation before
submission. Uncertain submission must be checked in Activity, never automatically retried.

`POST /api/send/prepare` accepts only `{ "invoice": "..." }` with the existing
same-origin and CSRF checks. It prepares asynchronously and exposes the immutable
plan through `/api/session`; the existing plan-specific approve/reject endpoints
remain the only browser approval mechanism.

For wallet operations without OpenAI, explicitly start with:

```bash
WALLET_AGENT_ENABLED=false ./scripts/wallet/api.sh alice
WALLET_AGENT_ENABLED=false ./scripts/wallet/api.sh bob
```

Normal mode still requires `OPENAI_API_KEY` at startup. Disabled mode performs no
model requests and chat explains that the agent is disabled. Direct Send, Receive,
balances and Activity work in this mode.

The UI distinguishes loading, unavailable and cached balances. Recovered connectivity
clears connection errors without erasing action errors. Pending rows indicate active
status refresh. Merchant suggestions appear only when commerce is configured.
Receive capacity is not yet exposed; outbound balance is spendable capacity, not an
estimate of what the wallet can receive.

## Human BTC addresses

The Agent supports sats to the configured public recipient addresses with a
separate **Confirm BTC payment** sheet. It always requires human approval, uses
separate BTC limits/journal, and shows BTC Activity in sats. See
[BTC recipient payments](btc-recipient-payments.md). The direct invoice Send sheet
continues to accept RGB invoices.
