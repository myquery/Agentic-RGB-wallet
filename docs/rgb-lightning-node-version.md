# Pinned RGB Lightning Node

- Repository: https://github.com/RGB-Tools/rgb-lightning-node
- Commit: `e4008278c80495ea8a8514580b899e771feff872`
- Commit date: `2026-08-26T14:31:34Z`
- OpenAPI: `3.1.0`, API info version `0.1.0`, `openapi.yaml` at this commit.
- `rust-lightning` submodule: `e2a0b8e24dc919ccef289f103fd1f8e1974fcc1d`
- `rgb-lib`: `0.3.0-beta.7`, Git commit
  `2b9af1152f29caf4ac75ae1debdce38088ea9816` from upstream `Cargo.lock`.

Always build with `--locked`. Although upstream Cargo.toml names the rgb-lib
master branch, its lockfile fixes the dependency to the commit above. Do not run
`cargo update` in this checkout or silently move the pin.

The upstream minimum Rust version is 1.94.0; its Dockerfile uses Rust 1.95.
The upstream regtest Compose file specifies Bitcoin Core 30.2, Electrs 0.11.0,
and RGB proxy 0.3.0. Esplora is unnecessary for the Electrum-based demo.

The local integration must isolate its Compose project and bind published ports
to 127.0.0.1. Upstream binds its daemon to 0.0.0.0, so unauthenticated nodes must
run inside containers, not directly on the host. Authentication is disabled only
for this local regtest environment; the application still supports bearer tokens.

Resolved infrastructure images for this run (linux/amd64):

```text
registry.gitlab.com/hashbeam/docker/bitcoind:30.2@sha256:5473c951ca8a703f7c15d9a353f99135e0a1f0cc7ef976f29d073160950098bf
registry.gitlab.com/hashbeam/docker/electrs:0.11.0@sha256:560778444e7fa47a718ffaee371de9066e61aab4a674aae9ab90038899c50da6
ghcr.io/rgb-tools/rgb-proxy-server:0.3.0@sha256:b06fe0f234c53030d54fe1e704daf4bc6ff2879a335124353175130a5112d313
debian:trixie-slim@sha256:d7e12182ce18b85b93007c1dedf31f2d29e01ccf3182cc4017c709b6259bc132
```
