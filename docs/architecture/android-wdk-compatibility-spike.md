# Android WDK/UTEXO compatibility spike

## Decision

**NO — exact blocker identified for the selected WDK/Bare integration.**

Luma cannot yet replace its per-wallet server RGB Lightning runtime with the tested embedded Android WDK/UTEXO runtime while claiming the Capsule A guarantees. The application and native artifacts build into an Android release APK, and the React Native host starts, but the Bare worklet aborts before WDK initialization because it cannot resolve a linked native addon. No wallet is created, so identity, balances, channel state, payments, restart recovery, VSS restore, and ownership fencing remain unproven.

This result is specific to the pinned WDK worklet route tested here. It does not show that an embedded RGB Lightning node is impossible on Android. It shows that this version combination is not usable as Luma’s wallet runtime without an upstream packaging/runtime fix and a repeat of the full experiment.

## 1. Exact WDK/UTEXO versions

All JavaScript packages are exact pins in the experiment lockfile.

| Component | Pin / revision |
| --- | --- |
| `@tetherto/wdk` | `1.0.0-beta.14`, npm git head `e7de0c0df7fac3a109101f55cf151c140c785360` |
| `@tetherto/wdk-wallet` | `1.0.0-beta.14`, npm git head `8688d0d118c7a73acd674e9bdae2cb9a4eb09187` |
| `@tetherto/wdk-secret-manager` | `1.0.0-beta.3`, npm git head `d3e357a3437b82d1bdb903f0bf968a1f8c825ead` |
| `@tetherto/wdk-react-native-core` | `1.0.0-beta.21` |
| `@tetherto/wdk-worklet-bundler` | `1.0.0-beta.14`, npm git head `2d76459ce1c7c6e14151348df26755394cb08c0d` |
| `@utexo/wdk-rgb-lightning` | `0.1.0-beta.15`, tag commit `4883283fb00a98db0251d5697a9eaa6edb870bb9` |
| `@utexo/rgb-lightning-node-bare` | `0.1.0-beta.15`, tag commit `f6c56cbf66a212d22ca7f8c2e4e4b03126a7061d` |
| embedded native `rgb-lightning-node` | `v0.10.0-beta.3`, commit `0bfa66fa256a6c36f3737d5b6402eacea40c68fc` |
| `react-native-bare-kit` | `0.15.5`; `0.14.5` tested as a diagnostic control |
| Expo / React Native | `54.0.33` / `0.81.5` |
| React / Hermes / New Architecture | `19.1.0` / enabled / enabled |
| MMKV / Nitro | `4.3.1` / `0.35.0` |
| Expo Crypto | `15.0.8`, selected through Expo 54’s compatibility manifest |

The official beta.15 Bare release identifies native RLN `v0.10.0-beta.3` and publishes the x64 artifact with SHA-256 `2e8c740bf4402da30739d97ab30bd8cded737284ee60b5fbde7e8503757cb200`. The experiment fetch script verifies that digest.

Primary upstream references: [WDK RGB Lightning module](https://docs.wdk.tether.io/sdk/community-modules/wdk-rgb-lightning/), [API reference](https://docs.wdk.tether.io/sdk/community-modules/wdk-rgb-lightning/api-reference/), [configuration](https://docs.wdk.tether.io/sdk/community-modules/wdk-rgb-lightning/configuration/), [VSS recovery guide](https://docs.wdk.tether.io/sdk/community-modules/wdk-rgb-lightning/guides/vss-backup-recovery/), [React Native quickstart](https://docs.wdk.tether.io/start-building/react-native-quickstart/), and [UTEXO Bare binding releases](https://github.com/UTEXO-Protocol/rgb-lightning-node-bare/releases/tag/v0.1.0-beta.15).

## 2. Android environment

| Item | Tested value |
| --- | --- |
| Host Node.js | `22.22.3` |
| Java | OpenJDK `21.0.11` |
| Emulator | Android Emulator `35.5.10.0`, build `13402964` |
| Android runtime | API 36, x86_64, clean cold boot |
| Android minimum SDK | 29 |
| Compile / target SDK | 35 / 35 |
| NDK | `27.1.12297006` |
| Build tools | 36.0.0 |
| Physical device | none attached; API 36 emulator was the strongest available runtime |

The native UTEXO beta.15 package publishes Android arm, arm64, and x64 assets. This spike intentionally packages the x64 artifact because that matches the available emulator. ARM hardware remains untested.

## 3. Build result

**PASS for compilation and APK packaging. FAIL for executable WDK runtime.**

The final release build completed `509` actionable Gradle tasks. The APK is `477,444,058` bytes with SHA-256 `46e7730e9655786fc409de32e1ca090e3b719073311e039652b8bd0d857b65ab`.

Two dependency issues were isolated during the build:

1. `expo-crypto@55.0.14` is incompatible with Expo 54’s native module core. The installed WDK React Native README directs applications to install the crypto version compatible with their Expo SDK. Expo 54’s own manifest pins `15.0.8`; using that version removes the startup crash.
2. The UTEXO package postinstall downloads every platform artifact, including several large static libraries. The experiment instead downloads only the official Android x64 beta.15 prebuild and verifies its published digest. This is reproducible through `scripts/fetch-android-x64-native.sh`.

The Android-only WDK worklet is `3,355,133` bytes, SHA-256 `8de63ea1e151c94078a2969bc44b3f0530f79ff7895e0ff51e32210d47df018d`.

## 4. State ownership map

The complete map is in [android-wdk-capsule-map.md](./android-wdk-capsule-map.md). The intended boundary aligns with Luma’s edge-owned design:

- Android OS-backed WDK storage owns the mnemonic credential.
- WDK derives signer material and hands it to an in-process VLS signer.
- native RLN owns Lightning, RGB, Bitcoin, channel, and node payment state under an application-private `dataDir`.
- optional VSS stores client-encrypted channel/RGB recovery data and enforces an ownership fence.
- Luma still owns application authorization, reservations, idempotency, reconciliation, and generation/epoch state.

Exact native file mappings for channel manager, channel monitors, monitor updates, BDK, and payment records are not exposed by WDK. Treating those internals as one native state boundary is valid; assuming Capsule A paths map one-to-one would be incorrect.

## 5. Capsule A to WDK mapping

The pinned module documents the main primitives needed by Capsule A:

- deterministic external-signer identity from the WDK mnemonic;
- one RLN/LDK node per persistent `dataDir`;
- native APIs for node info, BTC, RGB assets, channels, invoices, and payment records;
- idempotent `shutdown()`;
- optional encrypted VSS backup with snapshot versions;
- an ownership fence and explicit stale-fence takeover.

These are source-supported candidates. None crossed the Android runtime boundary in this spike, so they are not promoted to proven recovery behavior.

## 6. Wallet creation result

**BLOCKED.** The React Native activity launched and logged `Running "main"`. Approximately 2.18 seconds later, before WDK initialized or displayed the wallet creation screen, the Bare worklet aborted:

```text
ModuleTraverseError: ADDON_NOT_FOUND: Cannot find addon '.' imported from
file:///wdk-worklet.bundle/node_modules/bare-channel/binding.js
```

The package is not simply absent:

- the APK contains `lib/x86_64/libbare-channel.5.3.0.so`;
- the worklet manifest maps `bare-channel/binding.js` to `linked:libbare-channel.5.3.0.so`;
- the bundle was generated with `--host android-x64 --linked`;
- restricting the worklet to one Android host did not change the failure.

Bare then raises `SIGABRT` in the `bare-worklet` thread. With Bare Kit `0.14.5`, the minimum version accepted by WDK React Native Core beta.21, the same class of failure occurs at `bare-type/binding.js`. This control rules out a simple 0.15.5-only regression.

## 7. BTC result

**BLOCKED.** No wallet or native node existed, so no funding address was generated and no balance was asserted.

## 8. RGB result

**BLOCKED.** No disposable asset was sent and no asset balance was asserted.

## 9. Active-channel result

**BLOCKED.** The Android runtime never reached peer connection or channel creation. Interoperability with Luma’s isolated RLN peer is therefore unknown.

## 10. First payment result

**BLOCKED.** No payment was prepared, approved, submitted, or settled. No experiment journal entry was created. The diagnostic UI contains only the minimum native calls required for a later test; it must not be treated as payment evidence.

## 11. Restart and persistence result

**BLOCKED.** Application relaunch was exercised only at the host-shell level. Wallet identity, node identity, BTC, RGB, channel state, payment status, and journal continuity could not be measured because initialization never completed.

## 12. Restore and VSS result

**NOT PROVEN at runtime.** The official [VSS guide](https://docs.wdk.tether.io/sdk/community-modules/wdk-rgb-lightning/guides/vss-backup-recovery/) and pinned source state that:

- VSS mirrors LDK channel state and RGB wallet data in near-real-time;
- payloads use client-side XChaCha20-Poly1305 encryption with a key derived from the original mnemonic;
- `vssBackup()` forces a flush and returns a version;
- recovery occurs during initialization with the same seed, network, namespace/configuration, and a clean `dataDir`;
- `vssStatus()` is only a local session view and exposes no read-only server version;
- a held ownership fence rejects a second writer;
- `clearVssFence(password)` is a dangerous manual takeover that is safe only after the prior writer is dead.

No VSS server was configured because the runtime failed before wallet initialization. Active-channel recovery, rollback refusal, remote generation monotonicity, and two-device behavior remain unproven.

## 13. Duplicate-refusal result

**BLOCKED.** WDK node payment records do not replace Luma’s economic journal. The experiment never reached the point where a reservation or submission could be journaled, so no duplicate attempt was made.

## 14. Post-restore payment result

**BLOCKED.** Neither a first payment nor a restore occurred.

## 15. Mobile lifecycle results

| Transition | Result |
| --- | --- |
| foreground → background → foreground | **BLOCKED** before wallet runtime |
| process kill → restart | app shell relaunches; wallet invariants **NOT PROVEN** |
| network loss → reconnect | **BLOCKED** |
| Android reboot → restart | emulator cold boot worked; wallet invariants **NOT PROVEN** |
| app update/rebuild with persisted state | APK reinstall worked; no wallet state existed |

No battery or reconciliation measurement is meaningful because the native node never ran.

## 16. Single-owner and fencing result

**Documented, NOT PROVEN.** VSS supplies a native ownership fence only when VSS is configured. That can protect the native VSS state from two simultaneous writers. It does not fence Luma’s separate economic journal or bind an approval to one journal generation. Luma therefore still needs a writer epoch for its own state, and it must coordinate that epoch with native activation and VSS takeover.

Without VSS, WDK’s local one-node-per-`dataDir` rule is not a cross-device fence. The safe classification is: **Luma must provide fencing unless and until the VSS-backed activation path is proven end-to-end.**

## 17. Storage and startup measurements

| Observation | Value |
| --- | ---: |
| release APK | 477,444,058 bytes |
| Android x64 UTEXO `.bare` artifact | 158,991,120 bytes |
| packaged/stripped UTEXO x86_64 `.so` | 122,555,344 bytes |
| Android-only WDK worklet | 3,355,133 bytes |
| React Native main → native abort | about 2.18 seconds |
| fresh wallet state | **BLOCKED / not created** |
| state after channel/payment | **BLOCKED** |
| VSS payload | **NOT EXPOSED / not produced** |

The APK is a universal React Native/Bare build while the UTEXO node inside this tested artifact is x86_64 only. Production size must be measured from ABI-split release artifacts. No comparison with Capsule A’s roughly 203 KB and 222 KB generations is possible until a native wallet state directory exists.

## 18. Shared-infrastructure compatibility

The spike keeps endpoint configuration outside wallet identity and points the emulator only at the isolated capsule ports:

- Bitcoin RPC: `10.0.2.2:29443`;
- Electrum: `tcp://10.0.2.2:56001`;
- RGB proxy: `rpc://10.0.2.2:3400/json-rpc`.

No Alice/Bob/Carol endpoint or state directory is referenced. Source inspection classifies Bitcoin/indexer and RGB proxy endpoints as replaceable configuration. The seed/node identity is device-owned. Existing channels bind the wallet to their peer counterparties, and RGB assets bind it to their asset IDs/schema history. Actual endpoint replacement was **NOT PROVEN** because unlock was never reached.

## 19. Gaps from current Capsule A

| Capsule invariant | Android WDK result |
| --- | --- |
| same node identity after restart/restore | **BLOCKED** |
| same BTC state | **BLOCKED** |
| same RGB state | **BLOCKED** |
| active funded channel remains usable | **BLOCKED** |
| Luma journal continuity | **BLOCKED** |
| previous payment not duplicated | **BLOCKED** |
| fresh payment works after restart/restore | **BLOCKED** |
| stale state refusal | **NOT PROVEN**; VSS versions documented only |
| second active owner refusal | **NOT PROVEN**; VSS fence documented only |
| corrupt/incomplete state refusal | **NOT PROVEN** |
| uncertain submission reconciliation | **NOT SUPPORTED by WDK alone**; remains Luma-owned |

The success gate is unmet. Even the first condition, “WDK/UTEXO builds and runs on Android,” fails when “runs” means the wallet worklet initializes rather than merely launching an Activity.

## 20. Existing Luma regression results

The spike changed no file under `apps/` or `crates/`, no existing wallet API or journal, no startup script, and no Alice/Bob/Carol state.

| Check | Result |
| --- | --- |
| `python3 -m unittest tools/capsule/test_capsule.py` | 7 passed |
| `cargo fmt --all -- --check` | pass |
| `cargo check --workspace` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | 154 passed |
| spike `npm run typecheck` | pass |
| frontend tests | not run; frontend files were untouched |

**Existing Luma happy path: UNCHANGED**

**Android spike: additive and isolated**

## 21. Recommendation

Do not start production mobile migration on this WDK worklet stack yet. Report the reproducible `ADDON_NOT_FOUND` case upstream with the APK ABI listing, bundle manifest resolution, version matrix, and focused log in this report. Require an upstream release or documented packaging change that makes the beta.15 native node load on Android.

After that fix, rerun this same spike in order: wallet creation, stable identities, disposable BTC/RGB, active RGB channel, first approved/journaled payment, clean shutdown/restart, duplicate refusal, fresh payment, VSS destruction/restore, second-writer refusal, and corrupt/stale restore tests. Use a physical arm64 device as well as an emulator before making a production decision.

The architecture remains promising: it places wallet authority and native state at the edge and uses shared infrastructure for chain access, RGB transport, routing, liquidity, and encrypted backup. The tested implementation does not yet satisfy the minimum runtime gate.

### What remains Luma-owned

- immutable payment intent, human/application approval, and policy checks;
- durable reservation before submission and exactly-once submission authority;
- the local economic journal, journal sequence, and uncertain-submission reconciliation;
- capsule generation/manifest, cross-store consistency, stale/corrupt/incomplete restore refusal;
- the Luma writer epoch and coordination with device/VSS activation;
- compatibility checks across app, WDK, native RLN, peer/LSP, and schema upgrades.

### What WDK/UTEXO may own after runtime proof

- OS-backed mnemonic storage and deterministic signer derivation;
- in-process VLS signing and stable Lightning node identity;
- local native Lightning/RGB/Bitcoin state and node payment records;
- documented clean shutdown and reinitialization;
- encrypted VSS replication, native snapshot versions, and the VSS ownership fence.

At present these are delegation candidates, not validated production dependencies.
