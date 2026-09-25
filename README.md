# Luma

**Agentic commerce on Bitcoin and RGB over Lightning.**

Luma is a mobile-first wallet and commerce platform for Bitcoin Lightning and RGB assets, designed so both people and software agents can discover, evaluate, authorize, and execute payments under explicit wallet policy.

The project combines:

* **Bitcoin Lightning payments**
* **RGB asset payments over Lightning**
* **Agent-driven wallet interactions**
* **Application-owned human approval**
* **Bounded automatic spending policies**
* **L402 machine-to-machine commerce**
* **Optional merchant/store capabilities**
* **Wallet-to-wallet RGB and BTC flows**

Luma grew out of **RGB402**, the original Bitshala BOSS Battle bootstrap project for machine-to-machine paid services. That early deterministic simulation remains in the repository for reference and testing, but the current Luma paths use real RGB Lightning nodes, real wallet state, real settlement, and a mobile PWA.

> **Important:** the development RGB asset used throughout the regtest demos is **RGB402 Demo Dollar (`R402USD`)**, not official Tether USD₮.

---

## Current status

Luma has progressed from a simulated HTTP 402 experiment into a working agentic wallet and commerce stack.

| Capability                          | Status                                                             |
| ----------------------------------- | ------------------------------------------------------------------ |
| RGB asset payment over Lightning    | Real regtest settlement observed                                   |
| Deterministic wallet CLI            | Working                                                            |
| Conversational wallet agent         | Real application-approved payment observed                         |
| Mobile wallet PWA                   | Real application-approved payment observed                         |
| Bitcoin Lightning / L402 purchase   | Real settlement and authenticated retry observed                   |
| Human approval boundary             | Application-owned                                                  |
| Automatic spending policy           | Supported within configured limits                                 |
| Wallet-to-wallet operation          | Alice/Bob isolated wallet demo                                     |
| Merchant/store mode                 | Optional capability with separate architecture and acceptance path |
| Original RGB402 merchant/buyer demo | Retained as historical simulated flow                              |

For the frozen human-operated demo, start with:

* [Final demo runbook](docs/final-demo-runbook.md)
* [Manual acceptance](docs/final-manual-acceptance.md)
* [Hackathon deployment boundaries](docs/hackathon-deployment.md)

Read-only readiness check:

```bash
python3 scripts/demo/check.py
```

Merchant/store documentation:

* [Merchant commerce architecture and setup](docs/merchant-commerce.md)
* [Merchant acceptance progress](docs/acceptance/carol-commerce.md)

---

# How Luma works

At a high level:

```text
Human or Agent
      │
      ▼
┌─────────────────────┐
│      Luma PWA       │
│ Home / Agent /      │
│ Activity / Settings │
└─────────┬───────────┘
          │
          ▼
┌─────────────────────┐
│ Wallet / Agent API  │
│                     │
│ typed tools         │
│ payment planning    │
│ policy checks       │
│ approval boundary   │
│ reservations        │
└─────────┬───────────┘
          │
          ▼
┌─────────────────────┐
│ RGB Lightning Node  │
│                     │
│ Bitcoin Lightning   │
│ RGB assets          │
│ channels/liquidity  │
│ settlement state    │
└─────────────────────┘
```

The model does **not** directly control wallet execution.

The application owns payment authorization. Wallet policy is checked before a payment is prepared and checked again immediately before submission. A model may request or explain an action, but it cannot grant its own approval or replace the economic details of an approved payment.

---

## Agent Harness v1

The RGB and L402 workflows share a common economic safety model:

* checked economic states
* contract-bound authorization
* conservative reservation recovery
* bounded model observations

See:

[Agent Harness architecture and invariant/test map](docs/agent-harness.md)

This allows Luma to expose commerce capabilities to an agent without handing the model unrestricted control of wallet funds.

---

# Proven payment paths

## 1. Real RGB Lightning payment

The deterministic wallet path talks to a real RGB Lightning node and does **not** depend on an LLM.

A real regtest payment was observed:

```text
Alice: 500 → 495 R402USD
Bob:   100 → 105 R402USD
Payment: 5 R402USD
```

On Linux with Docker/Compose, Rust, Git, Python 3, a C compiler, and CMake:

```bash
./scripts/regtest/demo.sh
```

Review the generated wallet plan and answer `y` at the approval prompt.

The script uses pinned upstream infrastructure, creates the demo asset/channel, executes the payment through Luma, and verifies settlement and both balance changes.

State is preserved. Re-running the script verifies the recorded attempt rather than paying again.

Documentation:

* [Regtest setup and troubleshooting](docs/regtest.md)
* [Exact upstream pins](docs/rgb-lightning-node-version.md)
* [Observed payment evidence](docs/acceptance/rgb-lightning-payment.json)

Useful lifecycle commands:

```bash
./scripts/regtest/status.sh
./scripts/regtest/stop.sh
./scripts/regtest/reset.sh --yes
```

`reset.sh --yes` deletes only this project's development regtest state.

---

## 2. Conversational wallet

The `agent` binary connects an OpenAI model to seven typed wallet tools.

The agent can inspect the wallet, decode payment requests, prepare payments, and explain wallet decisions, but payment authorization remains under application control.

Preparation pauses for application-owned confirmation when required.

The model cannot:

* approve its own payment
* replace payment details after authorization
* bypass a policy denial
* directly access node credentials

Each turn has an eight-step limit.

Wallet results are displayed independently of model explanations and distinguish:

```text
pending
settled
failed
uncertain
```

Run the agent with `OPENAI_API_KEY` already exported:

```bash
set -a
source .var/regtest/wallet.env
set +a

cargo run -p buyer-agent --bin agent
```

Example requests:

```text
What's in my wallet?
```

or:

```text
Pay this invoice: <invoice>
```

Paste the complete invoice on the same line.

When required, inspect the exact payment plan and respond to the application's:

```text
[y/N]
```

prompt.

Use:

```text
/quit
```

to release the wallet lock and exit.

`AGENT_MODEL` can select a compatible Chat Completions model with function calling. The current default is:

```text
gpt-5.6-terra
```

The agent loads missing settings from the ignored `.env` file, then `.env.example`. Exported environment variables take precedence, followed by `.env`, then `.env.example`.

Startup fails if the OpenAI API key is missing or empty.

Keep the key out of:

* committed files
* transcripts
* logs

The OpenAI entry in `.env.example` is deliberately empty, and RGB Lightning node credentials are never included in model messages.

### Observed agent payment

A real `gpt-4.1-mini` session prepared and executed one application-approved payment:

```text
Payment: 5 R402USD

Alice: 495 → 490
Bob:   105 → 110
```

Independent node verification confirmed settlement.

Evidence:

* [Live agent payment](docs/acceptance/agent-rgb-lightning-payment.json)
* [Agent conversation](docs/acceptance/agent-conversation.txt)
* [Agent implementation and acceptance procedure](docs/agent.md)

---

## 3. Mobile wallet PWA

Luma's mobile-first React/TypeScript PWA provides:

* **Home**
* **Agent**
* **Activity**
* **Settings**

Payment approval is presented through an application-owned approval sheet.

The UI uses actual backend wallet state and the same:

* typed agent tools
* wallet policy
* authorization rules
* durable spend reservations

used by the CLI.

Build the frontend:

```bash
npm --prefix apps/wallet-ui ci
npm --prefix apps/wallet-ui run build
```

Load the wallet and server-side OpenAI configuration, then start the API:

```bash
cargo run -p buyer-agent --bin api
```

Open:

```text
http://127.0.0.1:3030
```

For Vite development:

```bash
npm --prefix apps/wallet-ui run dev
```

Then open:

```text
http://127.0.0.1:5173
```

Documentation:

* [PWA setup, security, and tests](docs/pwa.md)
* [Wallet API service setup](deploy/systemd/README.md)

### Observed PWA payment

A PWA-driven, application-approved payment settled successfully:

```text
Payment: 5 R402USD

Alice: 490 → 485
Bob:   110 → 115
```

Evidence:

* [PWA acceptance record](docs/acceptance/pwa-rgb-lightning-payment.json)
* [Approval screenshot](docs/acceptance/pwa-approval.png)

The earlier CLI and conversational-agent settlement evidence remains unchanged.

---

## 4. L402 machine commerce

Luma can also act as an autonomous buyer of Bitcoin-native paid services.

The PWA agent successfully purchased the local premium report for:

```text
3 BTC Lightning sats
```

under an independent automatic policy allowing payments below the configured 10-sat threshold.

The complete flow demonstrated:

```text
agent requests resource
        ↓
merchant returns payment requirement
        ↓
Luma evaluates spending policy
        ↓
Lightning payment settles
        ↓
payment proof is attached
        ↓
request is authenticated and retried
        ↓
HTTP 200
```

Real settlement, payment proof, authenticated retry, and the final HTTP `200` response were observed.

A larger:

```text
50 sat
```

extended report requires application approval.

The L402 spending policy is independent of the RGB asset transfer policy. Existing `R402USD` transfers retain their approval path.

See:

* [L402 setup, policy, and recovery](docs/l402.md)
* [L402 acceptance evidence](docs/acceptance/l402-purchase.json)

The acceptance evidence also records the initial pending-status recovery and the subsequent timing fix.

---

# Merchant commerce

Luma is not limited to wallet-to-wallet transfers.

An optional merchant/store capability allows a wallet instance to participate in commerce flows while preserving the wallet's existing payment and policy architecture.

The current merchant work is documented separately:

* [Merchant commerce architecture and setup](docs/merchant-commerce.md)
* [Carol merchant acceptance](docs/acceptance/carol-commerce.md)

This capability is intentionally separate from the core wallet so a wallet does not need to become a merchant merely by running Luma.

---

# Two-wallet demo

Alice and Bob can run as isolated instances of the same Luma PWA/API.

The demo includes Receive functionality and inbound activity.

See:

[Two-wallet launch and acceptance instructions](docs/two-wallets.md)

---

# Wallet CLI

For an independently managed RGB Lightning node, Luma also exposes a deterministic wallet CLI.

Wallet domain and policy live in:

```text
rgb402-core::wallet
```

Configuration, the narrow `RgbNode` client, and `WalletService` live in:

```text
rgb402-payment
```

The wallet CLI lives under:

```text
apps/buyer-agent
```

alongside the legacy buyer and conversational agent CLI.

The deterministic wallet path does not depend on an AI model.

## Requirements

The wallet requires:

* a compatible unlocked **regtest** RGB Lightning node
* a funded RGB channel to another node

See:

[Node API contract and live demo](docs/node-api.md)

Configure the environment:

```bash
cp .env.example .env

# Edit .env:
# - real test asset ID
# - node URL/token
# - wallet policy limits

set -a
source .env
set +a
```

Wallet commands:

```bash
cargo run -p buyer-agent --bin wallet -- --help
cargo run -p buyer-agent --bin wallet -- assets
cargo run -p buyer-agent --bin wallet -- balance
cargo run -p buyer-agent --bin wallet -- balance "$ALLOWED_ASSET_IDS"
cargo run -p buyer-agent --bin wallet -- decode "$RGB_INVOICE"
cargo run -p buyer-agent --bin wallet -- prepare "$RGB_INVOICE"
cargo run -p buyer-agent --bin wallet -- pay "$RGB_INVOICE"
cargo run -p buyer-agent --bin wallet -- status "$PAYMENT_HASH"
```

Set `RGB_INVOICE` to Node B's fixed-amount RGB **Lightning** invoice:

```text
lnbcrt...
```

For the explicit balance command, supply one asset ID rather than a comma-separated list.

---

# Payment authorization and policy

`pay` creates a fresh payment plan and shows the exact:

* invoice
* asset
* amount in base units
* carrier millisatoshis
* wallet policy result

If application approval is required, only:

```text
y
```

or:

```text
yes
```

authorizes execution.

Blank input, EOF, and all other responses cancel.

`prepare` never submits a payment.

Its process-local plan ID cannot be executed by a later CLI process.

---

## Pending payments

Submission can return:

```text
Pending
```

Settlement should be queried using the **payment hash**, not the node payment ID.

---

## Amount representation

Policy limits use integer **asset base units** and do not use floating-point arithmetic.

For an RGB asset with precision `2`:

```text
100 = 1.00
```

The configured limits apply independently to each allowed asset.

For the current demo, keep one asset configured initially.

The exact automatic-approval threshold itself requires approval.

To require approval for every positive payment:

```bash
AUTO_APPROVE_BELOW=0
```

---

# Wallet safety model

Before submission, Luma validates:

* allowed asset
* positive amount
* regtest network
* invoice expiry
* carrier amount
* outbound RGB liquidity
* per-payment limit
* UTC-day spend

The economic details are checked again immediately before submission.

If the invoice changes, execution is rejected.

A user approval cannot override a policy denial.

---

## Spend reservations

Spend reservations are synchronized before node calls, including calls that fail or return an uncertain state.

Replay of a reserved payment hash is blocked.

An exclusive lock serializes wallet CLI processes sharing the same journal.

Always use the same:

```text
WALLET_STATE_PATH
```

for a given node.

The journal tracks only payments submitted through this wallet. Do not run additional independent spenders against the same node while relying on Luma's wallet limits as the authoritative accounting boundary.

Do **not** delete the journal to:

* retry an uncertain payment
* reset a spending limit

After a crash:

1. Confirm no Luma wallet process remains alive.
2. Query the node for reserved payment hashes.
3. Only then remove a stale `.wallet-state.lock`.
4. Preserve the wallet journal.

A truncated or invalid journal fails closed and requires operator recovery.

Reservations count against the UTC day on which they were submitted.

Failed reservations are not refunded automatically.

---

# Security boundaries

The current implementation is a **single-user local demo**, not a production custody platform or tamper-proof accounting ledger.

Node response bodies, credentials, payment secrets, and preimages are not logged or returned.

Audit events are emitted to `stderr` through tracing.

To save an audit trail:

```bash
cargo run -p buyer-agent --bin wallet -- pay "$RGB_INVOICE" 2>>wallet-audit.log
```

The RGB Lightning node controls Lightning routing fees.

Luma currently limits invoice carrier value but does not implement a separate Lightning routing-fee policy.

---

# Development

Run the full workspace checks:

```bash
cargo check --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The automated tests do not require an RGB node or OpenAI API key.

HTTP contract tests open loopback sockets.

Coverage includes:

* wallet policy boundaries
* application approval
* execution-time revalidation
* failed-payment reservations
* duplicate-payment prevention
* durable persistence and locking
* HTTP DTO mapping

Real two-node settlement is covered separately by the recorded acceptance evidence.

---

# Repository layout

```text
crates/rgb402-core
    Domain types, legacy challenges, wallet policy

crates/rgb402-payment
    Wallet service, typed node client, configuration,
    and legacy simulated provider

crates/rgb402-merchant
    Axum merchant HTTP 402 API

crates/rgb402-agent
    Wallet agent tools, bounded agent loop,
    OpenAI provider, and legacy buyer

apps/merchant-server
    Demo merchant process

apps/buyer-agent
    Interactive agent, wallet CLI,
    and legacy demo buyer process

apps/wallet-ui
    Mobile-first Luma PWA

docs/
    Protocol, architecture, deployment,
    acceptance evidence, and roadmap notes
```

---

# Historical: RGB402 simulated demo

Luma began as **RGB402**, a deterministic experiment in machine-to-machine paid digital services.

The simulated buyer:

```text
requests a protected resource
        ↓
receives HTTP 402
        ↓
reads an RGB-style payment challenge
        ↓
checks a hard spending policy
        ↓
pays through a simulated provider
        ↓
retries with a payment identifier
        ↓
receives the resource
```

That flow remains in the repository because it captures the protocol idea that led to the current wallet and provides deterministic integration tests.

It should not be confused with Luma's newer real RGB Lightning payment path.

## Run the historical simulation

Terminal 1:

```bash
cargo run -p merchant-server
```

Terminal 2:

```bash
cargo run -p buyer-agent
```

Both processes use:

```text
.rgb402-sim-ledger.json
```

by default so the simulated settlement state is shared across terminals.

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

The historical integration tests cover:

* successful purchase
* too-expensive challenge
* wrong asset
* budget exhaustion
* fake receipt
* expired challenge

Run them with:

```bash
cargo test
```

---

## Historical Polar stub

The original simulated merchant/buyer flow retains its placeholder:

```text
RgbLightningPaymentProvider
```

The working wallet path instead uses:

```text
RgbLightningClient
```

with the upstream regtest stack.

Polar is not required for the current working wallet flow.

See:

[Roadmap](docs/roadmap.md)

---

# Project direction

Luma is evolving beyond a conventional Bitcoin wallet.

The current architecture demonstrates the components needed for **agentic commerce on Bitcoin**:

```text
wallet
  +
Bitcoin Lightning
  +
RGB assets
  +
merchant discovery / commerce
  +
machine-readable payment requests
  +
bounded agent tools
  +
wallet policy
  +
human authorization where required
```

The goal is not to let an AI model control a wallet.

The goal is to give people and software agents a common commerce interface where the **wallet remains the authority over money**.
