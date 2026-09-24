#!/usr/bin/env python3
"""Disposable Capsule A snapshot/restore and writer-fencing demonstrator."""
from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import tempfile
from datetime import datetime, timezone

SCHEMA_VERSION = 1
CLASSES = {"AUTHORITY", "SAFETY-CRITICAL", "APPLICATION-JOURNAL", "RECOVERY-METADATA"}
FORBIDDEN_PARTS = {"logs", "log", "conversation", "sessions"}
RESERVED_WALLETS = {"alice", "bob", "carol"}
DISPOSABLE_MARKER = ".luma-capsule-disposable"


class CapsuleError(RuntimeError):
    pass


def reject_live_or_reserved(path: Path) -> Path:
    resolved = path.resolve()
    lowered = tuple(part.lower() for part in resolved.parts)
    if any(part in RESERVED_WALLETS for part in lowered):
        raise CapsuleError("LIVE_FIXTURE: alice/bob/carol paths are forbidden")
    rendered = resolved.as_posix().lower()
    if "/.var/regtest" in rendered and "/.var/regtest/capsule-test/" not in rendered + "/":
        raise CapsuleError("LIVE_FIXTURE: normal .var/regtest paths are forbidden")
    return resolved


def disposable_root(path: Path) -> Path:
    resolved = reject_live_or_reserved(path)
    candidate = resolved if resolved.is_dir() else resolved.parent
    for parent in (candidate, *candidate.parents):
        if (parent / DISPOSABLE_MARKER).is_file():
            return parent
    raise CapsuleError(f"NOT_DISPOSABLE: missing {DISPOSABLE_MARKER} marker")


def initialize_fixture(path: Path) -> Path:
    resolved = reject_live_or_reserved(path)
    if "capsule-test" not in resolved.name.lower():
        raise CapsuleError("NOT_DISPOSABLE: fixture directory name must contain capsule-test")
    if resolved.exists() and any(resolved.iterdir()):
        raise CapsuleError("NOT_DISPOSABLE: fixture directory must be absent or empty")
    resolved.mkdir(parents=True, exist_ok=True)
    marker = resolved / DISPOSABLE_MARKER
    marker.write_text("disposable capsule recovery fixture\n")
    return marker


def atomic_json(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as stream:
            json.dump(value, stream, indent=2, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(tmp, path)
        directory = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.exists(tmp):
            os.unlink(tmp)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def safe_relative(value: str) -> PurePosixPath:
    path = PurePosixPath(value)
    if path.is_absolute() or not path.parts or ".." in path.parts:
        raise CapsuleError(f"unsafe relative path: {value}")
    if any(part in FORBIDDEN_PARTS or part.endswith(".log") or part.endswith(".lock") for part in path.parts):
        raise CapsuleError(f"forbidden capsule path: {value}")
    return path


def load_spec(path: Path) -> dict:
    fixture = disposable_root(path)
    spec = json.loads(path.read_text())
    required = {"wallet_id", "wallet_fingerprint", "node_id", "network", "implementation", "components"}
    missing = required - spec.keys()
    if missing:
        raise CapsuleError(f"spec missing: {', '.join(sorted(missing))}")
    if not isinstance(spec["components"], list) or not spec["components"]:
        raise CapsuleError("spec requires a non-empty components allowlist")
    if spec["wallet_id"].lower() in RESERVED_WALLETS:
        raise CapsuleError("LIVE_FIXTURE: alice/bob/carol wallet IDs are forbidden")
    names = set()
    destinations = set()
    for item in spec["components"]:
        if set(item) - {"name", "class", "source", "destination", "journal"}:
            raise CapsuleError(f"unsupported component field in {item.get('name', '?')}")
        if item.get("class") not in CLASSES:
            raise CapsuleError(f"invalid component class: {item.get('class')}")
        name = safe_relative(item["name"]).as_posix()
        destination = safe_relative(item["destination"]).as_posix()
        if name in names or destination in destinations:
            raise CapsuleError("duplicate component name or destination")
        names.add(name)
        destinations.add(destination)
        source = Path(item["source"])
        if disposable_root(source) != fixture:
            raise CapsuleError(f"component outside disposable fixture: {name}")
        if not source.exists() or source.is_symlink():
            raise CapsuleError(f"component source unavailable or symlinked: {name}")
    return spec


class Registry:
    def __init__(self, root: Path):
        self.root = reject_live_or_reserved(root)
        disposable_root(self.root)
        self.path = root / "registry.json"
        self.lock_path = root / "registry.lock"

    def mutate(self, operation):
        self.root.mkdir(parents=True, exist_ok=True)
        with self.lock_path.open("a+") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            state = json.loads(self.path.read_text()) if self.path.exists() else {"wallets": {}}
            result = operation(state)
            atomic_json(self.path, state)
            return result

    def read(self) -> dict:
        return json.loads(self.path.read_text()) if self.path.exists() else {"wallets": {}}

    def acquire(self, wallet: str, owner: str, takeover: bool = False) -> int:
        if wallet.lower() in RESERVED_WALLETS:
            raise CapsuleError("LIVE_FIXTURE: alice/bob/carol wallet IDs are forbidden")
        def op(state):
            current = state["wallets"].get(wallet)
            if current and current["owner"] != owner and not takeover:
                raise CapsuleError("SPLIT_BRAIN: wallet already has an active writer")
            if current and current["owner"] == owner:
                return current["epoch"]
            epoch = 1 if not current else current["epoch"] + 1
            state["wallets"][wallet] = {
                "owner": owner, "epoch": epoch,
                "current_generation": 0 if not current else current["current_generation"],
            }
            return epoch
        return self.mutate(op)

    def assert_writer(self, wallet: str, owner: str, epoch: int) -> dict:
        current = self.read()["wallets"].get(wallet)
        if not current or current["owner"] != owner or current["epoch"] != epoch:
            raise CapsuleError("SPLIT_BRAIN: stale or inactive writer epoch")
        return current


def enumerate_files(source: Path):
    paths = [source] if source.is_file() else sorted(p for p in source.rglob("*") if p.is_file())
    for path in paths:
        relative = PurePosixPath(path.name) if source.is_file() else PurePosixPath(path.relative_to(source).as_posix())
        safe_relative(relative.as_posix())
        if path.is_symlink():
            raise CapsuleError(f"symlink not allowed: {path}")
        yield path, relative


def component_sequence(source: Path, journal: bool) -> int | None:
    if not journal:
        return None
    if not source.is_file():
        raise CapsuleError("journal component must be a file")
    with source.open("rb") as stream:
        return sum(1 for line in stream if line.strip())


def snapshot(repo: Path, spec_path: Path, owner: str, epoch: int) -> Path:
    if disposable_root(repo) != disposable_root(spec_path):
        raise CapsuleError("repository and spec must share one disposable fixture")
    spec = load_spec(spec_path)
    wallet = spec["wallet_id"]
    registry = Registry(repo)
    current = registry.assert_writer(wallet, owner, epoch)
    generation = current["current_generation"] + 1
    wallet_root = repo / "wallets" / wallet
    final = wallet_root / "generations" / str(generation)
    if final.exists():
        raise CapsuleError("generation directory already exists")
    staging = wallet_root / "generations" / f".{generation}.partial-{os.getpid()}"
    staging.mkdir(parents=True)
    manifest_components = {}
    try:
        for item in spec["components"]:
            registry.assert_writer(wallet, owner, epoch)
            source = Path(item["source"])
            destination = safe_relative(item["destination"])
            component_root = staging / "state" / destination
            files = []
            total = 0
            for src, rel in enumerate_files(source):
                dst = component_root if source.is_file() else component_root / rel
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(src, dst)
                size = dst.stat().st_size
                total += size
                files.append({"path": rel.as_posix(), "size": size, "sha256": sha256(dst)})
            manifest_components[item["name"]] = {
                "class": item["class"], "destination": destination.as_posix(),
                "kind": "file" if source.is_file() else "directory",
                "size": total, "journal_sequence": component_sequence(source, item.get("journal", False)),
                "files": files,
            }
        manifest = {
            "schema_version": SCHEMA_VERSION, "wallet_id": wallet,
            "wallet_fingerprint": spec["wallet_fingerprint"], "node_id": spec["node_id"],
            "network": spec["network"], "implementation": spec["implementation"],
            "epoch": epoch, "generation": generation,
            "previous_generation": current["current_generation"],
            "created_at": datetime.now(timezone.utc).isoformat(), "components": manifest_components,
        }
        atomic_json(staging / "manifest.json", manifest)
        (staging / "COMMITTED").write_text("\n")
        os.replace(staging, final)
        def commit(state):
            active = state["wallets"].get(wallet)
            if not active or active["owner"] != owner or active["epoch"] != epoch:
                raise CapsuleError("SPLIT_BRAIN: writer changed before commit")
            if active["current_generation"] != generation - 1:
                raise CapsuleError("STALE: generation advanced during export")
            active["current_generation"] = generation
        registry.mutate(commit)
        return final
    except Exception:
        shutil.rmtree(staging, ignore_errors=True)
        raise


def validate(repo: Path, generation_dir: Path, allow_stale: bool = False) -> dict:
    if disposable_root(repo) != disposable_root(generation_dir):
        raise CapsuleError("repository and generation must share one disposable fixture")
    if not (generation_dir / "COMMITTED").is_file():
        raise CapsuleError("INCOMPLETE: missing COMMITTED marker")
    try:
        manifest = json.loads((generation_dir / "manifest.json").read_text())
    except Exception as error:
        raise CapsuleError(f"CORRUPT: manifest unreadable: {error}") from error
    if manifest.get("schema_version") != SCHEMA_VERSION:
        raise CapsuleError("INCOMPATIBLE: schema version")
    wallet = manifest["wallet_id"]
    current = Registry(repo).read()["wallets"].get(wallet)
    if not current:
        raise CapsuleError("INCOMPLETE: wallet absent from epoch registry")
    if not allow_stale and manifest["generation"] != current["current_generation"]:
        raise CapsuleError("STALE: capsule is not the durable current generation")
    if manifest["epoch"] > current["epoch"]:
        raise CapsuleError("INCOMPATIBLE: capsule epoch is ahead of registry")
    declared_files = set()
    for name, component in manifest["components"].items():
        safe_relative(name)
        root = generation_dir / "state" / safe_relative(component["destination"])
        expected = set()
        for entry in component["files"]:
            rel = safe_relative(entry["path"])
            path = root if component["kind"] == "file" else root / rel
            expected.add(path)
            declared_files.add(path)
            if not path.is_file() or path.stat().st_size != entry["size"] or sha256(path) != entry["sha256"]:
                raise CapsuleError(f"CORRUPT: component hash mismatch: {name}")
        actual = {p for p in ([root] if component["kind"] == "file" and root.exists() else root.rglob("*")) if p.is_file()}
        if actual != expected:
            raise CapsuleError(f"CORRUPT: unexpected or missing files: {name}")
    state_root = generation_dir / "state"
    actual_files = {path for path in state_root.rglob("*") if path.is_file()}
    if actual_files != declared_files:
        raise CapsuleError("CORRUPT: undeclared or missing capsule state")
    return manifest


def restore(repo: Path, generation_dir: Path, target: Path, owner: str, epoch: int) -> None:
    fixture = disposable_root(repo)
    if disposable_root(generation_dir) != fixture:
        raise CapsuleError("generation is outside disposable fixture")
    target = reject_live_or_reserved(target)
    if target.exists():
        if disposable_root(target) != fixture:
            raise CapsuleError("restore target is outside disposable fixture")
    elif disposable_root(target.parent) != fixture:
        raise CapsuleError("restore target is outside disposable fixture")
    manifest = validate(repo, generation_dir)
    Registry(repo).assert_writer(manifest["wallet_id"], owner, epoch)
    if target.exists() and any(target.iterdir()):
        raise CapsuleError("restore target must be absent or empty")
    partial = target.with_name(f".{target.name}.partial-{os.getpid()}")
    shutil.rmtree(partial, ignore_errors=True)
    shutil.copytree(generation_dir / "state", partial)
    Registry(repo).assert_writer(manifest["wallet_id"], owner, epoch)
    os.replace(partial, target)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", type=Path)
    sub = parser.add_subparsers(dest="command", required=True)
    initialize = sub.add_parser("init-fixture")
    initialize.add_argument("target", type=Path)
    acquire = sub.add_parser("acquire")
    acquire.add_argument("--wallet", required=True); acquire.add_argument("--owner", required=True)
    acquire.add_argument("--takeover", action="store_true")
    snap = sub.add_parser("snapshot")
    snap.add_argument("--spec", type=Path, required=True); snap.add_argument("--owner", required=True); snap.add_argument("--epoch", type=int, required=True)
    check = sub.add_parser("validate"); check.add_argument("generation", type=Path)
    recover = sub.add_parser("restore")
    recover.add_argument("generation", type=Path); recover.add_argument("target", type=Path)
    recover.add_argument("--owner", required=True); recover.add_argument("--epoch", type=int, required=True)
    args = parser.parse_args()
    try:
        if args.command == "init-fixture":
            print(initialize_fixture(args.target))
        elif args.repo is None:
            raise CapsuleError("--repo is required for this command")
        elif args.command == "acquire":
            print(Registry(args.repo).acquire(args.wallet, args.owner, args.takeover))
        elif args.command == "snapshot":
            print(snapshot(args.repo, args.spec, args.owner, args.epoch))
        elif args.command == "validate":
            print(json.dumps(validate(args.repo, args.generation), indent=2, sort_keys=True))
        else:
            restore(args.repo, args.generation, args.target, args.owner, args.epoch)
    except CapsuleError as error:
        parser.exit(2, f"capsule refused: {error}\n")


if __name__ == "__main__":
    main()
