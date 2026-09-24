# Capsule A to Android WDK/UTEXO state map

This map compares the proven Luma Capsule A responsibilities with `@utexo/wdk-rgb-lightning@0.1.0-beta.15`. “Documented” means the pinned module documentation or source states the behavior. It does not mean the Android spike proved it at runtime. Runtime work stopped before wallet creation because the Bare worklet aborted while resolving a linked native addon.

| Current responsibility | Current implementation | WDK/UTEXO equivalent | Persistence owner | Recoverable? | Evidence |
| --- | --- | --- | --- | --- | --- |
| seed / signing authority | mnemonic + `KeysManager` derivation | WDK secret manager owns the BIP-39 mnemonic; a derived 32-byte seed is attached to an in-process `NativeExternalSigner` | OS-backed WDK secure storage for mnemonic; volatile zeroizable binding buffer while active | Documented from the same mnemonic; **NOT PROVEN** on Android | Pinned module security section and `wallet-account-rgb-lightning.js`; worklet failed before wallet creation |
| Lightning node identity | derived node key | external VLS signer derives the node identity; RLN persists public `node_id`, xpubs, and master fingerprint and checks them on unlock | signer derivation plus native RLN `dataDir` public key-source data | Documented stable across restart; **NOT PROVEN** | Pinned module README and exact persisted-identity mismatch handling in `node-binding.js` |
| channel manager state | `.ldk/manager` | native RLN state beneath its one-node-per-`dataDir` boundary; exact file mapping is **NOT EXPOSED** by WDK | native RGB Lightning storage in application-private storage; optional encrypted VSS mirror | Documented as included in LDK channel state; **NOT PROVEN** | WDK API/configuration/VSS docs; no Android node initialized |
| channel monitors | `.ldk/monitors` | part of native RLN/LDK channel state; exact sub-store is **NOT EXPOSED** | native RLN `dataDir`; optional VSS | Documented collectively; **NOT PROVEN** | VSS docs say LDK channel state is mirrored; no subcomponent API |
| monitor updates | `.ldk/monitor_updates` | part of native RLN/LDK channel state; exact sub-store is **NOT EXPOSED** | native RLN `dataDir`; optional VSS | Documented collectively; **NOT PROVEN** | Same as channel monitors |
| RGB ownership state | `rgb_lib_db`, stash/state/index | RLN-owned RGB wallet/data inside the RGB Lightning node; separate from the optional on-chain WDK RGB module | native RLN `dataDir`; optional encrypted VSS | Documented as VSS-backed RGB wallet data; **NOT PROVEN** | Pinned module README explicitly requires separate `dataDir` values and says VSS mirrors RGB wallet data |
| Bitcoin wallet state | BDK DB | native RLN Bitcoin wallet state exposed through balance, transaction, UTXO, address, and sync methods; physical layout **NOT EXPOSED** | native RLN `dataDir` | Local persistence expected; VSS coverage specifically for BDK state is **UNKNOWN** | Account API and source; VSS wording names LDK channel and RGB wallet data, not a distinct BDK database |
| payment status/state | node payment records | `listPayments()` and `getPayment(hash, type)` backed by RLN | native RLN `dataDir` | Expected locally; backup inclusion is **NOT EXPOSED / NOT PROVEN** | Pinned account API only |
| active-channel RGB state | channel RGB state | RGB-bearing RLN channel state exposed through `listChannels()` and balances where the native API returns them | native RLN/LDK state; optional VSS | Documented as channel plus RGB state; **NOT PROVEN** | Account and VSS docs; runtime blocked |
| local payment journal | Luma journal | no equivalent that enforces Luma’s prepare/reserve/approve/submit-once contract | **Luma-owned** experiment/production store | Must be backed up and checked by Luma; **NOT IMPLEMENTED** because payment phase was blocked | WDK exposes node payment records, which do not replace application authorization/idempotency state |
| backup/recovery | Capsule A | local application-private persistence plus optional `vssBackup()`; restore happens during native initialization with the same seed/network/VSS namespace rather than through a separate restore API | Android private storage and/or remote encrypted VSS | Documented; **NOT PROVEN** | Official VSS guide and pinned source; no VSS runtime reached |
| single active owner | Luma epoch registry | VSS ownership fence when VSS is enabled; `clearVssFence(password)` is manual stale-owner takeover | VSS service/native client; Luma still owns its journal writer epoch | Documented fence; **NOT PROVEN**. Without VSS, Luma must provide fencing | Pinned README warns two live nodes can corrupt shared VSS state and says held fences reject restart |

## Trust boundary

```text
ANDROID DEVICE
  OS-backed WDK secret storage
    BIP-39 mnemonic / wallet credential
  React Native host
    Luma approval and journal boundary (future; not reached in spike)
  Bare worklet + WDK
    wallet manager, module dispatch, external-signer seed handoff
  native UTEXO RGB Lightning runtime
    VLS signer, node identity, LDK/RGB/Bitcoin state, payment records
  app-private dataDir
    authoritative local native wallet/node state

NETWORK
  replaceable configuration: Bitcoin RPC/indexer and RGB proxy
  channel-bound dependency: Lightning peer/LSP and channel counterparties
  optional recovery service: VSS encrypted blob store and ownership fence
  optional service dependency: LSP/routing/liquidity
```

The target boundary is structurally compatible with an edge-owned wallet: the seed, signer, and node state are intended to stay on the device, while chain access, RGB transport, routing/liquidity, and encrypted backup are shared services. The Android runtime result did not prove that boundary operationally.

## Ownership conclusion

Luma must continue to own the economic journal, immutable authorization binding, durable reservation, exactly-once submission gate, reconciliation, generation/epoch metadata, stale or incomplete restore refusal, and cross-store consistency checks. WDK/UTEXO is a candidate to own mnemonic protection, deterministic signer/node identity, native node persistence, node payment records, and encrypted VSS replication. Those candidates cannot be delegated safely in production until the Android native-addon blocker is fixed and the full recovery matrix passes.
