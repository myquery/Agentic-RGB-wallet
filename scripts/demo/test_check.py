import unittest
import check


class PreflightTests(unittest.TestCase):
    def test_full_preflight_and_snapshot_omit_secrets(self):
        urls = []
        def read(url):
            urls.append(url)
            role = 'alice' if ':3030/' in url or ':3101/' in url or 'alice' in url else 'bob'
            if '/api/wallet' in url:
                return dict(wallet_name=role.title()+' Wallet', network='regtest',
                            recipient_address=role+'@example.com', commerce_enabled=True,
                            btc_policy=dict(human_approval_required=True, max_payment_sats='100'),
                            holdings=[dict(asset=dict(asset_id='rgb:x', ticker='R402USD', precision=0), outbound='50')])
            if '/api/activity' in url:
                return [dict(payment_hash='hash', preimage='SECRET')]
            if '/networkinfo' in url:
                return dict(network='Regtest', height=100)
            if '/nodeinfo' in url:
                return dict(pubkey=role, private_key='SECRET')
            if '/listchannels' in url:
                return dict(channels=[dict(channel_id='shared', asset_id='rgb:x', ready=True,
                    peer_pubkey='bob' if role=='alice' else 'alice', is_usable=True,
                    next_outbound_htlc_minimum_msat=1, next_outbound_htlc_limit_msat=10000000,
                    inbound_balance_msat=10000000, outbound_balance_msat=10000000)])
            if '/healthz' in url:
                return 'ok'
            return dict(subject='acct:'+role+'@example.com', links=[dict(
                rel='https://rgb402.example/relations/'+rail+'-invoice',
                href='https://example.com/'+rail+'/invoice/'+role) for rail in ('rgb','btc')])
        result = check.collect('example.com', read)
        self.assertTrue(result['passed'], result['checks'])
        self.assertNotIn('SECRET', str(result))
        self.assertFalse(any('/premium/' in u or '/sendpayment' in u or '/api/session' in u for u in urls))

    def test_sanitizer_drops_nested_secrets(self):
        summary = check.wallet_summary({'holdings': [{'asset': {'asset_id': 'rgb:x', 'seed': 'SECRET'},
                                                    'outbound': '5', 'preimage': 'SECRET'}],
                                        'token': 'SECRET', 'policy': {'token': 'SECRET'}})
        self.assertNotIn('SECRET', str(summary))
        self.assertEqual(summary['holdings'][0]['outbound'], '5')

    def test_minimum_and_unusable_channels_fail_closed(self):
        channel = dict(ready=True, is_usable=True, next_outbound_htlc_minimum_msat=3000000,
                       next_outbound_htlc_limit_msat=10000000)
        self.assertFalse(check.supports(channel, 5000))
        self.assertTrue(check.supports(channel, 3000000))
        self.assertFalse(check.supports(dict(channel, is_usable=False), 3000000))
        self.assertFalse(check.supports(dict(channel, next_outbound_htlc_minimum_msat=None), 3000000))

    def test_only_read_only_endpoints_even_when_services_fail(self):
        urls = []
        def unavailable(url):
            urls.append(url)
            raise ValueError('SECRET')
        result = check.collect('example.com', unavailable)
        self.assertFalse(result['passed'])
        self.assertNotIn('SECRET', str(result))
        self.assertTrue(all('/api/wallet' in u or '/healthz' in u or '/.well-known/webfinger?' in u for u in urls))


if __name__ == '__main__':
    unittest.main()
