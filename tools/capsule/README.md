# Capsule A recovery demonstrator

This isolated prototype creates versioned, hash-verified wallet capsules and
enforces one active writer through a local durable epoch registry. It does not
operate on `.var/regtest` and is not a production backup service.

Run its synthetic safety tests with:

```sh
python3 -m unittest tools/capsule/test_capsule.py
```

Initialize only an empty, unmistakably named fixture:

```sh
python3 tools/capsule/capsule.py init-fixture .var/regtest/capsule-test
```

All other commands require `--repo` beneath that marker. Paths containing a
wallet component named `alice`, `bob`, or `carol`, and normal `.var/regtest`
paths outside `capsule-test`, are refused in code.

Use `python3 tools/capsule/capsule.py --help` for the CLI. A snapshot spec is an
explicit list of named source paths, classifications, and capsule destinations.
Any log, lock, symlink, missing component, hash mismatch, incomplete generation,
stale generation, or inactive writer epoch fails closed.
