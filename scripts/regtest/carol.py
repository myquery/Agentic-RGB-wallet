#!/usr/bin/env python3
"""Add Carol to the existing regtest project without resetting Alice/Bob. No invoice payments."""
import argparse
import json
import os
import regtest as r


def intent(path, value):
    with path.open('x') as stream:
        json.dump(value, stream)
        stream.flush()
        os.fsync(stream.fileno())


def main(apply=False):
    r.safe_paths()
    asset = r.load('bootstrap.json')['asset_id']
    print(json.dumps({'new_wallet': 'carol', 'node_port': 3103, 'wallet_port': 3032,
                      'asset_id': asset, 'carol_onchain_funding_btc': 1,
                      'alice_to_carol_channel_sats': 100000, 'channel_rgb_units': 100,
                      'push_rgb_units': 0, 'invoice_payments': False}))
    if not apply:
        return
    data = r.STATE / 'data/carol'
    data.mkdir(exist_ok=True)
    override = r.STATE / 'carol-compose.json'
    override.write_text(json.dumps({'services': {'carol': {
        'image': r.RUNTIME, 'user': f'{os.getuid()}:{os.getgid()}',
        'command': ['/data', '--daemon-listening-port', '3001', '--ldk-peer-listening-port', '9735', '--network', 'regtest', '--disable-authentication'],
        'ports': ['127.0.0.1:3103:3001', '127.0.0.1:19737:9735'],
        'volumes': [str(r.UPSTREAM / 'target/debug/rgb-lightning-node') + ':/usr/local/bin/rgb-lightning-node:ro', str(data) + ':/data'],
        'stop_grace_period': '60s'}}}, indent=2))
    r.run(['docker', 'compose', '-p', r.PROJECT, '-f', r.STATE / 'compose.yaml', '-f', override, 'up', '-d', '--no-deps', 'carol'])
    r.PORTS['carol'] = 3103
    r.wait_for('Carol API', lambda: r.api_ready('carol'))
    r.unlock('carol')
    balance = r.api('carol', 'btcbalance', {'skip_sync': False})['vanilla']['spendable']
    funding = r.STATE / 'carol-funding-attempt.json'
    if balance == 0:
        if funding.exists():
            raise r.Failure('Carol funding already attempted; inspect rather than repeat')
        intent(funding, {'requested': True})
        address = r.api('carol', 'address', {})['address']
        r.btc('sendtoaddress', address, '1')
        r.mine(6)
        r.synced()
    try:
        r.api('carol', 'createutxos', {'up_to': True, 'num': 10, 'size': 32500, 'fee_rate': 2, 'skip_sync': False})
    except r.ApiError as e:
        if e.name != 'AllocationsAlreadyAvailable':
            raise
    r.mine(6)
    r.synced()
    pubkey = r.api('carol', 'nodeinfo')['pubkey']
    r.api('alice', 'connectpeer', {'peer_pubkey_and_addr': pubkey + '@carol:9735'})
    matches = [c for c in r.channels('alice') if c.get('peer_pubkey') == pubkey and c.get('asset_id') == asset]
    if len(matches) > 1:
        raise r.Failure('Ambiguous Carol channels')
    attempt = r.STATE / 'carol-channel-attempt.json'
    if not matches:
        if attempt.exists():
            raise r.Failure('Carol channel already attempted; inspect rather than reopen')
        intent(attempt, {'requested': True, 'asset_id': asset, 'amount': 100})
        r.api('alice', 'openchannel', {'peer_pubkey_and_opt_addr': pubkey + '@carol:9735',
              'capacity_sat': 100000, 'push_msat': 10000000, 'asset_id': asset,
              'asset_amount': 100, 'push_asset_amount': 0, 'public': True, 'with_anchors': True})
    channel = r.wait_for('Carol funding transaction', lambda: next((c for c in r.channels('alice') if c.get('peer_pubkey') == pubkey and c.get('asset_id') == asset and c.get('funding_txid')), None))
    if not channel['ready']:
        r.wait_for('Carol funding broadcast', lambda: r.btc('gettxout', channel['funding_txid'], 0))
        r.mine(6)
        r.synced()
    for node in ('alice', 'carol'):
        r.wait_for(node + ' Carol channel ready', lambda node=node: r.ready_channel(node, channel['channel_id']))
    print('Carol ready; no invoice payment submitted.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--apply', action='store_true')
    try:
        main(parser.parse_args().apply)
    except (r.Failure, OSError, ValueError) as error:
        raise SystemExit(str(error)) from None
