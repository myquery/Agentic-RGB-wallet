"""Offline safety regressions for the development runner; no Docker or node required."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import regtest


class RunnerSafetyTests(unittest.TestCase):
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
