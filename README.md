# Luma Wallet

Carol's optional store capability: [merchant commerce architecture and setup](docs/merchant-commerce.md)
and [live acceptance progress](docs/acceptance/carol-commerce.md).

For the feature freeze and final human-operated demo, start with the
[final demo runbook](docs/final-demo-runbook.md),
[manual acceptance](docs/final-manual-acceptance.md), and
[hackathon deployment boundaries](docs/hackathon-deployment.md).
Read-only readiness: `python3 scripts/demo/check.py`.

RGB402 is a Bitshala BOSS Battle bootstrap project for machine-to-machine paid digital services.

The original simulated demo is local and deterministic: an autonomous buyer agent requests a protected resource, receives an HTTP 402 challenge denominated in an RGB-style asset, checks a hard spending policy, pays through a simulated RGB payment provider, retries with a payment identifier, and receives the resource.

The original merchant/buyer demo below remains simulated. The new `wallet` binary calls a real RGB Lightning node; it does not use an LLM. A real regtest payment has been observed: Alice paid Bob 5 R402USD; balances changed 500 → 495 and 100 → 105.


## Agent Harness v1

The existing RGB and L402 workflows share checked economic states, contract-bound
authorization, conservative reservation recovery, and bounded model observations.
See [the harness architecture and invariant/test map](docs/agent-harness.md).

## Milestone 4: L402 machine commerce

The PWA agent can purchase the local premium report for **3 BTC Lightning sats**
under an independent **up-to-10-sat automatic policy**. Real settlement, payment
proof, authenticated retry and HTTP 200 were observed. The 50-sat extended report
requires application approval. R402USD transfers retain their existing approval
path. See [setup, policy and recovery](docs/l402.md) and
[acceptance evidence](docs/acceptance/l402-purchase.json), including the initial
pending-status recovery and subsequent timing fix.

## Milestone 1: real RGB Lightning payment

On Linux with Docker/Compose, Rust, Git, Python 3, a C compiler and CMake:

```bash
./scripts/regtest/demo.sh
```

Review the real wallet plan and answer `y` at its approval prompt. The command uses
pinned upstream infrastructure, creates a demo asset/channel, pays through our
wallet, and verifies settlement plus both balance changes. It preserves state;
rerunning verifies the recorded attempt without paying again.

See [regtest setup and troubleshooting](docs/regtest.md),
[exact upstream pins](docs/rgb-lightning-node-version.md), and
[observed payment evidence](docs/acceptance/rgb-lightning-payment.json).

```bash
./scripts/regtest/status.sh
./scripts/regtest/stop.sh
./scripts/regtest/reset.sh --yes  # deletes only this project's development regtest state
```

For an independently managed node, the original manual configuration follows.

The existing workspace is preserved. Wallet domain/policy lives in
`rgb402-core::wallet`; configuration, the narrow `RgbNode` client, and
`WalletService` live in `rgb402-payment`. The wallet CLI lives in `apps/buyer-agent`
alongside the legacy buyer and the Milestone-2 agent CLI. The deterministic wallet
path does not depend on a model.

Requires a compatible, unlocked **regtest** RGB Lightning node and a funded RGB
channel to another node. See [API contract and live demo](docs/node-api.md).

```bash
cp .env.example .env
# Edit .env: set the real test asset ID, node URL/token, and policy limits.
set -a
source .env
set +a
cargo run -p buyer-agent --bin wallet -- --help
cargo run -p buyer-agent --bin wallet -- assets
cargo run -p buyer-agent --bin wallet -- balance
cargo run -p buyer-agent --bin wallet -- balance "$ALLOWED_ASSET_IDS"
cargo run -p buyer-agent --bin wallet -- decode "$RGB_INVOICE"
cargo run -p buyer-agent --bin wallet -- prepare "$RGB_INVOICE"
cargo run -p buyer-agent --bin wallet -- pay "$RGB_INVOICE"
cargo run -p buyer-agent --bin wallet -- status "$PAYMENT_HASH"
```

Set `RGB_INVOICE` to Node B's fixed-amount RGB **Lightning** invoice (`lnbcrt...`).
Use one asset ID for the explicit balance command, not a comma-separated list.
`pay` prepares a fresh plan, displays its exact invoice, asset, base-unit amount,
carrier millisatoshis and policy, and requires `y` or `yes` when approval is needed.
Blank input, EOF and other responses cancel. `prepare` never submits a payment;
its process-local plan ID cannot be executed by a later CLI process.
Submission can return `Pending`; use the **payment hash**, not the node payment ID,
to query settlement.

Limits use integer **asset base units**, without floating point: for precision 2,
100 means 1.00. The same limits apply independently to each allowed asset; keep
one demo asset configured initially. Exactly the auto threshold requires approval.
Use `AUTO_APPROVE_BELOW=0` to require approval for every positive payment.
The regtest demo asset is **RGB402 Demo Dollar / R402USD** (precision 0), not official Tether USD₮.

The service checks asset, positive amount, regtest network, invoice expiry,
carrier amount, outbound RGB liquidity, single limit and UTC-day spend.
It checks again immediately before submission and rejects changed invoice details.
Approval cannot override a denial. Spend reservations are synced before node calls,
including failed/uncertain calls; replay of a reserved payment hash is blocked.
An exclusive lock serializes wallet CLI processes using the same journal.

Always use the same `WALLET_STATE_PATH` for this node. The journal tracks payments
submitted through this wallet only; do not run other spenders against the node.
Do not delete it to retry an uncertain payment or reset a limit. After a crash,
confirm no wallet process is alive, query the node for reserved hashes, and only
then remove the stale `.wallet-state.lock` file. Preserve the journal. A truncated
or invalid journal fails closed and requires operator recovery. Reservations count
on their UTC submission day; failed reservations are not refunded automatically.
This is a single-user local demo, not a production custody or tamper-proof ledger.

Audit events go to stderr through tracing. For a saved trail:

```bash
cargo run -p buyer-agent --bin wallet -- pay "$RGB_INVOICE" 2>>wallet-audit.log
```

Node response bodies, credentials, payment secrets and preimages are not logged or
returned. The node controls Lightning routing fees; the wallet limits invoice
carrier value but does not implement a separate routing-fee policy.

## Milestone 2: conversational wallet

The `agent` binary connects OpenAI to seven typed wallet tools. Preparation pauses
for application-owned confirmation; the model cannot grant approval or replace
payment details. Each turn has an eight-step limit. Wallet results are displayed
separately from model explanations, including pending, settled, failed and
uncertain payment states.

With `OPENAI_API_KEY` already exported in your process environment:

```bash
set -a
source .var/regtest/wallet.env
set +a
cargo run -p buyer-agent --bin agent
```

Ask `What's in my wallet?` or `Pay this invoice: <invoice>` (paste the full invoice
on the same line). Review the exact plan, then answer the application's `[y/N]`
prompt. Type `/quit` to release the wallet lock and exit.

`AGENT_MODEL` optionally selects a Chat Completions model with function calling;
the default is `gpt-4.1-mini`. The agent loads missing settings from `.env.example` in the current directory;
exported environment variables take precedence, including over empty example
values. It does not load `.env`, and fails at startup if the key is missing or empty.
Keep the key out of files, transcripts and logs. The OpenAI entry in `.env.example`
is empty. Node credentials are never sent in model messages.

**Milestone 2 is complete:** a real OpenAI `gpt-4.1-mini` session prepared and
executed one application-approved 5 R402USD payment. Independent node verification
confirmed settlement and balances Alice 495 → 490, Bob 105 → 110.
See [live agent evidence](docs/acceptance/agent-rgb-lightning-payment.json) and
[the conversation](docs/acceptance/agent-conversation.txt).
See [agent implementation and acceptance procedure](docs/agent.md).

## Milestone 3: mobile wallet PWA

The mobile-first React/TypeScript PWA provides Home, Agent, Activity and Settings,
with an application-owned payment approval sheet. It uses actual backend state
and the same agent tools, wallet policy and durable reservations as the CLI.

```bash
npm --prefix apps/wallet-ui ci
npm --prefix apps/wallet-ui run build
# Load the existing wallet.env and server-side OpenAI configuration as above.
cargo run -p buyer-agent --bin api
```

Open **http://127.0.0.1:3030**. For Vite development, run
`npm --prefix apps/wallet-ui run dev` and open http://127.0.0.1:5173.
See [PWA setup, security and tests](docs/pwa.md). **Milestone 3 is complete:** one
PWA-driven, application-approved 5 R402USD payment settled, independently verified
with Alice 490 → 485 and Bob 110 → 115. The [acceptance record](docs/acceptance/pwa-rgb-lightning-payment.json)
and [approval screenshot](docs/acceptance/pwa-approval.png) document the flow.
Prior CLI/agent settlement evidence remains unchanged.

## Development checks

```bash
cargo check --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Tests need no RGB node or OpenAI key. HTTP contract tests open loopback sockets. Tests cover
policy boundaries, approval, execution-time revalidation, failure reservations,
duplicate payment prevention, persistence/locking, and actual HTTP DTO mapping.
The two-node live acceptance payment has settled; see the recorded evidence above.

## Workspace

```text
crates/rgb402-core      Domain types, legacy challenges, wallet policy
crates/rgb402-payment   Wallet service, typed node client, config; legacy simulated provider
crates/rgb402-merchant  Axum merchant HTTP 402 API
crates/rgb402-agent     Wallet agent tools, bounded loop, OpenAI provider; legacy buyer
apps/merchant-server    Demo merchant process
apps/buyer-agent        Interactive agent, wallet CLI and legacy demo buyer process
docs/                   Protocol, architecture, and roadmap notes
```

## Demo

Terminal 1:

```bash
cargo run -p merchant-server
```

Terminal 2:

```bash
cargo run -p buyer-agent
```

Both processes use `.rgb402-sim-ledger.json` by default so the simulated settlement state is shared across terminals.

Expected flow:

```text
Requesting premium-analysis...

Merchant response: 402 Payment Required

Payment challenge:
asset: rgb:boss
amount: 4.00

Policy:
allowed_asset: yes
within_max_payment: yes
within_budget: yes

Decision: APPROVED

Paying simulated RGB invoice...

Payment settled:
payment_id: sim-rgb-payment-req-premium-analysis

Retrying premium-analysis...

200 OK
```

## Tests

```bash
cargo test
```

The integration tests cover successful purchase, too-expensive challenge, wrong asset, budget exhaustion, fake receipt, and expired challenge.

## Historical Polar stub

The original simulated merchant/buyer flow retains its placeholder `RgbLightningPaymentProvider`. The working wallet path uses `RgbLightningClient` and the upstream regtest stack; Polar is not required.

See [docs/roadmap.md](docs/roadmap.md) for the integration plan.

## Two-wallet demo

Run Alice and Bob as isolated instances of the same PWA/API with Receive and inbound
activity. See [launch and acceptance instructions](docs/two-wallets.md).
