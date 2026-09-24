# Capsule A recovery demonstrator

This isolated prototype creates versioned, hash-verified wallet capsules and
enforces one active writer through a local durable epoch registry. It does not
operate on `.var/regtest` and is not a production backup service.

Run its synthetic safety tests with:

```sh
python3 -m unittest tools/capsule/test_capsule.py
```

Use `python3 tools/capsule/capsule.py --help` for the CLI. A snapshot spec is an
explicit list of named source paths, classifications, and capsule destinations.
Any log, lock, symlink, missing component, hash mismatch, incomplete generation,
stale generation, or inactive writer epoch fails closed.
