import json
from pathlib import Path
import tempfile
import unittest

from tools.capsule.capsule import CapsuleError, Registry, restore, snapshot, validate


class CapsuleTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        root = Path(self.temp.name)
        self.repo = root / "capsules"
        self.source = root / "source"
        (self.source / "node/ldk/monitors").mkdir(parents=True)
        (self.source / "node/ldk/manager").write_text("manager-v1")
        (self.source / "node/ldk/monitors/channel").write_text("monitor-v1")
        (self.source / "rgb.jsonl").write_text('{"payment":"settled"}\n')
        (self.source / "btc.jsonl").write_text('{"payment":"reserved"}\n')
        self.spec = root / "spec.json"
        self.spec.write_text(json.dumps({
            "wallet_id": "disposable-wallet", "wallet_fingerprint": "fingerprint-hash",
            "node_id": "node-id-hash", "network": "regtest",
            "implementation": {"project_revision": "test", "rgb_lightning_revision": "test"},
            "components": [
                {"name":"node_state", "class":"SAFETY-CRITICAL", "source":str(self.source / "node"), "destination":"node"},
                {"name":"rgb_journal", "class":"APPLICATION-JOURNAL", "source":str(self.source / "rgb.jsonl"), "destination":"journals/rgb.jsonl", "journal":True},
                {"name":"btc_journal", "class":"APPLICATION-JOURNAL", "source":str(self.source / "btc.jsonl"), "destination":"journals/btc.jsonl", "journal":True},
            ]}))
        self.registry = Registry(self.repo)
        self.epoch = self.registry.acquire("disposable-wallet", "writer-a")

    def tearDown(self): self.temp.cleanup()

    def test_round_trip_and_journal_boundary(self):
        generation = snapshot(self.repo, self.spec, "writer-a", self.epoch)
        manifest = validate(self.repo, generation)
        self.assertEqual(manifest["generation"], 1)
        self.assertEqual(manifest["components"]["rgb_journal"]["journal_sequence"], 1)
        target = Path(self.temp.name) / "restored"
        restore(self.repo, generation, target, "writer-a", self.epoch)
        self.assertEqual((target / "node/ldk/manager").read_text(), "manager-v1")

    def test_stale_generation_rejected(self):
        old = snapshot(self.repo, self.spec, "writer-a", self.epoch)
        (self.source / "rgb.jsonl").write_text('{"payment":"settled"}\n{"payment":"new"}\n')
        snapshot(self.repo, self.spec, "writer-a", self.epoch)
        with self.assertRaisesRegex(CapsuleError, "STALE"):
            validate(self.repo, old)

    def test_second_writer_and_old_epoch_rejected(self):
        with self.assertRaisesRegex(CapsuleError, "SPLIT_BRAIN"):
            self.registry.acquire("disposable-wallet", "writer-b")
        new_epoch = self.registry.acquire("disposable-wallet", "writer-b", takeover=True)
        self.assertEqual(new_epoch, self.epoch + 1)
        with self.assertRaisesRegex(CapsuleError, "SPLIT_BRAIN"):
            snapshot(self.repo, self.spec, "writer-a", self.epoch)

    def test_corruption_and_incomplete_generation_rejected(self):
        generation = snapshot(self.repo, self.spec, "writer-a", self.epoch)
        (generation / "state/node/ldk/manager").write_text("tampered")
        with self.assertRaisesRegex(CapsuleError, "CORRUPT"):
            validate(self.repo, generation)
        (generation / "COMMITTED").unlink()
        with self.assertRaisesRegex(CapsuleError, "INCOMPLETE"):
            validate(self.repo, generation)

    def test_logs_locks_and_symlinks_fail_closed(self):
        (self.source / "node/debug.log").write_text("private diagnostic")
        with self.assertRaisesRegex(CapsuleError, "forbidden"):
            snapshot(self.repo, self.spec, "writer-a", self.epoch)


if __name__ == "__main__": unittest.main()
