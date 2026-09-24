#!/usr/bin/env python3
"""Real, isolated Capsule A active-channel recovery experiment.

All mutable paths and Docker resources are fixed to the marked capsule-test
fixture. This module never imports or calls the normal Alice/Bob/Carol scripts.
"""
from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request

sys.path.insert(0, str(Path(__file__).resolve().parents[2]))
from tools.capsule.capsule import (
    CapsuleError, Registry, initialize_fixture, restore, snapshot, validate,
)

ROOT = Path(__file__).resolve().parents[2]
FIXTURE = ROOT / ".var/regtest/capsule-test"
MARKER = FIXTURE / ".luma-capsule-disposable"
UPSTREAM = ROOT / ".dev/rgb-lightning-node"
COMPOSE_FILE = FIXTURE / "compose.yaml"
PROJECT = "luma-capsule-test"
PASSWORD = "luma-capsule-test-only"
NODE_REVISION = "e4008278c80495ea8a8514580b899e771feff872"
RUNTIME = "rgb402-rln-runtime:e4008278"
DEBIAN = "debian:trixie-slim@sha256:d7e12182ce18b85b93007c1dedf31f2d29e01ccf3182cc4017c709b6259bc132"
IMAGES = {
    "bitcoind": "registry.gitlab.com/hashbeam/docker/bitcoind:30.2@sha256:5473c951ca8a703f7c15d9a353f99135e0a1f0cc7ef976f29d073160950098bf",
    "electrs": "registry.gitlab.com/hashbeam/docker/electrs:0.11.0@sha256:560778444e7fa47a718ffaee371de9066e61aab4a674aae9ab90038899c50da6",
    "proxy": "ghcr.io/rgb-tools/rgb-proxy-server:0.3.0@sha256:b06fe0f234c53030d54fe1e704daf4bc6ff2879a335124353175130a5112d313",
}
NODES = {"capsule-test-alice": 3201, "capsule-test-bob": 3202}
PEER_PORTS = {"capsule-test-alice": 20835, "capsule-test-bob": 20836}
CAPSULE_REPO = FIXTURE / "capsules"
SPEC = FIXTURE / "capsule-spec.json"
RGB_JOURNAL = FIXTURE / "luma-rgb.jsonl"
BTC_JOURNAL = FIXTURE / "luma-rgb.jsonl.btc.jsonl"
MACHINE_JOURNAL = FIXTURE / "luma-machine.jsonl"
DIAGNOSTICS = FIXTURE / "diagnostics"


class Failure(RuntimeError):
    pass


def say(value: str) -> None:
    print(value, flush=True)


def require_fixture() -> None:
    if not MARKER.is_file() or FIXTURE.is_symlink():
        raise Failure("refusing operation without fixed disposable capsule-test marker")
    if any(part.lower() in {"alice", "bob", "carol"} for part in FIXTURE.parts):
        raise Failure("refusing live wallet path")


def run(args, *, input_text=None, check=True) -> subprocess.CompletedProcess:
    result = subprocess.run([str(x) for x in args], cwd=ROOT, text=True,
                            input=input_text, capture_output=True)
    if check and result.returncode:
        raise Failure(f"{args[0]} failed: {result.stderr.strip() or result.stdout.strip()}")
    return result


def compose(*args, check=True) -> str:
    require_fixture()
    return run(["docker", "compose", "-p", PROJECT, "-f", COMPOSE_FILE, *args], check=check).stdout.strip()


def btc(*args):
    value = compose("exec", "-T", "-u", "blits", "bitcoind", "bitcoin-cli",
                    "-regtest", "-rpcwallet=miner", *args)
    try:
        return json.loads(value)
    except ValueError:
        return value


def api(node: str, endpoint: str, body=None, timeout=120):
    if node not in NODES:
        raise Failure("refusing unknown node")
    request = urllib.request.Request(
        f"http://127.0.0.1:{NODES[node]}/{endpoint}",
        data=None if body is None else json.dumps(body).encode(),
        headers={"Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            result = json.load(response)
        return {} if endpoint == "init" else result
    except urllib.error.HTTPError as error:
        try:
            name = json.loads(error.read()).get("name", "UnknownError")
        except (ValueError, UnicodeError):
            name = "InvalidResponse"
        raise Failure(f"{node} /{endpoint}: HTTP {error.code} ({name})") from None
    except OSError as error:
        raise Failure(f"{node} /{endpoint} unavailable") from error


def wait_for(label, operation, timeout=180):
    deadline = time.monotonic() + timeout
    last = ""
    while time.monotonic() < deadline:
        try:
            value = operation()
            if value:
                return value
        except (Failure, OSError, ValueError, KeyError) as error:
            last = str(error)
        time.sleep(1)
    raise Failure(f"{label} timed out: {last}")


def write_compose() -> None:
    require_fixture()
    uid, gid = os.getuid(), os.getgid()
    for name in ("core", "index", *NODES):
        (FIXTURE / "data" / name).mkdir(parents=True, exist_ok=True)
    lines = ["services:"]
    for service, host, internal, folder, destination in (
        ("bitcoind", 29443, 18443, "core", "/srv/app/.bitcoin"),
        ("electrs", 56001, 50001, "index", "/srv/app/db"),
        ("proxy", 3400, 3000, None, None),
    ):
        lines += [f"  {service}:", "    extends:",
                  f"      file: {json.dumps(str(UPSTREAM / 'compose.yaml'))}",
                  f"      service: {service}", f"    image: {IMAGES[service]}",
                  f"    ports: !override [\"127.0.0.1:{host}:{internal}\"]"]
        if folder:
            lines += [f"    environment: {{MYUID: {uid}, MYGID: {gid}}}",
                      f"    volumes: !override [{json.dumps(str(FIXTURE / 'data' / folder) + ':' + destination)}]"]
    for node in NODES:
        lines += [f"  {node}:", f"    image: {RUNTIME}", f"    user: \"{uid}:{gid}\"",
                  "    command: [\"/data\", \"--daemon-listening-port\", \"3001\", \"--ldk-peer-listening-port\", \"9735\", \"--network\", \"regtest\", \"--disable-authentication\"]",
                  f"    ports: [\"127.0.0.1:{NODES[node]}:3001\", \"127.0.0.1:{PEER_PORTS[node]}:9735\"]",
                  "    volumes:",
                  f"      - {json.dumps(str(UPSTREAM / 'target/debug/rgb-lightning-node') + ':/usr/local/bin/rgb-lightning-node:ro')}",
                  f"      - {json.dumps(str(FIXTURE / 'data' / node) + ':/data')}",
                  "    stop_grace_period: 60s"]
    COMPOSE_FILE.write_text("\n".join(lines) + "\n")


def index_height() -> int:
    with socket.create_connection(("127.0.0.1", 56001), timeout=3) as connection:
        connection.sendall(b'{"id":1,"method":"blockchain.headers.subscribe","params":[]}\n')
        return json.loads(connection.makefile().readline())["result"]["height"]


def mine(count: int) -> None:
    btc("-generate", str(count))
    say(f"mined {count} isolated blocks")


def sync() -> None:
    height = btc("getblockcount")
    wait_for("isolated Electrs sync", lambda: index_height() >= height)
    for node in NODES:
        wait_for(f"{node} chain sync", lambda node=node: api(node, "networkinfo", timeout=5)["height"] >= height)


def unlock(node: str) -> None:
    try:
        api(node, "nodeinfo", timeout=5)
        return
    except Failure as error:
        if "LockedNode" not in str(error):
            raise
    try:
        api(node, "init", {"password": PASSWORD})
    except Failure as error:
        if "AlreadyInitialized" not in str(error):
            raise
    api(node, "unlock", {"password": PASSWORD,
        "ldk_chain_sync": {"mode": "BlockSync", "config": {
            "bitcoind_rpc_username": "user", "bitcoind_rpc_password": "password",
            "bitcoind_rpc_host": "bitcoind", "bitcoind_rpc_port": 18443}},
        "indexer_url": "electrs:50001", "announce_addresses": [], "announce_alias": node})


def start() -> None:
    require_fixture()
    write_compose()
    compose("up", "-d", "bitcoind")
    wait_for("isolated bitcoind", lambda: btc("getblockchaininfo"))
    wallets = btc("listwallets")
    if "miner" not in wallets:
        known = [entry["name"] for entry in btc("listwalletdir")["wallets"]]
        btc("loadwallet" if "miner" in known else "createwallet", "miner")
    height = btc("getblockcount")
    if height < 103:
        mine(103 - height)
    compose("up", "-d", "electrs", "proxy")
    wait_for("isolated Electrs", lambda: index_height() >= btc("getblockcount"))
    compose("up", "-d", *NODES)
    for node in NODES:
        wait_for(f"{node} API", lambda node=node: api_ready(node))
        unlock(node)
    sync()


def api_ready(node: str) -> bool:
    try:
        api(node, "nodeinfo", timeout=3)
        return True
    except Failure as error:
        return "LockedNode" in str(error)


def channels(node: str):
    return api(node, "listchannels")["channels"]


def ready_channel(node: str, channel_id: str):
    return next((item for item in channels(node) if item["channel_id"] == channel_id
                 and item["ready"] and item["is_usable"] and item["status"] == "Opened"), None)


def bootstrap() -> dict:
    require_fixture()
    progress_path = FIXTURE / "setup-progress.json"
    progress = json.loads(progress_path.read_text()) if progress_path.exists() else {}
    for node in NODES:
        balance = api(node, "btcbalance", {"skip_sync": False})
        if sum(part["future"] for part in balance.values()) == 0:
            if progress.get(node + "_funding"):
                raise Failure("uncertain prior funding; refusing duplicate")
            progress[node + "_funding"] = True
            progress_path.write_text(json.dumps(progress))
            btc("sendtoaddress", api(node, "address", {})["address"], "1")
    mine(6); sync()
    for node in NODES:
        try:
            api(node, "createutxos", {"up_to": True, "num": 10, "size": 32500,
                                      "fee_rate": 2, "skip_sync": False})
        except Failure as error:
            if "AllocationsAlreadyAvailable" not in str(error):
                raise
    mine(6); sync()
    alice, bob = tuple(NODES)
    if "asset_id" not in progress:
        asset = api(alice, "issueassetnia", {"amounts": [1000], "ticker": "CAPSULE",
            "name": "Capsule Recovery Asset", "precision": 0})["asset"]
        progress["asset_id"] = asset["asset_id"]
        progress_path.write_text(json.dumps(progress))
    asset_id = progress["asset_id"]
    pubkeys = {node: api(node, "nodeinfo")["pubkey"] for node in NODES}
    api(alice, "connectpeer", {"peer_pubkey_and_addr": pubkeys[bob] + f"@{bob}:9735"})
    matching = [item for item in channels(alice) if item.get("asset_id") == asset_id
                and item.get("peer_pubkey") == pubkeys[bob]]
    if not matching:
        if progress.get("channel_open"):
            raise Failure("uncertain prior channel open; refusing duplicate")
        progress["channel_open"] = True
        progress_path.write_text(json.dumps(progress))
        api(alice, "openchannel", {"peer_pubkey_and_opt_addr": pubkeys[bob] + f"@{bob}:9735",
            "capacity_sat": 100000, "push_msat": 10000000, "asset_id": asset_id,
            "asset_amount": 600, "push_asset_amount": 100, "public": True, "with_anchors": True})
    def funded():
        found = [item for item in channels(alice) if item.get("asset_id") == asset_id
                 and item.get("peer_pubkey") == pubkeys[bob] and item.get("funding_txid")]
        return found[0] if len(found) == 1 else None
    channel = wait_for("isolated channel funding", funded)
    if not channel["ready"]:
        wait_for("isolated channel funding broadcast",
                 lambda: btc("gettxout", channel["funding_txid"], "0"))
        mine(6); sync()
    channel_id = channel["channel_id"]
    for node in NODES:
        wait_for(f"{node} usable channel", lambda node=node: ready_channel(node, channel_id))
    setup = {"asset_id": asset_id, "channel_id": channel_id, "pubkeys": pubkeys}
    (FIXTURE / "setup.json").write_text(json.dumps(setup))
    return setup


def wallet_env(asset_id: str) -> dict:
    env = os.environ.copy()
    env.update(RGB_NODE_URL=f"http://127.0.0.1:{NODES['capsule-test-alice']}", RGB_NODE_TOKEN="",
               ALLOWED_ASSET_IDS=asset_id, AUTO_APPROVE_BELOW="1", MAX_SINGLE_PAYMENT="100",
               MAX_DAILY_SPEND="500", MAX_CARRIER_MSAT="3000000",
               WALLET_STATE_PATH=str(RGB_JOURNAL))
    return env


def wallet(args, asset_id: str, input_text=None, check=True):
    binary = ROOT / "target/debug/wallet"
    if not binary.is_file():
        run(["cargo", "build", "--locked", "-p", "buyer-agent", "--bin", "wallet"])
    result = subprocess.run([str(binary), *args], cwd=ROOT, env=wallet_env(asset_id),
                            text=True, input=input_text, capture_output=True)
    DIAGNOSTICS.mkdir(exist_ok=True)
    with (DIAGNOSTICS / "wallet.log").open("a") as stream:
        stream.write(result.stderr)
    if check and result.returncode:
        raise Failure(f"wallet {' '.join(args[:1])} failed; inspect disposable diagnostics")
    return result


def payment(setup: dict, label: str, amount=5) -> dict:
    bob = "capsule-test-bob"
    invoice = api(bob, "lninvoice", {"amt_msat": 3000000, "expiry_sec": 3600,
        "asset_id": setup["asset_id"], "asset_amount": amount, "description": label})["invoice"]
    decoded = api("capsule-test-alice", "decodelninvoice", {"invoice": invoice})
    result = wallet(["pay", invoice], setup["asset_id"], input_text="yes\n")
    if result.returncode:
        raise Failure("approved disposable payment failed")
    payment_hash = decoded["payment_hash"]
    def settled():
        status = json.loads(wallet(["status", payment_hash], setup["asset_id"]).stdout)
        if status == "Failed":
            raise Failure("disposable payment failed")
        return status if status == "Settled" else None
    wait_for("disposable payment settlement", settled, 90)
    return {"invoice": invoice, "payment_hash": payment_hash, "amount": amount, "status": "Settled"}


def fingerprint() -> str:
    return (FIXTURE / "data/capsule-test-alice/wallet_master_fingerprint").read_text().strip()


def balance_state(setup: dict) -> dict:
    alice = "capsule-test-alice"
    channel = ready_channel(alice, setup["channel_id"])
    if not channel:
        raise Failure("expected active channel missing")
    return {"node_id": api(alice, "nodeinfo")["pubkey"], "fingerprint": fingerprint(),
            "network": "regtest", "btc": api(alice, "btcbalance", {"skip_sync": False}),
            "rgb": api(alice, "assetbalance", {"asset_id": setup["asset_id"]}),
            "channel": channel, "journal_sha256": file_hash(RGB_JOURNAL),
            "journal_sequence": journal_sequence(RGB_JOURNAL)}


def file_hash(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def journal_sequence(path: Path) -> int:
    return sum(1 for line in path.read_text().splitlines() if line.strip())


def stop_node(node: str) -> None:
    compose("stop", "-t", "60", node)
    container = f"{PROJECT}-{node}-1"
    running = run(["docker", "inspect", "-f", "{{.State.Running}}", container]).stdout.strip()
    if running != "false":
        raise Failure(f"{node} did not stop cleanly")


def component_spec(setup: dict) -> dict:
    node_root = FIXTURE / "data/capsule-test-alice"
    components = []
    forbidden = {"logs", "log"}
    for source in sorted(path for path in node_root.rglob("*") if path.is_file()):
        relative = source.relative_to(node_root)
        if source.is_symlink() or any(part in forbidden or part.endswith((".log", ".lock")) for part in relative.parts):
            continue
        classification = "AUTHORITY" if relative.as_posix() == "mnemonic" else "SAFETY-CRITICAL"
        if relative.as_posix() in {"wallet_master_fingerprint", "indexer_url"}:
            classification = "RECOVERY-METADATA"
        components.append({"name": "node_" + hashlib.sha256(relative.as_posix().encode()).hexdigest()[:16],
            "class": classification, "source": str(source),
            "destination": "node/" + relative.as_posix()})
    for name, source in (("rgb_journal", RGB_JOURNAL), ("btc_journal", BTC_JOURNAL),
                         ("machine_journal", MACHINE_JOURNAL)):
        source.touch(exist_ok=True)
        components.append({"name": name, "class": "APPLICATION-JOURNAL", "source": str(source),
                           "destination": "app/" + source.name, "journal": True})
    return {"wallet_id": "capsule-test-alice", "wallet_fingerprint": hash_id(fingerprint()),
            "node_id": hash_id(setup["pubkeys"]["capsule-test-alice"]), "network": "regtest",
            "implementation": {"project_revision": run(["git", "rev-parse", "HEAD"]).stdout.strip(),
                               "rgb_lightning_revision": NODE_REVISION}, "components": components}


def hash_id(value: str) -> str:
    return hashlib.sha256(value.encode()).hexdigest()


def create_snapshot(setup: dict, owner: str, epoch: int) -> Path:
    SPEC.write_text(json.dumps(component_spec(setup), indent=2) + "\n")
    started = time.monotonic()
    generation = snapshot(CAPSULE_REPO, SPEC, owner, epoch)
    duration = time.monotonic() - started
    validate(CAPSULE_REPO, generation)
    (DIAGNOSTICS / f"snapshot-{generation.name}.json").write_text(json.dumps({"seconds": duration}))
    return generation


def remove_active_alice() -> None:
    require_fixture()
    node = FIXTURE / "data/capsule-test-alice"
    if node.exists():
        shutil.rmtree(node)
    for path in (RGB_JOURNAL, BTC_JOURNAL, MACHINE_JOURNAL, RGB_JOURNAL.with_suffix(RGB_JOURNAL.suffix + ".lock")):
        path.unlink(missing_ok=True)


def restore_active(generation: Path, owner: str, epoch: int) -> float:
    target = FIXTURE / "capsule-test-restored"
    shutil.rmtree(target, ignore_errors=True)
    started = time.monotonic()
    restore(CAPSULE_REPO, generation, target, owner, epoch)
    duration = time.monotonic() - started
    node_target = FIXTURE / "data/capsule-test-alice"
    if node_target.exists():
        raise Failure("restore target unexpectedly exists")
    shutil.move(target / "node", node_target)
    app = target / "app"
    shutil.move(app / RGB_JOURNAL.name, RGB_JOURNAL)
    shutil.move(app / BTC_JOURNAL.name, BTC_JOURNAL)
    shutil.move(app / MACHINE_JOURNAL.name, MACHINE_JOURNAL)
    shutil.rmtree(target)
    return duration


def assert_same(before: dict, after: dict) -> None:
    for key in ("node_id", "fingerprint", "network", "btc", "rgb", "journal_sha256", "journal_sequence"):
        if before[key] != after[key]:
            raise Failure(f"restore mismatch: {key}")
    for key in ("channel_id", "funding_txid", "peer_pubkey", "capacity_sat", "asset_id"):
        if before["channel"].get(key) != after["channel"].get(key):
            raise Failure(f"restore channel mismatch: {key}")


def copy_variant(source: Path, name: str) -> Path:
    target = FIXTURE / "variants" / name
    shutil.rmtree(target, ignore_errors=True)
    target.parent.mkdir(exist_ok=True)
    shutil.copytree(source, target)
    return target


def expect_refusal(path: Path, label: str, allow_stale: bool = False) -> str:
    try:
        validate(CAPSULE_REPO, path, allow_stale=allow_stale)
    except CapsuleError as error:
        return f"{label}: {str(error).split(':', 1)[0]}"
    raise Failure(f"invalid capsule activated: {label}")


def real_corruption_matrix(current: Path, previous: Path) -> list[str]:
    results = []
    manifest = json.loads((current / "manifest.json").read_text())
    files = [current / "state" / c["destination"] for c in manifest["components"].values()]
    def find_fragment(fragment):
        return next((p for p in files if fragment in p.as_posix()), None)
    cases = {"missing_manager": find_fragment("/.ldk/manager"),
             "missing_monitor": next((p for p in files if "/.ldk/monitors/" in p.as_posix()), None),
             "missing_monitor_update": next((p for p in files if "/.ldk/monitor_updates/" in p.as_posix()), None),
             "missing_rgb_db": find_fragment("/rgb_lib_db"),
             "missing_journal": find_fragment("/app/luma-rgb.jsonl")}
    for label, original in cases.items():
        if original is None:
            results.append(label + ": BLOCKED_ABSENT")
            continue
        variant = copy_variant(current, label)
        (variant / original.relative_to(current)).unlink()
        results.append(expect_refusal(variant, label))
    variant = copy_variant(current, "corrupt_manifest")
    (variant / "manifest.json").write_text("{")
    results.append(expect_refusal(variant, "corrupt_manifest"))
    variant = copy_variant(current, "wrong_hash")
    target = find_fragment("/.ldk/manager")
    (variant / target.relative_to(current)).write_bytes(b"wrong")
    results.append(expect_refusal(variant, "wrong_hash"))
    variant = copy_variant(current, "missing_committed"); (variant / "COMMITTED").unlink()
    results.append(expect_refusal(variant, "missing_committed"))
    variant = copy_variant(current, "extra_file"); (variant / "state/undeclared").write_text("extra")
    results.append(expect_refusal(variant, "extra_file"))
    # Real cross-store skew: replace current journal bytes with prior generation bytes.
    current_manifest = manifest
    journal = current_manifest["components"]["rgb_journal"]["destination"]
    variant = copy_variant(current, "skew_old_journal")
    shutil.copyfile(previous / "state" / journal, variant / "state" / journal)
    results.append(expect_refusal(variant, "skew_old_journal"))
    manager = current_manifest["components"][next(name for name, component in current_manifest["components"].items()
        if component["destination"].endswith("/.ldk/manager"))]["destination"]
    variant = copy_variant(current, "skew_old_node")
    shutil.copyfile(previous / "state" / manager, variant / "state" / manager)
    results.append(expect_refusal(variant, "skew_old_node"))
    previous_manifest = json.loads((previous / "manifest.json").read_text())
    previous_journal = previous_manifest["components"]["rgb_journal"]["destination"]
    variant = copy_variant(previous, "skew_new_journal_into_old")
    shutil.copyfile(current / "state" / journal, variant / "state" / previous_journal)
    results.append(expect_refusal(variant, "skew_new_journal_into_old", allow_stale=True))
    return results


def dir_size(path: Path) -> int:
    return sum(item.stat().st_size for item in path.rglob("*") if item.is_file())


def redacted_evidence(setup: dict, before: dict, after: dict, generation1: Path,
                      generation2: Path, restore_seconds: float, matrix: list[str], epoch: int) -> dict:
    manifest1 = json.loads((generation1 / "manifest.json").read_text())
    manifest2 = json.loads((generation2 / "manifest.json").read_text())
    node_root = FIXTURE / "data/capsule-test-alice"
    ldk = next((path for path in node_root.rglob(".ldk") if path.is_dir()), node_root / ".ldk")
    rgb_paths = [p for p in node_root.rglob("*") if p.is_file() and ("rgb" in p.name.lower() or "rgb" in p.as_posix())]
    return {"observed_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "identifiers": {"wallet_fingerprint_sha256": hash_id(before["fingerprint"]),
            "node_id_sha256": hash_id(before["node_id"]),
            "channel_id_sha256": hash_id(before["channel"]["channel_id"]),
            "asset_id_sha256": hash_id(setup["asset_id"])},
        "identity_equal": before["node_id"] == after["node_id"] and before["fingerprint"] == after["fingerprint"],
        "btc_equal": before["btc"] == after["btc"], "rgb_equal": before["rgb"] == after["rgb"],
        "channel_equal": before["channel"]["channel_id"] == after["channel"]["channel_id"],
        "journal_equal": before["journal_sha256"] == after["journal_sha256"],
        "generations": {"first": manifest1["generation"], "second": manifest2["generation"],
            "second_previous": manifest2["previous_generation"], "epoch": epoch,
            "journal_sequence_first": manifest1["components"]["rgb_journal"]["journal_sequence"],
            "journal_sequence_second": manifest2["components"]["rgb_journal"]["journal_sequence"]},
        "measurements_bytes": {"node": dir_size(node_root), "ldk": dir_size(ldk),
            "rgb_named_files": sum(p.stat().st_size for p in rgb_paths),
            "luma_rgb_journal": RGB_JOURNAL.stat().st_size, "capsule_generation_1": dir_size(generation1),
            "capsule_generation_2": dir_size(generation2)},
        "restore_seconds": restore_seconds, "corruption_matrix": matrix}


def finish_after_restore(setup: dict, before: dict, first: dict, generation1: Path,
                         registry: Registry, epoch: int, restore_seconds: float) -> None:
    wait_for("restored active channel", lambda: ready_channel("capsule-test-alice", setup["channel_id"]))
    after = balance_state(setup)
    assert_same(before, after)
    say("[6/8] verifying duplicate refusal and making post-restore payment")
    duplicate = wallet(["pay", first["invoice"]], setup["asset_id"], input_text="yes\n", check=False)
    if duplicate.returncode == 0 or journal_sequence(RGB_JOURNAL) != before["journal_sequence"]:
        raise Failure("duplicate payment was not rejected without journal advancement")
    second = payment(setup, "capsule after restore")
    payments = json.loads((DIAGNOSTICS / "payments.json").read_text()); payments["second"] = second
    (DIAGNOSTICS / "payments.json").write_text(json.dumps(payments))
    say("[7/8] clean-stopping and committing generation 2")
    stop_node("capsule-test-alice")
    generation2 = create_snapshot(setup, "runtime-a", epoch)
    try:
        validate(CAPSULE_REPO, generation1)
        raise Failure("stale real generation activated")
    except CapsuleError as error:
        if "STALE" not in str(error): raise
    try:
        registry.acquire("capsule-test-alice", "runtime-b")
        raise Failure("second writer acquired without takeover")
    except CapsuleError as error:
        if "SPLIT_BRAIN" not in str(error): raise
    new_epoch = registry.acquire("capsule-test-alice", "runtime-b", takeover=True)
    try:
        create_snapshot(setup, "runtime-a", epoch)
        raise Failure("old writer remained active")
    except CapsuleError as error:
        if "SPLIT_BRAIN" not in str(error): raise
    matrix = real_corruption_matrix(generation2, generation1)
    evidence = redacted_evidence(setup, before, after, generation1, generation2,
                                 restore_seconds, matrix, new_epoch)
    (FIXTURE / "redacted-evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
    say("[8/8] experiment complete; disposable restored node remains stopped")
    say(json.dumps(evidence, indent=2))


def continue_after_restore() -> None:
    require_fixture()
    setup = json.loads((FIXTURE / "setup.json").read_text())
    before = json.loads((DIAGNOSTICS / "baseline.json").read_text())
    first = json.loads((DIAGNOSTICS / "payments.json").read_text())["first"]
    registry = Registry(CAPSULE_REPO)
    state = registry.read()["wallets"].get("capsule-test-alice")
    if not state or state["owner"] != "runtime-a" or state["current_generation"] != 1:
        raise Failure("continuation requires runtime-a ownership of committed generation 1")
    if (CAPSULE_REPO / "wallets/capsule-test-alice/generations/2").exists():
        raise Failure("continuation refuses an existing generation 2")
    generation1 = CAPSULE_REPO / "wallets/capsule-test-alice/generations/1"
    validate(CAPSULE_REPO, generation1)
    say("[5/8] repeating measured destruction and restore from generation 1")
    stop_node("capsule-test-alice")
    remove_active_alice()
    restore_seconds = restore_active(generation1, "runtime-a", state["epoch"])
    compose("up", "-d", "capsule-test-alice")
    wait_for("restored node API", lambda: api_ready("capsule-test-alice")); unlock("capsule-test-alice")
    finish_after_restore(setup, before, first, generation1, registry, state["epoch"], restore_seconds)


def verify_and_enrich_evidence() -> None:
    require_fixture()
    evidence_path = FIXTURE / "redacted-evidence.json"
    evidence = json.loads(evidence_path.read_text())
    setup = json.loads((FIXTURE / "setup.json").read_text())
    before = json.loads((DIAGNOSTICS / "baseline.json").read_text())
    payments = json.loads((DIAGNOSTICS / "payments.json").read_text())
    registry = Registry(CAPSULE_REPO)
    state = registry.read()["wallets"]["capsule-test-alice"]
    if state["owner"] != "runtime-b" or state["current_generation"] != 2:
        raise Failure("evidence verification requires runtime-b ownership of generation 2")
    generation1 = CAPSULE_REPO / "wallets/capsule-test-alice/generations/1"
    generation2 = CAPSULE_REPO / "wallets/capsule-test-alice/generations/2"
    compose("stop", "-t", "60", "capsule-test-alice")
    remove_active_alice()
    restore_seconds = restore_active(generation2, "runtime-b", state["epoch"])
    reconcile_started = time.monotonic()
    compose("up", "-d", "capsule-test-alice")
    wait_for("generation 2 node API", lambda: api_ready("capsule-test-alice")); unlock("capsule-test-alice")
    wait_for("generation 2 active channel", lambda: ready_channel("capsule-test-alice", setup["channel_id"]))
    current = balance_state(setup)
    reconciliation_seconds = time.monotonic() - reconcile_started
    if current["node_id"] != before["node_id"] or current["fingerprint"] != before["fingerprint"]:
        raise Failure("generation 2 identity mismatch")
    second_status = json.loads(wallet(["status", payments["second"]["payment_hash"]], setup["asset_id"]).stdout)
    if second_status != "Settled":
        raise Failure("post-restore payment status did not reconcile as settled")
    rgb_delta = before["rgb"]["offchain_outbound"] - current["rgb"]["offchain_outbound"]
    if rgb_delta != payments["second"]["amount"]:
        raise Failure("post-restore RGB balance did not reconcile by payment amount")
    matrix = real_corruption_matrix(generation2, generation1)
    node_root = FIXTURE / "data/capsule-test-alice"
    transfer_bytes = sum(path.stat().st_size for path in node_root.rglob("*")
                         if path.is_file() and "transfer" in path.as_posix().lower())
    evidence.update({"post_restore_payment": {"status": "Settled", "rgb_outbound_delta": rgb_delta,
        "journal_sequence": journal_sequence(RGB_JOURNAL), "payment_count": 2},
        "counts": {"channels": 1, "rgb_assets": 1, "settled_payments": 2},
        "timings_seconds": {"snapshot_generation_1": json.loads((DIAGNOSTICS / "snapshot-1.json").read_text())["seconds"],
            "snapshot_generation_2": json.loads((DIAGNOSTICS / "snapshot-2.json").read_text())["seconds"],
            "restore_generation_1": evidence["restore_seconds"], "restore_generation_2": restore_seconds,
            "reconciliation_generation_2": reconciliation_seconds},
        "transfer_artifacts_bytes": transfer_bytes, "corruption_matrix": matrix,
        "uncertain_submission": "BLOCKED — no safe submission-boundary hook"})
    evidence_path.write_text(json.dumps(evidence, indent=2) + "\n")
    stop_node("capsule-test-alice")
    say(json.dumps(evidence, indent=2))


def run_experiment(resume: bool = False) -> None:
    if FIXTURE.exists():
        if not resume:
            raise Failure("capsule-test fixture already exists; inspect or use the guarded resume command")
        require_fixture()
        if (DIAGNOSTICS / "payments.json").exists() or CAPSULE_REPO.exists():
            raise Failure("resume is allowed only before any payment or capsule generation exists")
    else:
        if resume:
            raise Failure("no disposable fixture exists to resume")
        initialize_fixture(FIXTURE)
        DIAGNOSTICS.mkdir()
    say("[1/8] starting fully isolated chain, indexer, proxy, and nodes")
    start()
    say("[2/8] funding disposable nodes and opening active RGB channel")
    setup = bootstrap()
    say("[3/8] making baseline application-approved Luma payment")
    first = payment(setup, "capsule baseline")
    (DIAGNOSTICS / "payments.json").write_text(json.dumps({"first": first}))
    before = balance_state(setup)
    (DIAGNOSTICS / "baseline.json").write_text(json.dumps(before))
    registry = Registry(CAPSULE_REPO)
    epoch = registry.acquire("capsule-test-alice", "runtime-a")
    say("[4/8] clean-stopping disposable wallet and committing generation 1")
    stop_node("capsule-test-alice")
    generation1 = create_snapshot(setup, "runtime-a", epoch)
    say("[5/8] destroying and restoring disposable wallet state")
    remove_active_alice()
    restore_seconds = restore_active(generation1, "runtime-a", epoch)
    compose("up", "-d", "capsule-test-alice")
    wait_for("restored node API", lambda: api_ready("capsule-test-alice")); unlock("capsule-test-alice")
    finish_after_restore(setup, before, first, generation1, registry, epoch, restore_seconds)


def stop_all() -> None:
    require_fixture(); compose("stop")


def destroy_fixture(yes: bool) -> None:
    require_fixture()
    if not yes:
        raise Failure("destruction requires --yes and applies only to the marked capsule-test fixture")
    compose("down", check=False)
    data = FIXTURE / "data"
    if data.exists():
        run(["docker", "run", "--rm", "--user", "0:0", "--entrypoint", "/bin/rm",
             "--mount", f"type=bind,src={data},dst=/state", DEBIAN, "-rf", "--", "/state"])
    shutil.rmtree(FIXTURE)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["run", "resume", "continue", "verify-evidence", "stop", "destroy"])
    parser.add_argument("--yes", action="store_true")
    args = parser.parse_args()
    if args.command == "run":
        if not args.yes: raise Failure("real destructive disposable experiment requires --yes")
        run_experiment()
    elif args.command == "resume":
        if not args.yes: raise Failure("real destructive disposable experiment requires --yes")
        run_experiment(resume=True)
    elif args.command == "continue":
        if not args.yes: raise Failure("real destructive disposable experiment requires --yes")
        continue_after_restore()
    elif args.command == "verify-evidence":
        if not args.yes: raise Failure("real destructive disposable experiment requires --yes")
        verify_and_enrich_evidence()
    elif args.command == "stop": stop_all()
    else: destroy_fixture(args.yes)


if __name__ == "__main__":
    try:
        main()
    except (Failure, CapsuleError, OSError, ValueError, KeyError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        raise SystemExit(1)
