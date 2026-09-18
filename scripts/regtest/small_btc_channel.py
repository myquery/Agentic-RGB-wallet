#!/usr/bin/env python3
"""Review or explicitly open one Bob-funded BTC-only regtest channel. Never sends payments."""
import argparse
import json
import os
from pathlib import Path
import regtest

CAPACITY_SATS = 100_000
ATTEMPT = regtest.STATE / 'small-btc-channel.json'


def candidates(channels, alice_pubkey):
    return [c for c in channels if c.get('asset_id') is None
            and c.get('peer_pubkey') == alice_pubkey
            and c.get('capacity_sat') == CAPACITY_SATS]


def supports_small(channel):
    return (channel.get('is_usable') is True
            and 0 <= channel.get('next_outbound_htlc_minimum_msat', 2**64) <= 5_000
            and channel.get('next_outbound_htlc_limit_msat', 0) >= 5_000)


def main(apply=False):
    pubkeys = {n: regtest.api(n, 'nodeinfo')['pubkey'] for n in ('alice', 'bob')}
    existing = candidates(regtest.channels('bob'), pubkeys['alice'])
    if len(existing) > 1:
        raise regtest.Failure('Multiple matching BTC channels; inspect rather than guess')
    if not apply:
        balance = regtest.api('bob', 'btcbalance', {'skip_sync': False})['vanilla']['spendable']
        print(json.dumps({'mode': 'review_only', 'funder': 'bob', 'peer': 'alice',
                          'capacity_sats': CAPACITY_SATS, 'push_msat': 0,
                          'asset_id': None, 'bob_onchain_spendable_sats': balance,
                          'existing_matching_channels': len(existing),
                          'previous_open_attempt': ATTEMPT.exists(),
                          'fees': 'node-managed regtest funding fee',
                          'keeps_existing_rgb_channel': True,
                          'payment_submitted': False}, indent=2))
        return
    # Exclusive, durable intent before the potentially ambiguous economic request.
    # A rerun may finish an observed channel, but never repeats /openchannel.
    if not existing:
        if ATTEMPT.exists():
            raise regtest.Failure('Previous open attempt exists; inspect node state, do not reopen')
        balance = regtest.api('bob', 'btcbalance', {'skip_sync': False})['vanilla']['spendable']
        if balance <= CAPACITY_SATS:
            raise regtest.Failure('Insufficient Bob funding balance including fee headroom')
        intent = {'funder': 'bob', 'peer_pubkey': pubkeys['alice'], 'capacity_sats': CAPACITY_SATS,
                  'push_msat': 0, 'requested': True}
        fd = os.open(ATTEMPT, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, 'w') as stream:
            json.dump(intent, stream); stream.write('\n'); stream.flush(); os.fsync(stream.fileno())
        directory = os.open(ATTEMPT.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
        regtest.api('bob', 'openchannel', {'peer_pubkey_and_opt_addr': pubkeys['alice'] + '@alice:9735',
                                         'capacity_sat': CAPACITY_SATS, 'push_msat': 0,
                                         'public': True, 'with_anchors': True})

    def funded():
        channels = candidates(regtest.channels('bob'), pubkeys['alice'])
        if len(channels) != 1:
            return None
        channel = channels[0]
        txid = channel.get('funding_txid')
        if not txid:
            return None
        return channel if channel.get('ready') or regtest.btc('gettxout', txid, 0) else None

    channel = regtest.wait_for('BTC channel funding transaction', funded)
    if not channel['ready']:
        regtest.mine(6)
        regtest.synced()
    channel_id = channel['channel_id']
    ready = regtest.wait_for('Bob BTC channel ready', lambda: regtest.ready_channel('bob', channel_id))
    regtest.wait_for('Alice BTC channel ready', lambda: regtest.ready_channel('alice', channel_id))
    if not supports_small(ready):
        raise regtest.Failure('Channel ready but does not permit 5 sats; no payment attempted')
    print(json.dumps({'channel_id': channel_id, 'capacity_sats': ready['capacity_sat'],
                      'minimum_msat': ready['next_outbound_htlc_minimum_msat'],
                      'maximum_msat': ready['next_outbound_htlc_limit_msat'],
                      'supports_bob_to_alice_5_sats': True, 'payment_submitted': False}, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true', help='Open the reviewed regtest channel and mine confirmations')
    try:
        main(parser.parse_args().apply)
    except (regtest.Failure, OSError, ValueError) as error:
        raise SystemExit(str(error)) from None
