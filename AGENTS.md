# Project map

Six Rust workspace members: `rgb402-core` (domain/policy), `rgb402-payment`
(node clients, execution and persistence), `rgb402-agent` (model/tool gateway),
`rgb402-merchant` (protected resources), and the buyer/merchant applications.
The PWA lives in `apps/wallet-ui`.

## Core invariants

- Models propose/explain; application policy and human interfaces authorize.
- Bind authorization to immutable economic parameters. Flush a reservation before
  submission; never turn polling, restart or resource retry into another payment.
- Node settlement and verified resource responses establish completion.
- Keep RGB transfer and BTC machine-purchase policies separate. Machine automatic
  approval is inclusive and configured; human approval is never a model tool.
- Never expose credentials/proofs to models, traces, frontend configuration or docs.
- Preserve the eight-step agent limit and six workspace members.

## Major modules and deeper documentation

- `crates/rgb402-payment/src/harness.rs`: checked transitions and contract binding.
- `crates/rgb402-payment/src/{wallet,commerce,lightning,rgb}.rs`: execution, recovery and evidence.
- `crates/rgb402-agent/src/wallet_agent/{mod,observation,openai}.rs`: tools, trace and model context.
- `docs/agent-harness.md`: state machines, durable schema, invariant/test map.
- `docs/{architecture,agent,l402,pwa,regtest,node-api}.md`: relevant subsystem detail.
- `docs/acceptance/`: historical live evidence; consult only the relevant record.

Load subsystem documentation, then the exact task snapshot and required evidence.
Conversation history is not economic state.

## Recipient preparation

Use ordinary RGB preparation for direct invoices and the single recipient
preparation capability for `name@domain` plus explicit asset/base-unit amount.
Follow returned next actions; preparation is never human approval. Application
approval is authoritative. Execution and status use the existing wallet tools.
See `docs/recipient-agent.md` for orchestration and observation boundaries.
