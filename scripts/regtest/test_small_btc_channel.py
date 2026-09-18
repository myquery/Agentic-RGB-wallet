import unittest
from unittest.mock import patch
import small_btc_channel as channel


class SmallBtcChannelTests(unittest.TestCase):
    def test_only_matching_btc_channel_and_supported_minimum(self):
        base = {'asset_id': None, 'peer_pubkey': 'alice', 'capacity_sat': 100000,
                'is_usable': True, 'next_outbound_htlc_minimum_msat': 1,
                'next_outbound_htlc_limit_msat': 10000}
        self.assertEqual(channel.candidates([base, dict(base, asset_id='rgb:test'),
                                            dict(base, peer_pubkey='other')], 'alice'), [base])
        self.assertTrue(channel.supports_small(base))
        for field, value in [('is_usable', False), ('next_outbound_htlc_minimum_msat', 3000000),
                             ('next_outbound_htlc_limit_msat', 4999)]:
            self.assertFalse(channel.supports_small(dict(base, **{field: value})))
        self.assertFalse(channel.supports_small({'is_usable': True}))

    def test_existing_uncertain_attempt_never_opens_again(self):
        with patch.object(channel.regtest, 'api', return_value={'pubkey': 'alice'}) as api, \
                patch.object(channel.regtest, 'channels', return_value=[]), \
                patch.object(channel, 'ATTEMPT') as attempt:
            attempt.exists.return_value = True
            with self.assertRaises(channel.regtest.Failure):
                channel.main(apply=True)
            self.assertEqual([c.args[1] for c in api.call_args_list], ['nodeinfo', 'nodeinfo'])


if __name__ == '__main__':
    unittest.main()
