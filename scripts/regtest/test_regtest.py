"""Offline safety regressions for the development runner; no Docker or node required."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import regtest


class RunnerSafetyTests(unittest.TestCase):
    def test_resume_requires_existing_demo_before_starting_containers(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(regtest, 'STATE', Path(directory)), patch.object(regtest, 'start') as start:
                with self.assertRaisesRegex(regtest.Failure, 'incomplete'):
                    regtest.resume()
                start.assert_not_called()

    def test_resume_unlocks_existing_nodes_without_provisioning(self):
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory)
            for name in ('compose.yaml', 'bootstrap.json', 'wallet.env'):
                (state / name).touch()
            (state / 'chain-anchor.json').write_text(json.dumps({'height': 103, 'hash': 'old-chain'}))
            for name in ('alice', 'bob'):
                (state / 'data' / name).mkdir(parents=True)
            with patch.object(regtest, 'STATE', state), patch.object(regtest, 'start') as start, \
                 patch.object(regtest, 'unlock') as unlock, patch.object(regtest, 'wait_for') as wait:
                regtest.resume()
                start.assert_called_once_with(wake_ibd=True, expected_chain={'height': 103, 'hash': 'old-chain'}, reuse_build=True)
                self.assertEqual([call.args[0] for call in unlock.call_args_list], ['alice', 'bob'])
                self.assertEqual(wait.call_count, 2)

    def test_chain_mismatch_stops_before_mining_or_nodes(self):
        def bitcoin(command, *args):
            if command == 'getblockcount':
                return 103
            if command == 'getblockhash':
                return 'new-chain'
            return {}
        with patch.object(regtest, 'docker_ready'), patch.object(regtest, 'checkout_and_build'), \
             patch.object(regtest, 'write_compose'), patch.object(regtest, 'compose') as compose, \
             patch.object(regtest, 'wait_for'), patch.object(regtest, 'btc', side_effect=bitcoin), \
             patch.object(regtest, 'mine') as mine:
            with self.assertRaisesRegex(regtest.Failure, 'does not match'):
                regtest.start(expected_chain={'height': 103, 'hash': 'old-chain'})
            mine.assert_not_called()
            self.assertEqual(compose.call_args_list[-1].args, ('up', '-d', 'bitcoind'))

    def test_reset_without_flag_never_calls_docker(self):
        with patch.object(regtest, 'docker_ready') as docker:
            with self.assertRaises(regtest.Failure):
                regtest.reset(False)
            docker.assert_not_called()

    def test_symlinked_runtime_parent_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            external = root / 'other'
            external.mkdir()
            (root / '.var').symlink_to(external, target_is_directory=True)
            with patch.object(regtest, 'ROOT', root), patch.object(regtest, 'STATE', root / '.var/regtest'):
                with self.assertRaises(regtest.Failure):
                    regtest.safe_paths()

    def test_interactive_output_selects_receipt_not_plan(self):
        plan = {'request': {'payment_hash': 'hash'}, 'policy': {'decision': 'require_approval'}}
        result = {'payment_id': 'node-id', 'payment_hash': 'hash', 'status': 'Pending'}
        text = json.dumps(plan) + '\nApprove? [y/N] ' + json.dumps(result)
        self.assertEqual(regtest.payment_result(text), result)
        self.assertIsNone(regtest.payment_result(json.dumps(plan) + '\nPayment cancelled.'))

    def test_failed_payment_stops_verification_without_refund_or_resubmit(self):
        attempt = {'request': {'payment_hash': 'hash', 'asset_id': 'asset'}}
        with patch.object(regtest, 'load', return_value=attempt), patch.object(regtest, 'wallet', return_value='Failed') as wallet:
            with self.assertRaisesRegex(regtest.Failure, 'reservation retained'):
                regtest.verify()
            wallet.assert_called_once_with('status', 'hash', 'asset')


if __name__ == '__main__':
    unittest.main()
