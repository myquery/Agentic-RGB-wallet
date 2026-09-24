# Capsule A Recovery Demonstrator

Status: **PARTIALLY PROVEN**. Date: 2026-09-24. This demonstrator did not read,
stop, copy, or modify the funded Alice/Bob/Carol environment.

## Existing happy-path impact

**Existing happy path: UNCHANGED. Capsule demonstrator: additive and isolated.**

No file under `apps/`, `crates/`, the existing regtest scripts, node storage,
wallet startup, payment APIs, configuration, or journal formats changed. Normal
startup does not load this tool and has no capsule, generation, lease, epoch,
snapshot, or restore dependency. A source diff from the pre-constraint
demonstrator commit confirms the existing application source tree is identical.

The control flow remains:

```text
prepare → durable reservation → human/application approval
        → execute exactly once → settlement/reconciliation
```

The post-change regression run passed formatting, workspace compilation,
Clippy with warnings denied, and all 154 Rust tests. An initial sandboxed test
run denied five loopback listener creations; rerunning the unchanged suite with
loopback permission passed all five and the entire workspace. The frontend was
not touched. The six capsule-only Python tests also pass.

## 1. Objective

Prove the local safety machinery required to treat node state and Luma journals
as one versioned consistency boundary. The prototype is in `tools/capsule`.
Real active-channel recovery is deliberately not claimed.

## 2. Disposable topology

The automated fixture uses temporary directories containing synthetic manager,
monitor, RGB-journal, and BTC-journal objects. **PROVEN:** destructive tests are
isolated from `.var/regtest`. **NOT PROVEN:** a funded node topology has not yet
been executed.

Every operation requires a `.luma-capsule-disposable` marker created only in an
empty directory whose name contains `capsule-test`. Repository, spec, sources,
generation, and restore target must share that marked root. Alice, Bob, and
Carol path components and wallet IDs are rejected in code. The normal
`.var/regtest` tree is rejected unless the path is under the dedicated
`.var/regtest/capsule-test/` namespace.

## 3. Capsule Manifest v1

The JSON manifest records schema version, wallet ID, hashed/public fingerprint
and node ID supplied by the fixture, network, project/node revisions, writer
epoch, generation, previous generation, UTC creation time, and an explicit map
of components. Every component records classification, destination, kind, total
size, optional journal sequence, and each file's relative path, size, and
SHA-256 digest. Secret fields are not accepted by the spec schema.

## 4. Allowlisted contents

The input spec must enumerate every component and classify it as `AUTHORITY`,
`SAFETY-CRITICAL`, `APPLICATION-JOURNAL`, or `RECOVERY-METADATA`. There is no
archive-the-root fallback. Logs, `.log`, `.lock`, log/session/conversation
directories, symlinks, unsafe paths, and absent sources fail closed. Uncertain
transfer artifacts should be named as safety-critical components.

## 5. Snapshot procedure

The caller must first stop new preparation, reconcile in-flight work, flush
journals, and establish node quiescence. The tool then checks the writer epoch,
copies each allowed component to a partial generation, hashes the copies, writes
and fsyncs the manifest, writes `COMMITTED`, atomically renames the generation,
and conditionally advances the registry. **BLOCKED:** the pinned node exposes no
formal cross-store quiescence barrier; the tool cannot truthfully create one.

## 6. Commit and generation model

Generations are monotonic per wallet. A partial directory is invalid. A
generation is valid only with a readable v1 manifest, all declared objects and
hashes, a `COMMITTED` marker, and equality with the registry's current
generation. **PROVEN** by synthetic tests.

## 7. Writer epoch model

The local JSON registry is serialized with `flock` and atomically replaced.
Acquire rejects a different owner; explicit takeover increments the epoch; all
snapshot and restore boundaries recheck owner and epoch. **PROVEN at the
demonstrator boundary.** It is not cryptographic and does not fence direct node
API access or a process on another host.

## 8. Restore procedure

Validate the committed marker, schema, durable current generation, epoch,
paths, complete file set, sizes, and hashes. Require the active writer epoch and
an empty target. Copy to a partial target, recheck the epoch, then atomically
rename. The application must reconcile node status before entering `READY`.

## 9. Active-channel recovery result

**NOT PROVEN.** No funded active channel was destroyed or restored. The exact
blocker is the absence of a tested quiescence contract spanning the pinned
node's LDK/RGB persistence and Luma's journals. A clean process stop is the
safest candidate boundary for the disposable experiment, but it still requires
end-to-end validation.

## 10. Crash matrix

| Crash point | Expected persisted state | Resubmitted? | Result |
|---|---|---:|---|
| partial generation before manifest | no committed capsule | no | **PROVEN** marker required |
| after manifest, before `COMMITTED` | incomplete | no | **PROVEN** marker required |
| after generation rename, before registry advance | generation is not current | no | **PARTIALLY PROVEN** validation rejects it as stale |
| reservation before node submission | journal reservation | no | **NOT PROVEN** with real wallet |
| submission before response | reservation plus node status | no | **NOT PROVEN** with real wallet |
| monitor/RGB persistence | implementation-specific | no | **BLOCKED** pending safe node hooks |

## 11. Stale-generation test

Generation 1 was created, journal state advanced, and generation 2 committed.
Activation of generation 1 returned `STALE`. **PROVEN** against the durable
local registry.

## 12. Cross-component skew

Every file is bound to one manifest by size and hash; swapping a component from
another generation produces `CORRUPT`. **PARTIALLY PROVEN:** hash corruption is
tested, but a dedicated complete component-swap matrix and authenticated
manifest are pending.

## 13. Concurrent-writer test

Writer B was rejected while A held epoch 1. Explicit takeover gave B epoch 2,
then A's snapshot was rejected as `SPLIT_BRAIN`. **PROVEN at the Luma capsule
boundary.** Node-level fencing remains **NOT PROVEN**.

## 14. Corrupt and incomplete capsules

Manager modification was rejected by hash validation and removal of
`COMMITTED` was rejected as incomplete. Forbidden logs fail snapshot creation.
Missing-monitor, missing-RGB-DB, truncated-manifest, and unexpected-file paths
use the same validation mechanisms but need named tests. **PARTIALLY PROVEN.**

## 15. Seed-only negative test

**BLOCKED.** The current application has no durable external fact declaring
that a mnemonic previously controlled channel/RGB state. The demonstrator will
not invent readiness from a seed. Production activation needs the capsule
registry (or another authenticated recovery record) to mark seed-only recovery
as `MANUAL_RESCUE_REQUIRED`.

## 16. Measurements

The synthetic fixture is intentionally too small to support useful storage or
duration conclusions. The manifest records component byte sizes and journal
line sequences. Export/restore duration, channel/payment/transfer counts, and
real LDK/RGB size measurements await the disposable funded fixture.

## 17. Proven invariants

- Explicitly allowlisted inputs; forbidden diagnostic state fails closed.
- A partial or uncommitted generation cannot activate.
- File loss/change and undeclared files cannot activate.
- An older durable generation cannot activate.
- A second owner cannot acquire without explicit takeover.
- Takeover increments epoch and invalidates the old writer at the kernel edge.
- Restore uses a partial target and rechecks writer ownership.
- Unmarked, live-wallet-named, and cross-fixture paths are rejected before use.

## 18. Failed or blocked invariants

Same node identity, BTC/RGB ownership, active channel, prior payment state, and
post-restore payment are **NOT PROVEN**. Unknown submission reconciliation and
monitor/RGB crash boundaries are **NOT PROVEN**. Cross-host/cryptographic
fencing is **BLOCKED** by the local-only registry design.

## 19. Security limitations

SHA-256 detects accidental/copy corruption but the manifest is not signed or
MACed. IDs supplied by a fixture are not independently queried from the node.
The registry is suitable only for a local demonstrator. Snapshot plaintext must
remain on an encrypted disposable volume; this prototype does not implement
encryption, VSS, remote leases, authorization, or secret scanning.

## 20. Recommendation

Next, add a dedicated two-node regtest namespace and a clean stop/start adapter,
then run active-channel round-trip recovery without sharing ports, volumes,
keys, peers, or journals with Alice/Bob/Carol. Add explicit payment-boundary
hooks before attempting the crash matrix. Do not begin the Android WDK spike
until same-identity active-channel recovery and status-only payment recovery are
proven.

## Recovery state machine

```text
UNINITIALIZED → CAPSULE_FOUND → VALIDATING → RESTORING → RECONCILING → READY
                         ↘ STALE | CORRUPT | INCOMPATIBLE | SPLIT_BRAIN
                                  | INCOMPLETE | MANUAL_RESCUE_REQUIRED
```

Economic operations are permitted only in `READY`. The current CLI implements
validation and restore primitives; application/node reconciliation and the
transition into `READY` remain integration work.

## Current success criteria

- [ ] same node identity after real restore — **NOT PROVEN**
- [ ] same BTC state — **NOT PROVEN**
- [ ] same RGB state — **NOT PROVEN**
- [ ] active funded channel survives — **NOT PROVEN**
- [x] synthetic Luma journals survive restore — **PROVEN**
- [ ] real previous payment is not duplicated — **NOT PROVEN**
- [ ] new real payment succeeds — **NOT PROVEN**
- [ ] uncertain real submission reconciles — **NOT PROVEN**
- [x] stale generation rejected — **PROVEN**
- [~] component skew rejected — **PARTIALLY PROVEN**
- [~] corrupt/incomplete capsule rejected — **PARTIALLY PROVEN**
- [x] second writer rejected — **PROVEN at kernel boundary**
- [x] takeover increments epoch — **PROVEN at kernel boundary**
- [x] old writer rejected after takeover — **PROVEN at kernel boundary**

The milestone question therefore remains open: the capsule consistency and
single-owner mechanics work locally, but the exact wallet lifecycle cannot move
to an embedded mobile runtime until the active-channel and cross-store
quiescence experiments pass.
