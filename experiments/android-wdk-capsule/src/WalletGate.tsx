// WalletGate — handles NO_WALLET / LOCKED / READY transitions.
//
// createWallet generates and stores seed material inside the WDK wallet
// manager. This diagnostic UI never copies that material into React state or
// logs. On subsequent runs, the provider can unlock the stored wallet.
import React, { useState } from 'react'
import { ActivityIndicator, StyleSheet, Text, TextInput, TouchableOpacity, View } from 'react-native'
import { useWalletManager } from '@tetherto/wdk-react-native-core'
import type { WdkAppState } from '@tetherto/wdk-react-native-core'
import { RgbLightningScreen } from './RgbLightningScreen'

const WALLET_ID = 'luma-capsule-spike-wallet'

export function WalletGate ({ state }: { state: WdkAppState }) {
  const { createWallet, restoreWallet, unlock } = useWalletManager()
  const [busy, setBusy] = useState(false)
  const [err, setErr] = useState<string | null>(null)
  const [restoreMnemonic, setRestoreMnemonic] = useState('')

  async function onCreate () {
    setErr(null); setBusy(true)
    try {
      await createWallet(WALLET_ID)
    } catch (e: any) {
      const msg = String(e?.message ?? e)
      // iOS Keychain items can survive app uninstalls. If the wallet is
      // already there, just unlock it instead of failing hard.
      if (msg.includes('already exists')) {
        try {
          await unlock(WALLET_ID)
        } catch (e2: any) { setErr(String(e2?.message ?? e2)) }
      } else {
        setErr(msg)
      }
    } finally { setBusy(false) }
  }

  async function onRestore () {
    setErr(null); setBusy(true)
    try {
      await restoreWallet(restoreMnemonic.trim(), WALLET_ID)
    } catch (e: any) { setErr(String(e?.message ?? e)) }
    finally { setBusy(false) }
  }

  async function onUnlock () {
    setErr(null); setBusy(true)
    try { await unlock(WALLET_ID) }
    catch (e: any) { setErr(String(e?.message ?? e)) }
    finally { setBusy(false) }
  }

  if (state.status === 'READY') {
    // Demo focuses on RGB Lightning. The on-chain RGB tab and its
    // connect-gate were dropped because `useAccount({network:'rgb'})`
    // constructs `WalletAccountRgb` which hard-connects to the indexer
    // on construction — without a reachable electrum server (regtest
    // stack), it errors and useAddressLoader retries forever.
    return <RgbLightningScreen />
  }

  if (state.status === 'LOCKED') {
    return (
      <View style={s.center}>
        <Text style={s.h1}>Wallet locked</Text>
        <Text style={s.sub}>{state.walletId}</Text>
        <Btn label={busy ? 'Unlocking…' : 'Unlock'} onPress={onUnlock} disabled={busy} />
        {err && <Text style={s.err}>{err}</Text>}
      </View>
    )
  }

  // NO_WALLET
  return (
    <View style={s.center}>
      <Text style={s.h1}>Create RGB wallet</Text>
      <Text style={s.sub}>A fresh BIP-39 seed is generated on-device and stored in the platform keychain.</Text>
      <Btn label={busy ? 'Creating…' : 'Create new wallet'} onPress={onCreate} disabled={busy} />

      <Text style={[s.h1, { marginTop: 24 }]}>Or restore from mnemonic</Text>
      <TextInput
        style={s.input}
        multiline
        placeholder="12 or 24 word seed phrase"
        value={restoreMnemonic}
        onChangeText={setRestoreMnemonic}
        autoCapitalize="none"
        autoCorrect={false}
      />
      <Btn label={busy ? 'Restoring…' : 'Restore'} onPress={onRestore} disabled={busy || !restoreMnemonic.trim()} />

      {busy && <ActivityIndicator />}
      {err && <Text style={s.err}>{err}</Text>}
    </View>
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
  center: { padding: 16, gap: 10, backgroundColor: '#121212', flex: 1 },
  h1:     { fontSize: 20, fontWeight: '600', color: '#fff' },
  sub:    { color: '#999' },
  btn:    { backgroundColor: '#FF6501', padding: 12, borderRadius: 6, alignItems: 'center' },
  btnDisabled: { opacity: 0.4 },
  btnText: { color: '#000', fontWeight: '600' },
  input:  { borderWidth: 1, borderColor: '#333', backgroundColor: '#1E1E1E', color: '#fff', borderRadius: 6, padding: 10, minHeight: 80, textAlignVertical: 'top', fontFamily: 'Menlo' },
  err:    { color: '#FF6B6B' }
})
