# Reproduce a real RGB Lightning payment

Both the manual acceptance payment and a clean-reset `demo.sh` run succeeded on
11 September 2026. Alice paid Bob
**5 R402USD**, through this repository's `WalletService`, deterministic policy,
human confirmation, durable reservation and HTTP adapter. Wallet status was
`Settled`; Alice's outbound RGB allocation changed **500 → 495**, Bob's **100 → 105**.
See [the recorded evidence](acceptance/rgb-lightning-payment.json).

## Prerequisites

- Linux x86-64, Docker Engine running, Docker Compose **2.24.4 or newer**.
- Permission to access the Docker socket (a permission error is not a node failure).
- Rust/Cargo **1.94 or newer**, Git, Python 3, a C compiler and CMake.
  The observed build used Rust 1.98.1, two build jobs and disabled debug symbols.
- Internet access for the first checkout, locked Rust dependencies and Docker images.
- Several GB of free disk and approximately 8 GB available build memory.
- Free localhost ports listed below. Existing services are never stopped to free ports.

No Polar, OpenAI credentials, external RGB wallet or production secrets are required.
The node source, submodule and image pins are in
[rgb-lightning-node-version.md](rgb-lightning-node-version.md).

## Commands

From a fresh repository checkout:

```bash
./scripts/regtest/demo.sh
```

The first run clones the exact pinned upstream source into `.dev/rgb-lightning-node`,
builds it with `cargo build --locked`, builds an isolated runtime, starts the
upstream Bitcoin/Electrs/proxy services, and initializes both RGB nodes. Downloads
can take several minutes; progress is saved in `.var/regtest/logs/`.

It then funds each node with 1 regtest BTC, mines confirmations, creates RGB
allocation UTXOs, issues 1,000 units of `RGB402 Demo Dollar` (`R402USD`, precision 0),
and opens a direct 100,000-satoshi channel with 600 RGB units, pushing 100 to Bob.
It waits for the funding transaction and mines six confirmations, then requires
both nodes to report `ready`, `is_usable`, and `Opened`.

Bob creates a fixed-amount invoice for 5 demo units with a 3,000-satoshi regtest
carrier. Our CLI performs `decode`, `prepare` and `pay`. Review the displayed plan
and type **y** or **yes** when asked. Blank input, EOF or another answer cancels.
There is deliberately no payment `--yes` option. Policy requires human approval
for this payment; it cannot be bypassed by a script.

Expected final milestones:

```text
[1/5] Starting pinned infrastructure and nodes
[2/5] Bootstrapping funded wallets and usable RGB channel
[3/5] Creating Bob invoice; decoding and preparing through our wallet
[4/5] Paying through WalletService — review the exact plan and answer y/N
[5/5] Verifying settled status and exact RGB balance changes

RGB Lightning payment settled.
```

The JSON summary includes the generated asset ID, channel ID, amount, payment
ID/hash, settled status and both balance snapshots. IDs are generated dynamically;
the evidence file's asset ID is never used as a setup constant.

You can also use individual commands:

```bash
./scripts/regtest/start.sh       # Build/start; API readiness, not wallet unlock
./scripts/regtest/bootstrap.sh   # Start, unlock, fund, issue and establish channel
./scripts/regtest/status.sh      # Services, chain/index heights, usable channels
./scripts/regtest/demo.sh        # Prepare and interactively pay once, then verify
./scripts/regtest/verify.sh      # Verify the recorded attempt; never submit
./scripts/regtest/stop.sh        # Gracefully stop; preserve all node/wallet state
./scripts/regtest/reset.sh       # Print refusal and exact deletion scope
./scripts/regtest/reset.sh --yes # Delete only this project's development regtest state
```

`demo.sh` is deliberately idempotent after an attempt: subsequent runs verify the
recorded payment rather than paying again. For an entirely new demo, explicitly
reset and rerun. Reset deletes `.var/regtest` (including development wallet journals
and runtime evidence), but preserves the pinned checkout/build cache and the
tracked acceptance evidence. No command accepts an arbitrary cleanup directory.
Symlinked state parents are rejected. Lifecycle commands serialize with a process
lock; it releases automatically when the command exits.

## Ports and authentication

| Service | Host address | Internal address |
| --- | --- | --- |
| Alice API | 127.0.0.1:3101 | alice:3001 |
| Bob API | 127.0.0.1:3102 | bob:3001 |
| Alice Lightning peer | 127.0.0.1:19735 | alice:9735 |
| Bob Lightning peer | 127.0.0.1:19736 | bob:9735 |
| Bitcoin RPC | 127.0.0.1:28443 | bitcoind:18443 |
| Electrs | 127.0.0.1:55001 | electrs:50001 |
| RGB proxy | 127.0.0.1:3300 | proxy:3000 |

These alternate ports avoid existing services discovered on this host, including
3002 and 50001. Services extend the pinned upstream `compose.yaml` in the isolated
`rgb402-regtest` Compose project. Esplora is omitted because we use Electrum.

Authentication is disabled **only inside this localhost-published, regtest Docker
setup**. Upstream listens on all interfaces inside its container, which is why
these unauthenticated nodes are not run directly on the host. Node containers run
as the host UID/GID so bind-mounted state is writable. The hardcoded unlock
password is public, disposable development configuration; initialization responses
containing seeds are discarded. Do not fund these wallets on a real network.
The wallet's optional bearer-token support remains unchanged for authenticated nodes.

## Use the CLI directly

After bootstrap:

```bash
source .var/regtest/wallet.env
cargo run -p buyer-agent --bin wallet -- balance "$ALLOWED_ASSET_IDS"
```

The generated environment sets Alice's URL, the actual issued asset ID, a journal
under `.var/regtest`, and base-unit policy limits: auto threshold 1, maximum single
payment 100, maximum daily spend 500, carrier maximum 3,000,000 millisatoshis.
The carrier limit does not include node-managed routing fees.

`demo.sh` saves the invoice in `.var/regtest/invoice.json` and safe decoded details
in `.var/regtest/wallet-decode.json`. For a manual invoice:

```bash
cargo run -p buyer-agent --bin wallet -- decode "$RGB_INVOICE"
cargo run -p buyer-agent --bin wallet -- prepare "$RGB_INVOICE"
cargo run -p buyer-agent --bin wallet -- pay "$RGB_INVOICE"
cargo run -p buyer-agent --bin wallet -- status "$PAYMENT_HASH"
```

Do not manually pay the script's pending invoice and then create another script
attempt. The attempt journal and balance snapshots belong to one payment.
`verify.sh` can reconcile a completed or uncertain recorded attempt.

## Logs and troubleshooting

`.var/regtest/logs/` holds checkout/build logs, service stdout snapshots and
`wallet-audit.log`. Full node logs are also under each node's development data
directory. Live container logs:

```bash
docker compose -p rgb402-regtest -f .var/regtest/compose.yaml logs -f alice bob
```

| Symptom | Action |
| --- | --- |
| Docker unavailable / permission denied | Start Docker and enable socket access for your user. |
| Port already allocated | Inspect the listed localhost ports. Stop your conflicting service deliberately; scripts never stop unrelated services. |
| Upstream revision mismatch / dirty checkout | Inspect `.dev/rgb-lightning-node`; restore the documented pin. Scripts refuse silent updates. |
| Dependency fetch appears stalled | Inspect `logs/build.log`; Git CLI transport is enabled. First downloads can be slow. |
| Node cannot create log/data directory | Use the scripts' host UID/GID mapping; do not run bind-mounted nodes as upstream's hardcoded UID 1000. |
| `LockedNode` | Run `bootstrap.sh`; `start.sh` only establishes API readiness. |
| Insufficient BTC or RGB | Inspect balances and bootstrap logs. Setup uses regtest mining; never substitute real funds. |
| Channel not ready | Inspect funding transaction and both channel states. An accepted open request is not readiness. |
| Indexer lag | Compare Bitcoin/Electrs heights with `status.sh`; readiness polls state with a deadline. |
| Invoice expired or invalid | An unused invoice can be recreated. Never replace a reserved payment just because it timed out. |
| Pending/uncertain payment | Preserve the attempt and wallet journal. Run `verify.sh`/wallet status; do not resubmit. |
| Wallet API will not start | Confirm no wallet process still owns the journal; the kernel-held lock releases automatically after process exit or reboot. Inspect node status for reserved hashes and preserve the journal. |
| Failed/interrupted channel opening | Inspect `setup-progress.json` and logs; no automatic second open request is made after an uncertain attempt. Reset this disposable environment explicitly if necessary. |

The scripts poll API state with bounded deadlines; short pauses are polling
intervals, not fixed guesses about readiness. Repeated setup failure leaves
state and logs available for diagnosis. A nonzero result never claims settlement.

## Tests

The shell demo is the end-to-end acceptance test; it is not duplicated in a large
ignored Rust test. Default tests require no Docker or RGB node:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
python3 -m unittest discover -s scripts/regtest -p 'test_*.py'
```

The existing HTTP contract test opens a loopback socket and may need permission
outside a restricted sandbox. No wallet adapter changes were required by the
observed live API. The setup issues found were workspace nesting, occupied ports,
container UID/GID, slow first downloads, and the distinction between channel
funding acceptance and actual usability.
