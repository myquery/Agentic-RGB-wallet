// RgbLightningScreen — RGB-over-Lightning surface via the same
// `useAccount().extension()` pattern as RgbScreen.
//
// Talks to `WalletAccountRgbLightning` (in @utexo/wdk-rgb-lightning)
// inside the worklet through the generic `callMethod` HRPC dispatch.
// Every `ext.<method>(...)` call lands on the same-named method of
// the wallet account, which delegates to rgb-lightning-node-bare,
// which calls into the Rust C-FFI. Boot-up is two-step:
//
//   1. WalletGate creates/unlocks the WDK wallet (BIP-39 mnemonic).
//   2. THIS screen calls `ext.unlock({ ... })` on the account once,
//      passing bitcoind / indexer / proxy connection details. That
//      brings LDK + tokio fully online and is required before any
//      LN operation works.
import React, { useCallback, useEffect, useRef, useState } from 'react'
import {
  ActivityIndicator, Alert, Platform, ScrollView,
  StyleSheet, Text, TextInput, TouchableOpacity, View
} from 'react-native'
import * as Clipboard from 'expo-clipboard'
import { useAccount } from '@tetherto/wdk-react-native-core'

// Android emulator's `localhost`/127.0.0.1 is the emulator itself, NOT the
// host. The host-machine alias from inside the AVD is `10.0.2.2`. iOS sim
// shares the host's loopback so 127.0.0.1 works there.
const HOST_LOOPBACK = Platform.OS === 'android' ? '10.0.2.2' : '127.0.0.1'

// Mirror of WalletAccountRgbLightning.PENDING_ADDRESS — kept verbatim
// here so the screen can detect it before the node is unlocked.
const PENDING_ADDRESS = 'tb1qpendingunlock00000000000000000000000000'

const colors = {
  background: '#121212', primary: '#FF6501',
  text: '#fff', textSecondary: '#999', textTertiary: '#666',
  card: '#1E1E1E', border: '#333',
  success: '#4CAF50', danger: '#FF3B30', warning: '#FF9500', error: '#FF6B6B'
}

// Method surface forwarded by the extension proxy. Mirrors
// WalletAccountRgbLightning — see @utexo/wdk-rgb-lightning.
type LnExt = {
  unlock: (req: Record<string, unknown>) => Promise<unknown>
  shutdown: () => Promise<void>
  // RLN's on-chain receive address. The hook-level `address` field on
  // `useAccount` is loaded once pre-unlock (returns PENDING_ADDRESS until
  // the node is online) and isn't auto-refetched after unlock — so we
  // pull it explicitly through the extension proxy here.
  getAddress: () => Promise<{ address: string } | string>
  getNodeInfo: () => Promise<{
    pubkey?: string
    num_peers?: number
    num_channels?: number
    num_usable_channels?: number
    block_height?: number
  }>
  getNetworkInfo: () => Promise<{ network?: string; height?: number }>
  sync: () => Promise<unknown>
  getBalance: (skipSync?: boolean) => Promise<string>
  getBalanceDetails: (skipSync?: boolean) => Promise<{
    vanilla?: { settled?: number; future?: number; spendable?: number }
  }>
  // RGB-LN channels require the wallet to pre-allocate colorable UTXOs
  // before openchannel; called automatically inside `onOpenChannel`
  // ahead of opening a channel, matching Yurii's reference flow.
  createUtxos: (req: { up_to?: boolean; num?: number; size?: number; fee_rate: number; skip_sync?: boolean }) => Promise<unknown>
  connectPeer: (peerPubkeyAndAddr: string) => Promise<unknown>
  listPeers: () => Promise<{ peers?: Array<{ pubkey?: string; address?: string }> }>
  openChannel: (req: Record<string, unknown>) => Promise<{
    temporary_channel_id?: string
    channel_id?: string
  }>
  closeChannel: (req: { channel_id: string; peer_pubkey: string; force: boolean }) => Promise<unknown>
  // RLN's `list_channels()` returns `sequence<Channel>` — a bare JSON
  // array, not `{ channels: [...] }`. (The HTTP-route layer in the
  // daemon wraps in an object; the UniFFI / C-FFI surface does NOT.)
  listChannels: () => Promise<Array<{
    channel_id?: string
    peer_pubkey?: string
    asset_id?: string | null
    counterparty?: string
    capacity_sat?: number
    local_balance_msat?: number
    local_balance_sat?: number
    ready?: boolean
  }>>
  createInvoice: (req: Record<string, unknown>) => Promise<{
    invoice?: string
    payment_hash?: string
    expiry?: number
  }>
  decodeInvoice: (invoice: string) => Promise<unknown>
  sendPayment: (req: Record<string, unknown>) => Promise<{
    payment_hash?: string
    status?: string
  }>
  listPayments: () => Promise<{ payments?: unknown[] }>
  // RGB asset surface
  issueAssetNia: (req: { amounts: number[]; ticker: string; name: string; precision: number }) => Promise<{
    asset_id?: string
    ticker?: string
    name?: string
    precision?: number
  }>
  listAssets: (filterAssetSchemas?: string[]) => Promise<{
    nia?: Array<{ asset_id?: string; ticker?: string; name?: string; balance?: { settled?: number; spendable?: number } }>
    cfa?: unknown[]
    ifa?: unknown[]
    uda?: unknown[]
  }>
  getAssetBalance: (assetId: string) => Promise<{
    settled?: number; future?: number; spendable?: number
    offchain_outbound?: number; offchain_inbound?: number
  }>
  refreshTransfers: (req: { asset_id?: string; skip_sync?: boolean }) => Promise<unknown>
  createRgbInvoice: (req: { min_confirmations: number; asset_id?: string }) => Promise<{
    recipient_id?: string
    invoice?: string
    expiration_timestamp?: number
  }>
}

export function RgbLightningScreen () {
  const ln = useAccount<LnExt>({ network: 'rgb-lightning', accountIndex: 0 })
  const { address, isLoading, error, account, extension } = ln
  // `extension()` constructs a fresh `Proxy` on every call, so calling
  // it inline on each render produced a new `ext` reference per render —
  // which busted every `useCallback` downstream (`guard`, `refreshAll`),
  // re-firing the auto-refresh `useEffect` in a tight loop and tripping
  // React's update-depth limit. Memoise it on `account` instead.
  const ext = React.useMemo<LnExt | null>(
    () => (account ? extension() : null),
    [account, extension]
  )

  // ─── Unlock state ─────────────────────────────────────────────────────
  // RLN's `unlock` is a one-time bring-online — we cache "already unlocked"
  // in a ref so the Refresh button doesn't try to unlock twice.
  const unlockedRef = useRef(false)
  const [unlocked, setUnlocked] = useState(false)
  // Defaults match the regtest stack started by
  // `rgb-lightning-node/regtest.sh start` (compose.yaml in that repo,
  // with rgb-proxy moved off port 3000 to 3001 to avoid the demo's
  // dev-server collision).
  const [bitcoindHost, setBitcoindHost] = useState(process.env.EXPO_PUBLIC_RLN_BITCOIND_HOST ?? HOST_LOOPBACK)
  const [bitcoindPort, setBitcoindPort] = useState(process.env.EXPO_PUBLIC_RLN_BITCOIND_PORT ?? '29443')
  const [bitcoindUser, setBitcoindUser] = useState(process.env.EXPO_PUBLIC_RLN_BITCOIND_USER ?? 'user')
  const [bitcoindPass, setBitcoindPass] = useState(process.env.EXPO_PUBLIC_RLN_BITCOIND_PASSWORD ?? 'password')
  const [indexerUrl, setIndexerUrl] = useState(
    process.env.EXPO_PUBLIC_RLN_INDEXER_URL ?? `tcp://${HOST_LOOPBACK}:56001`
  )
  const [proxyEndpoint, setProxyEndpoint] = useState(
    process.env.EXPO_PUBLIC_RLN_PROXY_ENDPOINT ?? `rpc://${HOST_LOOPBACK}:3400/json-rpc`
  )

  // ─── Node state ───────────────────────────────────────────────────────
  const [nodeInfo, setNodeInfo] = useState<Awaited<ReturnType<LnExt['getNodeInfo']>> | null>(null)
  const [btcBalance, setBtcBalance] = useState<string | null>(null)
  const [peers, setPeers] = useState<Array<{ pubkey?: string; address?: string }>>([])
  const [channels, setChannels] = useState<Awaited<ReturnType<LnExt['listChannels']>>>([])
  const [onchainAddress, setOnchainAddress] = useState<string | null>(null)

  // ─── Form state ───────────────────────────────────────────────────────
  const [peerInput, setPeerInput] = useState('')
  const [channelPeer, setChannelPeer] = useState('')
  const [channelCapacitySat, setChannelCapacitySat] = useState('1000000')
  const [invoiceAmountMsat, setInvoiceAmountMsat] = useState('5000')
  const [lastInvoice, setLastInvoice] = useState<string | null>(null)
  const [payInvoice, setPayInvoice] = useState('')

  // ─── RGB asset state ──────────────────────────────────────────────────
  // Issuance + on-chain rgbsend + inflate + open-asset-channel-as-initiator
  // are all blocked in external-signer mode by RLN's iteration-1 design
  // (see src/sdk/mod.rs `external_signer_mode` checks). The sim can only
  // *receive* assets over LN — the host must originate.
  const [assetInvAssetId, setAssetInvAssetId] = useState('')
  const [assetInvAmount, setAssetInvAmount] = useState('100')
  const [assetInvAmtMsat, setAssetInvAmtMsat] = useState('3000000')
  const [lastAssetInvoice, setLastAssetInvoice] = useState<string | null>(null)
  const [assetBalances, setAssetBalances] = useState<Record<string, { settled?: number; offchain_inbound?: number; offchain_outbound?: number }>>({})

  const [busy, setBusy] = useState(false)
  const [busyLabel, setBusyLabel] = useState<string | null>(null)
  const [errMsg, setErrMsg] = useState<string | null>(null)

  // ─── Helpers ──────────────────────────────────────────────────────────
  // RLN's error enum collapses many causes into a single `Rln(Conflict)`
  // (FailedBitcoindConnection, FailedBroadcast, InvalidIndexer, ...).
  // For unlock specifically, Conflict almost always means bitcoind +
  // indexer + proxy aren't reachable — translate to something useful.
  const explainError = (label: string, raw: string) => {
    if (label.toLowerCase().includes('unlock') && raw.includes('Conflict')) {
      return raw + ' — usually means bitcoind/indexer/proxy unreachable. ' +
        'Check the host:port values and that a regtest stack is running.'
    }
    return raw
  }

  const guard = useCallback(async <T,>(label: string, fn: () => Promise<T>): Promise<T | undefined> => {
    if (!ext) { setErrMsg('Account not ready'); return }
    setBusy(true); setBusyLabel(label); setErrMsg(null)
    try {
      return await fn()
    } catch (e: any) {
      setErrMsg(explainError(label, String(e?.message ?? e)))
      return undefined
    } finally {
      setBusy(false); setBusyLabel(null)
    }
  }, [ext])

  // ─── Actions ──────────────────────────────────────────────────────────
  const onUnlock = useCallback(async () => {
    if (!ext) return
    if (unlockedRef.current) return
    let unlockedHere = false
    await guard('Unlocking node…', async () => {
      await ext.unlock({
        bitcoind_rpc_username: bitcoindUser,
        bitcoind_rpc_password: bitcoindPass,
        bitcoind_rpc_host: bitcoindHost,
        bitcoind_rpc_port: parseInt(bitcoindPort, 10),
        indexer_url: indexerUrl,
        proxy_endpoint: proxyEndpoint,
        announce_addresses: [],
        announce_alias: 'wdk-demo'
      })
      unlockedRef.current = true
      setUnlocked(true)
      unlockedHere = true
    })
    // Inline initial refresh — pulled out of the previous auto-refresh
    // `useEffect` because that effect's `refreshAll` dep was unstable
    // (closures over `ext`, which is a re-derived `Proxy`); the effect
    // re-fired every render and tripped React's update-depth limit.
    if (unlockedHere && ext) {
      try {
        const [info, bal, peerList, chanList, addrRes] = await Promise.all([
          ext.getNodeInfo().catch(() => null),
          ext.getBalanceDetails(true).catch(() => null),
          ext.listPeers().catch(() => ({ peers: [] })),
          ext.listChannels().catch(() => []),
          ext.getAddress().catch(() => null)
        ])
        setNodeInfo(info)
        if (bal?.vanilla?.settled !== undefined) setBtcBalance(String(bal.vanilla.settled))
        setPeers(peerList?.peers ?? [])
        setChannels(Array.isArray(chanList) ? chanList : [])
        const addr = typeof addrRes === 'string' ? addrRes : addrRes?.address ?? null
        if (addr && addr !== PENDING_ADDRESS) setOnchainAddress(addr)
      } catch (_e) { /* silent — manual Refresh can retry */ }
    }
  }, [ext, guard, bitcoindUser, bitcoindPass, bitcoindHost, bitcoindPort, indexerUrl, proxyEndpoint])

  const refreshAll = useCallback(async () => {
    if (!ext) return
    await guard('Refreshing…', async () => {
      // skipSync=false so RLN actually re-scans electrum each refresh.
      // The cached balance (skipSync=true) stays at whatever the last
      // sync found, which is 0 right after unlock.
      const [info, bal, peerList, chanList, addrRes] = await Promise.all([
        ext.getNodeInfo().catch(() => null),
        ext.getBalanceDetails(false).catch(() => null),
        ext.listPeers().catch(() => ({ peers: [] })),
        ext.listChannels().catch(() => []),
        ext.getAddress().catch(() => null)
      ])
      setNodeInfo(info)
      if (bal?.vanilla?.settled !== undefined) {
        setBtcBalance(String(bal.vanilla.settled))
      }
      setPeers(peerList?.peers ?? [])
      setChannels(Array.isArray(chanList) ? chanList : [])
      const addr = typeof addrRes === 'string' ? addrRes : addrRes?.address ?? null
      if (addr && addr !== PENDING_ADDRESS) setOnchainAddress(addr)
    })
  }, [ext, guard])

  const onConnectPeer = useCallback(async () => {
    if (!peerInput.trim()) return
    await guard('Connecting peer…', async () => {
      await ext!.connectPeer(peerInput.trim())
      setPeerInput('')
      const peerList = await ext!.listPeers().catch(() => ({ peers: [] }))
      setPeers(peerList?.peers ?? [])
    })
  }, [ext, guard, peerInput])

  const onOpenChannel = useCallback(async () => {
    if (!channelPeer.trim()) return
    await guard('Opening channel…', async () => {
      // Pre-allocate colorable UTXOs — RGB-LN channels need them
      // even for pure BTC channels. Mirrors Yurii's reference flow
      // (`wallet-flow.ts:wChanACreateUtxos`). Conflict means UTXOs
      // already exist on this dataDir — that's fine, swallow it.
      try {
        await ext!.createUtxos({ up_to: false, num: 10, fee_rate: 7, skip_sync: false })
      } catch (e: any) {
        const msg = String(e?.message ?? e)
        if (!msg.includes('Conflict')) throw e
      }
      const result = await ext!.openChannel({
        peer_pubkey_and_opt_addr: channelPeer.trim(),
        capacity_sat: parseInt(channelCapacitySat, 10),
        push_msat: 0,
        // Matches Yurii's reference flow — private channels for the
        // demo trial; switch to true once gossip / announcement is
        // also exercised end-to-end.
        public: false,
        with_anchors: true,
        fee_base_msat: 1000,
        fee_proportional_millionths: 0,
        asset_id: null,
        asset_amount: null
      })
      Alert.alert('Channel opening', JSON.stringify(result, null, 2))
      const chanList = await ext!.listChannels().catch(() => [])
      setChannels(Array.isArray(chanList) ? chanList : [])
    })
  }, [ext, guard, channelPeer, channelCapacitySat])

  const onCreateInvoice = useCallback(async () => {
    await guard('Creating invoice…', async () => {
      const result = await ext!.createInvoice({
        amt_msat: parseInt(invoiceAmountMsat, 10),
        expiry_sec: 3600
        // Schema accepts only `description_hash` (Option<String>), not a
        // raw description — leave default rather than asking the user
        // to hash it.
      })
      if (result.invoice) {
        setLastInvoice(result.invoice)
        await Clipboard.setStringAsync(result.invoice)
      }
    })
  }, [ext, guard, invoiceAmountMsat])

  const onPayInvoice = useCallback(async () => {
    if (!payInvoice.trim()) return
    await guard('Paying invoice…', async () => {
      const result = await ext!.sendPayment({ invoice: payInvoice.trim() })
      Alert.alert('Payment status', JSON.stringify(result, null, 2))
      setPayInvoice('')
    })
  }, [ext, guard, payInvoice])

  const onCloseChannel = useCallback(async (channelId: string, peerPubkey: string, force: boolean) => {
    await guard(force ? 'Force-closing channel…' : 'Closing channel…', async () => {
      await ext!.closeChannel({ channel_id: channelId, peer_pubkey: peerPubkey, force })
      Alert.alert('Channel close requested', `cid=${channelId.substring(0, 12)}… force=${force}`)
      const chanList = await ext!.listChannels().catch(() => [])
      setChannels(Array.isArray(chanList) ? chanList : [])
    })
  }, [ext, guard])

  // ─── RGB asset actions (sim is receive-only in external-signer mode) ──

  const onCreateAssetLnInvoice = useCallback(async () => {
    if (!assetInvAssetId.trim()) return
    await guard('Creating asset LN invoice…', async () => {
      // LN invoice carrying an RGB asset payment. `amt_msat` must be
      // >= SDK_INVOICE_MIN_MSAT when asset_id is set — 3M msat covers
      // the 3000-sat channel HTLC minimum on regtest.
      const res = await ext!.createInvoice({
        amt_msat: parseInt(assetInvAmtMsat, 10),
        expiry_sec: 3600,
        asset_id: assetInvAssetId.trim(),
        asset_amount: parseInt(assetInvAmount, 10)
      })
      if (res.invoice) {
        setLastAssetInvoice(res.invoice)
        await Clipboard.setStringAsync(res.invoice)
      }
      Alert.alert(
        'Asset LN invoice created',
        res.invoice
          ? `Copied to clipboard:\n${res.invoice.substring(0, 80)}…`
          : JSON.stringify(res, null, 2)
      )
    })
  }, [ext, guard, assetInvAssetId, assetInvAmount, assetInvAmtMsat])

  const onLoadAssetBalance = useCallback(async () => {
    if (!assetInvAssetId.trim()) return
    await guard('Loading asset balance…', async () => {
      // refreshTransfers reconciles on-chain state with rgb-lib's view.
      // Without this, force-close recovered allocations stay in `future`
      // and never roll into `settled` / `spendable`.
      // SDK's refresh only takes `skip_sync`; pass false to force an
      // electrum re-scan so post-force-close on-chain allocations roll
      // from `future` into `settled`.
      await ext!.refreshTransfers({ skip_sync: false }).catch(() => undefined)
      await ext!.refreshTransfers({ skip_sync: false }).catch(() => undefined)
      const bal = await ext!.getAssetBalance(assetInvAssetId.trim())
      setAssetBalances(prev => ({ ...prev, [assetInvAssetId.trim()]: bal }))
      Alert.alert('Asset balance', JSON.stringify(bal, null, 2))
    })
  }, [ext, guard, assetInvAssetId])

  // (Auto-refresh on unlock removed — it busted React's update-depth
  // limit because `refreshAll`'s ref kept churning. The initial
  // populate now happens inline at the end of `onUnlock`. Subsequent
  // refreshes go through the explicit Refresh button.)

  // ─── Render ───────────────────────────────────────────────────────────
  if (isLoading) return <Center><ActivityIndicator color={colors.primary} /></Center>
  if (error) return <Center><Text style={s.err}>{String(error.message ?? error)}</Text></Center>
  if (!ext) return <Center><Text style={s.dim}>Account not ready</Text></Center>

  if (!unlocked) {
    return (
      <ScrollView style={s.root} contentContainerStyle={s.content}>
        <Text style={s.h1}>RGB Lightning</Text>
        <Text style={s.sub}>
          Unlock once with bitcoind / indexer / proxy connection details to bring LDK online.
          The on-chain WDK address is shown in case you need to fund the node first.
        </Text>

        {address && address !== PENDING_ADDRESS
          ? <Row label="On-chain address" value={address} copy />
          : <Text style={s.dim}>On-chain address available after unlock.</Text>}

        <Card title="bitcoind RPC">
          <Field label="Host"     value={bitcoindHost} onChangeText={setBitcoindHost} />
          <Field label="Port"     value={bitcoindPort} onChangeText={setBitcoindPort} keyboardType="numeric" />
          <Field label="Username" value={bitcoindUser} onChangeText={setBitcoindUser} />
          <Field label="Password" value={bitcoindPass} onChangeText={setBitcoindPass} secureTextEntry />
        </Card>

        <Card title="RGB infra">
          <Field label="Indexer URL" value={indexerUrl} onChangeText={setIndexerUrl} />
          <Field label="Proxy endpoint" value={proxyEndpoint} onChangeText={setProxyEndpoint} />
        </Card>

        <Btn label={busy ? (busyLabel ?? '…') : 'Unlock node'} onPress={onUnlock} disabled={busy} />

        {errMsg && <Text style={s.err}>{errMsg}</Text>}
      </ScrollView>
    )
  }

  return (
    <ScrollView style={s.root} contentContainerStyle={s.content}>
      <View style={s.headerRow}>
        <Text style={s.h1}>RGB Lightning</Text>
        <TouchableOpacity onPress={refreshAll} disabled={busy}>
          <Text style={[s.link, busy && { opacity: 0.4 }]}>{busy ? (busyLabel ?? 'Working…') : 'Refresh'}</Text>
        </TouchableOpacity>
      </View>

      {nodeInfo && (
        <Card title="Node">
          {nodeInfo.pubkey && <Row label="Pubkey" value={nodeInfo.pubkey} copy />}
          {nodeInfo.num_peers !== undefined && <Row label="Peers" value={String(nodeInfo.num_peers)} />}
          {nodeInfo.num_channels !== undefined && <Row label="Channels" value={String(nodeInfo.num_channels)} />}
          {nodeInfo.num_usable_channels !== undefined && <Row label="Usable channels" value={String(nodeInfo.num_usable_channels)} />}
          {btcBalance !== null && <Row label="On-chain balance (sat)" value={btcBalance} />}
          {(() => {
            // Prefer the locally-refetched address (which we pull via
            // `ext.getAddress()` after unlock); fall back to the hook's
            // `address` only if it's a real value (not the placeholder).
            const a = onchainAddress ??
              (address && address !== PENDING_ADDRESS ? address : null)
            return a ? <Row label="On-chain address" value={a} copy /> : null
          })()}
        </Card>
      )}

      <Card title="Connect peer">
        <Field
          label="<pubkey>@<host>:<port>"
          value={peerInput}
          onChangeText={setPeerInput}
          placeholder="0289abcd...@127.0.0.1:9735"
        />
        <Btn label="Connect" onPress={onConnectPeer} disabled={busy || !peerInput.trim()} />
        {peers.length > 0 && peers.map((p, i) => (
          <View key={i} style={s.listItem}>
            <Text style={s.mono}>{p.pubkey}</Text>
            <Text style={s.dim}>{p.address}</Text>
          </View>
        ))}
      </Card>

      <Card title="Open channel">
        <Field label="Peer (pubkey@host:port)" value={channelPeer} onChangeText={setChannelPeer} />
        <Field label="Capacity (sat)" value={channelCapacitySat} onChangeText={setChannelCapacitySat} keyboardType="numeric" />
        <Btn label="Open channel" onPress={onOpenChannel} disabled={busy || !channelPeer.trim()} />
        {channels.length > 0 && channels.map((c, i) => (
          <View key={i} style={s.listItem}>
            <Text style={s.mono}>{c.channel_id}</Text>
            <Text style={s.dim}>
              cap {c.capacity_sat} sat · ready={String(c.ready)}
              {c.asset_id ? ` · asset=${String(c.asset_id).substring(0, 16)}…` : ''}
            </Text>
            <View style={{ flexDirection: 'row', gap: 8 }}>
              <Btn label="Close (coop)"
                onPress={() => onCloseChannel(c.channel_id!, c.peer_pubkey!, false)}
                disabled={busy} />
              <Btn label="Force close"
                onPress={() => onCloseChannel(c.channel_id!, c.peer_pubkey!, true)}
                disabled={busy} />
            </View>
          </View>
        ))}
      </Card>

      <Card title="Receive (BOLT11)">
        <Field label="Amount (msat)" value={invoiceAmountMsat} onChangeText={setInvoiceAmountMsat} keyboardType="numeric" />
        <Btn label="Create invoice" onPress={onCreateInvoice} disabled={busy} />
        {lastInvoice && (
          <View style={s.listItem}>
            <Text style={s.dim}>copied to clipboard:</Text>
            <Text style={s.mono} selectable>{lastInvoice}</Text>
          </View>
        )}
      </Card>

      <Card title="Pay (BOLT11)">
        <Field label="Invoice" value={payInvoice} onChangeText={setPayInvoice} multiline />
        <Btn label="Pay" onPress={onPayInvoice} disabled={busy || !payInvoice.trim()} />
      </Card>

      <Card title="Receive RGB asset (over LN)">
        <Text style={s.dim}>
          External-signer mode is receive-only for RGB. The host issues +
          opens the asset channel; the sim creates the LN invoice and
          settles the HTLC.
        </Text>
        <Field label="Asset ID" value={assetInvAssetId} onChangeText={setAssetInvAssetId} />
        <Field label="Asset amount (units)" value={assetInvAmount} onChangeText={setAssetInvAmount} keyboardType="numeric" />
        <Field label="amt_msat (≥ 3000000)" value={assetInvAmtMsat} onChangeText={setAssetInvAmtMsat} keyboardType="numeric" />
        <Btn label="Create asset LN invoice" onPress={onCreateAssetLnInvoice} disabled={busy || !assetInvAssetId.trim()} />
        <Btn label="Check asset balance" onPress={onLoadAssetBalance} disabled={busy || !assetInvAssetId.trim()} />
        {lastAssetInvoice && (
          <View style={s.listItem}>
            <Text style={s.dim}>copied to clipboard:</Text>
            <Text style={s.mono} selectable>{lastAssetInvoice}</Text>
          </View>
        )}
        {Object.entries(assetBalances).map(([id, b]) => (
          <View key={id} style={s.listItem}>
            <Text style={s.dim}>{id.substring(0, 24)}…</Text>
            <Text style={s.mono}>
              settled={b.settled ?? 0} inbound={b.offchain_inbound ?? 0} outbound={b.offchain_outbound ?? 0}
            </Text>
          </View>
        ))}
      </Card>

      {errMsg && <Text style={s.err}>{errMsg}</Text>}
    </ScrollView>
  )
}

// ─── Tiny UI components ─────────────────────────────────────────────────

function Center ({ children }: { children: React.ReactNode }) {
  return <View style={[s.root, { justifyContent: 'center', alignItems: 'center' }]}>{children}</View>
}

function Card ({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <View style={s.card}>
      <Text style={s.cardTitle}>{title}</Text>
      {children}
    </View>
  )
}

function Field (props: {
  label: string
  value: string
  onChangeText: (v: string) => void
  placeholder?: string
  keyboardType?: 'default' | 'numeric'
  secureTextEntry?: boolean
  multiline?: boolean
}) {
  return (
    <View style={{ gap: 4 }}>
      <Text style={s.fieldLabel}>{props.label}</Text>
      <TextInput
        style={[s.input, props.multiline && { minHeight: 70, textAlignVertical: 'top' }]}
        value={props.value}
        onChangeText={props.onChangeText}
        placeholder={props.placeholder}
        placeholderTextColor={colors.textTertiary}
        keyboardType={props.keyboardType ?? 'default'}
        secureTextEntry={props.secureTextEntry}
        multiline={props.multiline}
        autoCapitalize="none"
        autoCorrect={false}
      />
    </View>
  )
}

function Row ({ label, value, copy }: { label: string; value: string; copy?: boolean }) {
  return (
    <TouchableOpacity
      style={s.row}
      onPress={copy ? () => Clipboard.setStringAsync(value) : undefined}
      disabled={!copy}
    >
      <Text style={s.dim}>{label}</Text>
      <Text style={[s.mono, { flex: 1, marginLeft: 8 }]} numberOfLines={1}>{value}</Text>
    </TouchableOpacity>
  )
}

function Btn ({ label, onPress, disabled }: { label: string; onPress: () => void; disabled?: boolean }) {
  return (
    <TouchableOpacity style={[s.btn, disabled && s.btnDisabled]} onPress={onPress} disabled={disabled}>
      <Text style={s.btnText}>{label}</Text>
    </TouchableOpacity>
  )
}

const s = StyleSheet.create({
  root:    { flex: 1, backgroundColor: colors.background },
  content: { padding: 16, gap: 12 },
  headerRow: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between' },
  h1:      { fontSize: 20, fontWeight: '600', color: colors.text },
  sub:     { color: colors.textSecondary, fontSize: 13 },
  link:    { color: colors.primary },
  err:     { color: colors.error, fontFamily: 'Menlo', fontSize: 12 },
  dim:     { color: colors.textSecondary, fontSize: 12 },
  mono:    { color: colors.text, fontFamily: 'Menlo', fontSize: 12 },
  fieldLabel: { color: colors.textSecondary, fontSize: 12 },
  card:    { backgroundColor: colors.card, borderRadius: 8, padding: 12, gap: 8 },
  cardTitle: { color: colors.text, fontSize: 14, fontWeight: '600', marginBottom: 4 },
  input:   {
    borderWidth: 1, borderColor: colors.border, backgroundColor: '#0F0F0F',
    color: colors.text, borderRadius: 6, padding: 10, fontFamily: 'Menlo', fontSize: 12
  },
  row:     { flexDirection: 'row', alignItems: 'center', paddingVertical: 4 },
  listItem: { paddingVertical: 6, gap: 2, borderTopWidth: 1, borderTopColor: colors.border },
  btn:     { backgroundColor: colors.primary, padding: 12, borderRadius: 6, alignItems: 'center' },
  btnDisabled: { opacity: 0.4 },
  btnText: { color: '#000', fontWeight: '600' }
})
