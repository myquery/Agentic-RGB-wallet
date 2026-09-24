#!/usr/bin/env bash
set -euo pipefail

version="0.1.0-beta.15"
expected_sha256="2e8c740bf4402da30739d97ab30bd8cded737284ee60b5fbde7e8503757cb200"
asset="utexo__rgb-lightning-node-bare-android-x64.bare"
target="node_modules/@utexo/rgb-lightning-node-bare/prebuilds/android-x64/utexo__rgb-lightning-node-bare.bare"
url="https://github.com/UTEXO-Protocol/rgb-lightning-node-bare/releases/download/v${version}/${asset}"

mkdir -p "$(dirname "$target")"
curl --fail --location --output "${target}.tmp" "$url"
printf '%s  %s\n' "$expected_sha256" "${target}.tmp" | sha256sum --check --status
mv "${target}.tmp" "$target"
printf 'Installed %s (%s)\n' "$asset" "$version"
