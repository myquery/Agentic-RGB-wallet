# Luma roadmap

Milestone 1 is complete: a real two-node RGB Lightning payment settled through
the wallet with human approval and verified balances. The reproducible regtest
launcher pins RGB-Tools/rgb-lightning-node at commit
[`e4008278`](https://github.com/RGB-Tools/rgb-lightning-node/tree/e4008278c80495ea8a8514580b899e771feff872).

## How the application is wired

The tracked [`regtest.py`](../scripts/regtest/regtest.py) script clones the exact
upstream node revision, builds its runtime, and provisions Bitcoin Core,
Electrs, the RGB proxy, and independent Alice/Bob/Carol nodes. Runtime artifacts
and wallet state are generated locally from this reproducible setup and remain
outside version control.

The tracked integration points show the complete boundary:

- [`api.sh`](../scripts/wallet/api.sh) assigns each wallet its node URL and
  independent durable journals.
- [`rgb.rs`](../crates/rgb402-payment/src/rgb.rs) and
  [`lightning.rs`](../crates/rgb402-payment/src/lightning.rs) implement the typed
  RGB Lightning node adapter.
- [`wallet.rs`](../crates/rgb402-payment/src/wallet.rs) and
  [`btc.rs`](../crates/rgb402-payment/src/btc.rs) persist payment reservations
  before submission and recover status without automatic resubmission.
- [`harness.rs`](../crates/rgb402-payment/src/harness.rs) binds human approval to
  immutable payment parameters.
- [`web/mod.rs`](../apps/buyer-agent/src/web/mod.rs) exposes the wallet API used
  by the PWA.

## Completed foundation

- Real Bitcoin regtest infrastructure and pinned RGB Lightning nodes.
- BTC Lightning and RGB asset balances, invoices, payments, and activity.
- Independent Alice, Bob, and Carol wallets and recipient discovery.
- Deterministic payment policy, explicit human approval, durable reservations,
  and status-based recovery.
- Agent planning that cannot authorize or execute a payment by itself.
- Merchant resource purchases and separate machine-payment policy.
- Installable wallet PWA and restartable local services.

## Post-hackathon milestone 1: recovery capsule

- Specify one versioned wallet capsule containing signing authority, Lightning
  manager and monitors, RGB ownership state, and Luma payment journals.
- Prove backup and restore with an active funded BTC/RGB channel on disposable
  regtest nodes.
- Reject stale snapshots and simultaneous writers.
- Exclude logs, credentials, and disposable caches from backups.

## Post-hackathon milestone 2: mobile runtime

- Evaluate a pinned WDK/UTEXO RGB Lightning release on Android and iOS.
- Keep keys in the device secret manager and run the RGB Lightning runtime in
  the wallet application.
- Add encrypted, versioned remote snapshots with one active device.
- Verify suspend, reboot, upgrade, offline, and low-storage behavior while
  channels have pending work.

## Post-hackathon milestone 3: shared services

- Operate redundant Bitcoin/indexer and RGB proxy providers.
- Introduce LSP and RGB liquidity capacity management with provider diversity.
- Add tenant-isolated discovery, notifications, agent inference, and encrypted
  backup services without giving them signing or approval authority.
- Measure storage per wallet, state per active channel, backup delta rate,
  routing demand, and asset liquidity before setting fleet-size targets.

The target remains non-custodial: shared services may route, index, transport, or
store encrypted state, while wallet keys and human payment authorization remain
on the user device.
