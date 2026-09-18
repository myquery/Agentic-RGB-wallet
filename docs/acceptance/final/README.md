# Final acceptance index

Final manual A–E browser acceptance is **not executed by this freeze task**.
The [complete non-economic preflight](preflight-complete.json) passed all 26
checks against the existing environment. The earlier [initial snapshot](preflight.json)
is retained; neither run requested invoices or payments.
Use [the manual procedure](../../final-manual-acceptance.md) and copy the
[scenario template](scenario-template.md) for each scenario. Generated snapshots
are sanitized observations, not payment authorization or final acceptance.

Historical evidence is preserved at its original paths:

| Capability | Existing record | Scope |
| --- | --- | --- |
| Real RGB settlement | [RGB payment](../rgb-lightning-payment.json) | Earlier funded two-node payment |
| Natural-language payment | [Agent payment](../agent-rgb-lightning-payment.json) | Earlier provider/tool/approval flow |
| Human recipient | [Completed recipient run](../recipient-agent-completed.md) | Human-readable recipient flow |
| Two-wallet activity | [Wallet history](../two-wallet-history.json) | Same-hash sent/received evidence |
| L402 and rejection | [Latest L402 rerun](../l402-rerun-2026-09-17.md) | Earlier 3-sat settlement/resource and cancellation |
| BTC setup | [Activation](../btc-recipient-activation.json), [small BTC channel](../small-btc-channel.json) | Configuration/channel evidence, not payment acceptance |

The operator reported a successful fresh BTC transfer in conversation. Capture
authoritative hash/status and before/after evidence for final B; the report alone
does not provide that full evidence package. Historical domain, asset, balances
and timestamps may differ from the current environment; never rewrite them.

Suggested new files: `preflight.json`, `A-before.json`, `A-after.json`, `A.md`,
and equivalent B–E records. Review screenshots for secrets before saving them.
Use distinct filenames for reruns. Do not overwrite prior evidence.

Freeze validation: four demo-helper tests and six existing regtest-helper tests
passed; `cargo fmt --check`, Clippy with warnings denied, full workspace tests,
29 UI tests, production UI build, documentation-link checks and `git diff --check`
passed. The first Rust test run hit sandbox socket restrictions; the full rerun
with local socket permission passed. No application/payment semantics or UI
were changed during this documentation/helper task.
