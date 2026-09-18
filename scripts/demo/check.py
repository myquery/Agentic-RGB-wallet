#!/usr/bin/env python3
"""Read-only local demo preflight and allowlisted evidence snapshot. No invoices."""
import argparse
import datetime
import json
import os
from pathlib import Path
import urllib.parse
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
CHANNEL = ('channel_id', 'peer_pubkey', 'asset_id', 'ready', 'is_usable',
           'capacity_sat', 'outbound_balance_msat', 'inbound_balance_msat',
           'next_outbound_htlc_minimum_msat', 'next_outbound_htlc_limit_msat')
ACTIVITY = ('kind', 'payment_hash', 'asset_id', 'amount', 'timestamp', 'direction', 'status')


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def fetch(url):
    # Never fetch a premium resource, session, invoice, proof or journal.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    with opener.open(url, timeout=12) as response:
        data = response.read(2_000_001)
        if len(data) > 2_000_000:
            raise ValueError('response too large')
        return data.decode().strip() if url.endswith('/healthz') else json.loads(data)


def pick(value, keys):
    return {k: value[k] for k in keys if k in value}


def wallet_summary(w):
    result = pick(w, ('wallet_name', 'network', 'recipient_address',
                      'btc_outbound_sats', 'commerce_enabled'))
    result['holdings'] = [dict(asset=pick(h['asset'], ('asset_id', 'name', 'ticker', 'precision')),
                               **pick(h, ('outbound', 'onchain'))) for h in w['holdings']]
    for key, fields in [('btc_policy', ('max_payment_sats', 'max_daily_sats', 'human_approval_required')),
                        ('policy', ('auto_approve_below', 'max_single_payment', 'max_daily_spend', 'max_carrier_msat'))]:
        result[key] = pick(w.get(key) or {}, fields)
    return result


def supports(c, msat):
    return (c.get('is_usable') is True and c.get('ready') is True
            and c.get('next_outbound_htlc_minimum_msat') is not None
            and c['next_outbound_htlc_minimum_msat'] <= msat
            <= c.get('next_outbound_htlc_limit_msat', 0))


def collect(domain, read=fetch):
    result = {'timestamp': datetime.datetime.now(datetime.timezone.utc).isoformat(),
              'schema': 1, 'economic_actions': False, 'wallets': {}, 'checks': []}
    def check(name, ok):
        result['checks'].append({'check': name, 'passed': bool(ok)})
    for role, port, node in [('alice', 3030, 3101), ('bob', 3031, 3102)]:
        try:
            wallet = wallet_summary(read(f'http://127.0.0.1:{port}/api/wallet'))
            activity = [pick(a, ACTIVITY) for a in read(f'http://127.0.0.1:{port}/api/activity')]
            network = pick(read(f'http://127.0.0.1:{node}/networkinfo'), ('network', 'height'))
            identity = pick(read(f'http://127.0.0.1:{node}/nodeinfo'), ('pubkey',))
            channels = [pick(c, CHANNEL) for c in read(f'http://127.0.0.1:{node}/listchannels')['channels']]
            result['wallets'][role] = dict(summary=wallet, activity=activity, node=identity,
                                           network=network, channels=channels)
            check(role + ' wallet identity', wallet.get('wallet_name') == role.title() + ' Wallet')
            check(role + ' regtest', str(network.get('network')).lower() == 'regtest'
                  and wallet.get('network') == 'regtest')
            check(role + ' recipient identity', wallet.get('recipient_address') == role + '@' + domain)
            check(role + ' BTC human approval', wallet['btc_policy'].get('human_approval_required') is True)
        except Exception as error:
            # Error bodies/URLs can carry secrets; record only the exception class.
            check(role + ' services (' + type(error).__name__ + ')', False)
    if len(result['wallets']) == 2:
        a, b = (result['wallets'][r] for r in ('alice', 'bob'))
        ah = [h for h in a['summary']['holdings'] if h['asset'].get('ticker') == 'R402USD' and h['asset'].get('precision') == 0]
        bh = {h['asset']['asset_id'] for h in b['summary']['holdings']}
        check('one shared R402USD asset', len(ah) == 1 and ah[0]['asset']['asset_id'] in bh)
        asset = ah[0]['asset']['asset_id'] if len(ah) == 1 else None
        check('Alice RGB outbound >= 5 units', len(ah) == 1 and int(ah[0]['outbound']) >= 5)
        ac = [c for c in a['channels'] if c.get('peer_pubkey') == b['node'].get('pubkey')]
        bc = [c for c in b['channels'] if c.get('peer_pubkey') == a['node'].get('pubkey')]
        check('distinct node identities', bool(a['node'].get('pubkey')) and bool(b['node'].get('pubkey'))
              and a['node']['pubkey'] != b['node']['pubkey'])
        check('shared usable channel', bool({c.get('channel_id') for c in ac if c.get('is_usable')}
              & {c.get('channel_id') for c in bc if c.get('is_usable')}))
        check('RGB channel supports 3000-sat carrier', asset is not None and any(
            c.get('asset_id') == asset and supports(c, 3_000_000) for c in ac))
        check('Alice BTC routes support 10 sats and 3 sats', all(any(supports(c, ms) for c in ac) for ms in (10_000, 3_000)))
        check('Bob small-payment return channel', any(supports(c, 5_000) for c in bc))
        check('Bob inbound >= 3013 sats', sum(c.get('inbound_balance_msat', 0) for c in bc if c.get('is_usable')) >= 3_013_000)
        check('Alice outbound >= 3013 sats', sum(c.get('outbound_balance_msat', 0) for c in ac if c.get('is_usable')) >= 3_013_000)
        check('Alice commerce configured', a['summary'].get('commerce_enabled') is True)
        check('Alice BTC policy permits 10 sats', int(a['summary']['btc_policy'].get('max_payment_sats', 0)) >= 10)
    try:
        check('merchant health', read('http://127.0.0.1:3040/healthz') == 'ok')
    except Exception as error:
        check('merchant health (' + type(error).__name__ + ')', False)
    for role in ('alice', 'bob'):
        try:
            subject = 'acct:' + role + '@' + domain
            doc = read('https://' + domain + '/.well-known/webfinger?' + urllib.parse.urlencode({'resource': subject}))
            links = doc.get('links', [])
            check(role + ' WebFinger subject', doc.get('subject') == subject)
            for rail in ('rgb', 'btc'):
                check(role + ' ' + rail + ' discovery', any(
                    l.get('rel') == 'https://rgb402.example/relations/' + rail + '-invoice'
                    and l.get('href') == 'https://' + domain + '/' + rail + '/invoice/' + role for l in links))
        except Exception as error:
            check(role + ' discovery (' + type(error).__name__ + ')', False)
    result['passed'] = all(c['passed'] for c in result['checks'])
    result['limitations'] = ['No invoice, plan, approval or payment was created.',
                            'Reachability and liquidity do not prove settlement, provider access or remaining daily budget.',
                            'Existing L402 credentials may have expired; never erase reservations to repeat the demo.']
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--domain', default=os.environ.get('RECIPIENT_DOMAIN'))
    parser.add_argument('--snapshot', type=Path, help='New sanitized JSON file; refuses overwrite')
    args = parser.parse_args()
    domain = args.domain
    if not domain:
        domain = (ROOT / '.var/regtest/recipient-domain').read_text().strip()
    if not domain or any(c not in 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789.-' for c in domain):
        parser.error('Provide only the configured public hostname')
    result = collect(domain)
    if args.snapshot:
        args.snapshot.parent.mkdir(parents=True, exist_ok=True)
        with args.snapshot.open('x') as stream:
            json.dump(result, stream, indent=2)
            stream.write('\n')
    for c in result['checks']:
        print(('PASS ' if c['passed'] else 'FAIL ') + c['check'])
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
