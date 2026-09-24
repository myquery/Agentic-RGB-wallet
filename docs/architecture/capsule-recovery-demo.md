# Capsule A real active-channel recovery demonstrator

Status: **PROVEN for clean-stop Capsule A recovery; PARTIALLY PROVEN overall**.
Date: 2026-09-24.

## Existing happy-path impact

**Existing happy path: UNCHANGED. Capsule demonstrator: additive and isolated.**

No application, payment, journal, node API, node storage format, normal regtest
script, frontend, or startup file changed. Normal Luma startup has no capsule,
generation, lease, epoch, snapshot, or restore dependency. The control flow is
still:

```text
prepare → durable reservation → human/application approval
        → execute exactly once → settlement/reconciliation
```

All new behavior is under `tools/capsule` and requires a marked disposable
fixture. The final regression run is recorded below.

## 1. Objective and result

A real disposable Luma wallet with a funded active RGB Lightning channel was
cleanly stopped, snapshotted, deleted, restored, reconciled, and used for a new
settled payment. It then committed generation 2. **PROVEN.**

This proves the clean-stop experimental boundary. It does not prove crash-time
atomicity while LDK, RGB, or journal writes are in progress.

## 2. Disposable topology

The fixed namespace was `.var/regtest/capsule-test/`, marked by
`.luma-capsule-disposable`. The `luma-capsule-test` Compose project ran its own:

- Bitcoin Core and miner wallet;
- Electrs indexer;
- RGB proxy;
- `capsule-test-alice` and `capsule-test-bob` RGB Lightning nodes;
- node roots, peer identities, channel, journals, ports, and chain.

It did not share the control group's chain or Docker network. Destructive
commands require `--yes`, the fixed marker, and the fixed namespace. The capsule
kernel rejects unmarked/cross-fixture paths, normal `.var/regtest` paths,
Alice/Bob/Carol wallet IDs, and exact Alice/Bob/Carol path components.

One procedural deviation occurred during initial mapping: the existing
regtest Compose configuration was viewed read-only to understand image and port
conventions. No funded wallet API, node data, channel, journal, credential, or
wallet configuration was queried or copied, and no control-group process or
file was changed. The real experiment itself used only the isolated project.

## 3. Real fixture baseline

The fixture created one RGB asset, funded both nodes, opened one 100,000-sat RGB
channel with non-zero allocation, and settled one application-approved 5-unit
RGB payment through the existing wallet binary. Luma journal sequence was 1.
Committed evidence contains only SHA-256 hashes of the wallet fingerprint, node
ID, channel ID, and asset ID.

## 4. Manifest v1 and allowlist

Each source file is an explicit component; there is no whole-directory archive
or blacklist fallback. Generation 1 contained 44 components/44 files and
182,784 component bytes. Generation 2 contained 57 components/57 files and
195,895 component bytes. Each generation classified one authority component,
two recovery metadata components, three application journals, and all remaining
node/RGB/LDK objects as safety-critical.

The allowlist conservatively included encrypted key material, fingerprint and
indexer metadata, manager, monitors and monitor updates, payment/sweeper/channel
state, RGB channel/transfer objects, RGB and BDK databases, stash/state/index,
and Luma RGB/BTC/machine journal files. Empty optional journals were retained.
No minimization was attempted.

Logs, log directories, `.log`, `.lock`, sessions, conversations, diagnostics,
symlinks, and unsafe paths are rejected. Both real manifests contained zero
forbidden paths. Validation also rejects undeclared files anywhere under the
capsule state root.

## 5. Clean-stop snapshot boundary

No wallet API server was running for the disposable wallet; each payment CLI
process had exited and released its journal lock. Known payment status was
reconciled as `Settled`. The harness then sent Docker `SIGTERM` with a 60-second
grace period and required container state `Running=false` before reading files.

The pinned node handles `SIGTERM`, waits for an in-progress state change, calls
`stop_ldk`, and only then completes shutdown (`.dev/rgb-lightning-node/src/main.rs`,
`shutdown_signal`). This establishes a practical clean process boundary. It
does not provide a formal distributed transaction joining LDK, RGB, and Luma
journal writes. **Clean stop: PROVEN. Concurrent-write snapshot: NOT PROVEN.**

## 6. Commit and generation model

The writer epoch is checked before copying every component. Files enter a
partial generation, are hashed, and receive an fsynced manifest. `COMMITTED` is
written before atomic directory rename, then the registry conditionally advances
the generation. A partial directory cannot validate.

Generation 1 recorded journal sequence 1. After restore and a second settled
payment, generation 2 recorded `previous_generation=1`, `generation=2`, the same
writer epoch, and journal sequence 2. **PROVEN.**

## 7. Destruction and restore

After generation 1 committed, the harness removed only disposable Alice's node
root and Luma journals. The capsule and independent peer/chain infrastructure
remained. Restore validated the marker, schema, current generation, epoch,
component set, sizes, hashes, and `COMMITTED`; copied into a partial target;
rechecked epoch; atomically installed the target; and started the restored node.

The observed recovery state machine was:

```text
CAPSULE_FOUND → VALIDATING → RESTORING → RECONCILING → READY
```

No economic operation ran before the node unlocked and the restored channel
reported `Opened`, `ready`, and `is_usable`.

## 8. Identity, BTC, RGB, and active channel

Before/after values were read from the node, not the UI. Exact wallet
fingerprint and Lightning node ID matched. The full BTC balance response and
full RGB asset-balance response matched. Network remained regtest. Channel ID,
funding outpoint, peer public key, capacity, and asset ID matched; capacity was
100,000 sats. The channel returned to usable state automatically after unlock,
showing that restored peer/channel state reconnected without a new channel.

The aggregate RGB balance and snapshotted channel state were exact. The harness
did not separately persist the pre-restore `asset_local_amount` and
`asset_remote_amount` response fields in redacted evidence, so that narrower
per-channel presentation comparison is **PARTIALLY PROVEN**; file hashes and the
full RGB balance equality cover the authoritative restored state.

## 9. Journal continuity and duplicate protection

The restored RGB journal matched its pre-snapshot SHA-256 and sequence 1. Paying
the same invoice through the existing wallet idempotency path returned failure
and did not advance the journal. No second node submission path was invoked by
the wallet. A fresh, application-approved payment then settled, advanced the
journal to sequence 2, and reduced restored Alice's outbound RGB balance by the
exact 5-unit amount. **PROVEN at the Luma boundary.**

## 10. Real stale generation and writer epoch

After generation 2 contained the newer payment state, validation of generation
1 returned `STALE`. It could not enter `READY`. **PROVEN.**

Runtime B was refused while runtime A owned epoch 1. Explicit takeover advanced
the registry to epoch 2. Runtime A's subsequent snapshot attempt returned
`SPLIT_BRAIN` before state copying. No two runtimes were allowed to mutate the
channel. **PROVEN at the local capsule boundary.** Direct node APIs are not
cryptographically fenced by this demonstrator.

## 11. Cross-store skew and corrupt state

Real generation copies were used. Each of these refused validation:

- generation-2 node state plus generation-1 Luma journal;
- generation-1 node state plus generation-2 Luma journal, with stale checking
  bypassed only to isolate the hash/skew assertion;
- generation-2 manifest plus generation-1 manager;
- missing manager, monitor, monitor update, RGB database, or Luma journal;
- corrupt manifest, wrong component hash, missing `COMMITTED`, or undeclared
  extra state file.

All returned `CORRUPT` except missing `COMMITTED`, which returned `INCOMPLETE`.
No fallback/default node startup was attempted. **PROVEN.**

## 12. Status-only reconciliation

Settled status was queried after restore and before continuing. The application
never retried an uncertain payment. Creating an actual “submission may have
occurred but response was lost” boundary is **BLOCKED — no safe
submission-boundary hook**. Production payment semantics were not modified to
manufacture this state.

## 13. Measurements

These are observations from one disposable fixture and must not be extrapolated:

| Measurement | Observed |
|---|---:|
| Active channels | 1 |
| RGB assets | 1 |
| Settled payments | 2 |
| Node directory after recovery | 359,068 bytes |
| LDK state | 84,164 bytes |
| RGB-named state files | 154,848 bytes |
| BDK files in capsule | 5,248 bytes |
| Transfer artifacts | 13,071 bytes |
| Luma RGB journal after payment 2 | 1,582 bytes |
| Generation 1 capsule | 202,802 bytes |
| Generation 2 capsule | 222,333 bytes |
| Generation 1 snapshot | 0.032 seconds |
| Generation 2 snapshot | 0.040 seconds |
| Generation 1 restore copy | 0.016 seconds |
| Generation 2 restore copy | 0.024 seconds |
| Generation 2 start/unlock/reconcile | 17.357 seconds |

Capsule sizes exclude logs, locks, and diagnostics. Node-directory size is
reported separately and may include runtime-created diagnostic files.

## 14. Regression results

The capsule tests, format check, workspace check, Clippy, and full workspace
tests were run after implementation. The frontend was not touched.

| Check | Result |
|---|---|
| `python3 -m unittest tools/capsule/test_capsule.py` | 7 passed |
| `cargo fmt --all -- --check` | PASS |
| `cargo check --workspace` | PASS |
| `cargo clippy --workspace --all-targets -- -D warnings` | PASS |
| `cargo test --workspace` | 154 passed, 0 failed |

The first restricted test run denied loopback listener creation; rerunning the
unchanged suite with loopback permission passed. Existing production source is
unchanged from the pre-milestone commit.

## 15. Required evidence table

| Invariant | Result | Evidence |
|---|---|---|
| same node identity after restore | **PROVEN** | exact pre/post node ID and wallet fingerprint equality; committed hashes |
| same BTC state | **PROVEN** | exact authoritative `btcbalance` response equality |
| same RGB state | **PROVEN** | exact authoritative `assetbalance` response equality and component hashes |
| active funded channel survives | **PROVEN** | same ID/outpoint/peer/capacity; `Opened`, ready, usable; new payment |
| same journal restored | **PROVEN** | exact journal SHA-256 and sequence 1 |
| previous payment not duplicated | **PROVEN** | duplicate wallet attempt failed; journal stayed sequence 1 |
| new payment succeeds after restore | **PROVEN** | status `Settled`; exact 5-unit outbound delta; journal sequence 2 |
| restored wallet creates N+1 capsule | **PROVEN** | generation 2, previous 1, same writer epoch |
| stale real N rejected | **PROVEN** | generation 1 returned `STALE` after real payment 2 |
| cross-store skew rejected | **PROVEN** | old/new node/journal swap variants returned `CORRUPT` |
| second writer rejected | **PROVEN** | runtime B returned `SPLIT_BRAIN` before takeover |
| explicit takeover increments epoch | **PROVEN** | epoch 1 → 2 |
| old writer rejected | **PROVEN** | runtime A snapshot returned `SPLIT_BRAIN` before copying |
| corrupt/incomplete real capsule rejected | **PROVEN** | ten named real-state variants refused |
| uncertain payment reconciles without retry | **BLOCKED** | no safe submission-response boundary hook |

The redacted machine-readable record is
`capsule-recovery-evidence/real-active-channel-2026-09-24.json`.

## 16. Security and scope limitations

The manifest uses SHA-256 but is not authenticated with a signature or MAC. The
epoch registry is local and not a distributed or cryptographic lease. Capsule
plaintext remains local; no encryption or VSS was implemented. A direct caller
could bypass the Luma epoch and call the node API, so production fencing remains
future work. Seed-only recovery detection remains blocked without an
authenticated external record that the identity previously held channel/RGB
state.

## 17. Decision

**Yes.** Evidence shows that a real Luma RGB Lightning wallet with an active
funded channel can be represented as a versioned recoverable Capsule A and can
resume safe payments after full destruction and clean-stop restore. The proven
scope includes identity, BTC/RGB state, active-channel usability, Luma journal
continuity, duplicate refusal, a new settled payment, generation progression,
stale refusal, skew/corruption refusal, and local writer fencing.

There is no demonstrated recovery/state reason that prevents attempting this
same lifecycle with one embedded Android WDK/UTEXO wallet. The active-channel
gate passed, so an Android compatibility spike is now justified. That spike
must reproduce these invariants; it must not treat this clean-stop result as
proof of mobile crash atomicity, authenticated remote backup, or production
multi-device fencing. The missing safe uncertain-submission hook remains a test
gap, not a reason to redesign the existing happy path.
