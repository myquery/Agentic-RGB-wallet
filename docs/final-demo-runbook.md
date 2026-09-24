# Final regtest demo runbook

This is the freeze/run path for the existing funded Alice/Bob environment. It
does not reset state, bootstrap another asset, open channels or pay an invoice.
See [manual acceptance](final-manual-acceptance.md), [deployment](hackathon-deployment.md),
[liquidity](demo-liquidity.md), and [evidence index](acceptance/final/README.md).

## Startup order

Run from the repository root. Keep existing healthy processes running. Before
restarting a wallet, let in-flight work finish and cancel pending reviews in the
application; use Ctrl-C in its owning terminal. Never delete a journal or lock.
The launcher acquires exclusive state locks; a duplicate process must fail.

1. Infrastructure (Docker/Compose, pinned node build, Bitcoin, Electrs, proxy,
   Alice and Bob node APIs):

   ```bash
   ./scripts/regtest/start.sh
   ./scripts/regtest/status.sh
   ```

   If existing nodes are locked, unlock them using the existing development
   helper below. Verify this is the existing `.var/regtest/data` installation
   first; the helper can initialize an absent wallet. Do not run bootstrap,
   reset, demo.sh or small_btc_channel.py --apply for this acceptance.

   ```bash
   python3 - <<'PY'
   import sys
   sys.path.insert(0, 'scripts/regtest')
   import regtest
   for node in ('alice', 'bob'):
       regtest.unlock(node)
       print(node + ': unlocked')
   PY
   ./scripts/regtest/status.sh
   ```

   `FailedBitcoindConnection` means fix the Docker RPC/network dependency first;
   do not recreate wallets. Bitcoin and Electrs heights should agree.

2. Build the current artifacts (no model calls):

   ```bash
   cargo build -p buyer-agent --bin api -p merchant-server --bin l402-merchant --bin recipient-service
   npm --prefix apps/wallet-ui ci
   npm --prefix apps/wallet-ui run build
   ```

3. Keep/start the existing public recipient tunnel in its own terminal:

   ```bash
   ngrok http 127.0.0.1:3050 --config "$HOME/.ngrok2/ngrok.yml" --inspect=false
   ```

   Use its actual HTTPS hostname. If it changes, set `RECIPIENT_DOMAIN` in each
   service terminal and update the public-only `.var/regtest/recipient-domain`
   file. Historical evidence and bound plans must retain their original domain.

4. Recipient service, separate terminal:

   ```bash
   export RECIPIENT_DOMAIN="$(cat .var/regtest/recipient-domain)"
   export RECIPIENT_ASSET_ID="$(python3 - <<'PY'
   import json, urllib.request
   request = urllib.request.Request('http://127.0.0.1:3101/listassets',
       data=b'{"filter_asset_schemas":["Nia"]}', headers={'Content-Type':'application/json'})
   with urllib.request.urlopen(request) as response:
       assets = json.load(response)['nia'] or []
   matches = [a['asset_id'] for a in assets if a.get('ticker') == 'R402USD']
   assert len(matches) == 1, 'Expected one R402USD asset; inspect configuration'
   print(matches[0])
   PY
   )"
   target/debug/recipient-service
   ```

   This read-only asset lookup uses the current node's asset rather than a
   historical asset ID. The service maps Alice to 3101 and Bob to 3102.

5. Merchant, separate terminal:

   ```bash
   set -a
   source config/l402.env.example
   set +a
   target/debug/l402-merchant
   ```

   Merchant binds 127.0.0.1:3040 and uses Bob's node at 3102. Preserve its existing
   `.var/regtest/l402-merchant.key`; never print or include it in evidence.

6. Alice wallet, separate terminal with your existing `OPENAI_API_KEY` exported:

   ```bash
   set -a
   source config/l402.env.example
   set +a
   ./scripts/wallet/api.sh alice
   ```

7. Bob wallet, separate terminal with your existing `OPENAI_API_KEY` exported:

   ```bash
   ./scripts/wallet/api.sh bob
   ```

   The launchers load `.var/regtest/wallet.env` and select independent node URLs,
   journal paths and wallet names. Do not run a CLI holding those same journals.
   No secret belongs in an example file or browser configuration. Optional
   `AGENT_MODEL` selects the provider model; current default is gpt-4.1-mini.

8. PWA is served by those APIs; no separate Vite service is needed:
   Alice http://127.0.0.1:3030 and Bob http://127.0.0.1:3031. Refresh after builds.

## Non-economic readiness and snapshots

```bash
python3 scripts/demo/check.py
python3 scripts/demo/check.py --snapshot docs/acceptance/final/before.json
# Human performs the documented UI scenario, then:
python3 scripts/demo/check.py --snapshot docs/acceptance/final/after.json
```

Use unique names such as `A-before.json` and `A-after.json` for each scenario.
Snapshots refuse overwrite. `--domain <hostname>` overrides the public domain
file. A failed check returns exit 1 and still saves the requested partial snapshot.
The helper uses GET only: wallet summary/activity, node identity/network/channels,
merchant health and public WebFinger. It does not fetch paid resources or request
invoices. It has no approval, send, reset, mining or journal-write operation.
Raw responses are never saved: fields are explicitly allowlisted, including
nested assets/policy. Sessions, model conversations, proofs, tokens and keys are
excluded. Snapshot files are sequential observations, not an atomic chain snapshot.

A passing check is necessary but does not prove route success, remaining daily
budgets, model quota or freshness of saved L402 credentials. Check current policy
in Settings and capture its actual values. 3 sats must be within the inclusive
machine auto limit; 50 must be above it but within the single/daily limits.
Existing purchases can have expired credentials: report `paid_resource_unavailable`
as a blocker rather than clearing the reservation or inventing a new resource URL.

## Validation

```bash
python3 -m unittest discover -s scripts/demo -p 'test_*.py'
python3 -m unittest discover -s scripts/regtest -p 'test_*.py'
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm --prefix apps/wallet-ui test -- --run
npm --prefix apps/wallet-ui run build
```

Local HTTP contract tests need socket access. Offline tests need no real model
key. Final browser acceptance is exclusively the human operator's task.
