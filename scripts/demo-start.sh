#!/usr/bin/env bash
# Safe launcher for an existing Luma regtest demo.
# Usage: scripts/demo-start.sh [alice|bob|carol|all|check]
#
# This script never resets, bootstraps, provisions Carol, deletes state, removes
# containers, or clears locks. It prints the required wallet commands for `all`.
set -euo pipefail

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
role="${1:-alice}"

usage() {
  echo 'Usage: scripts/demo-start.sh [alice|bob|carol|all|check]' >&2
  exit 2
}

case "$role" in
  alice|bob|carol) ;;
  all)
    cat <<'EOF'
Start each wallet in a separate terminal after the infrastructure, recipient
service, and ngrok steps in docs/demo-start.md are healthy:

  ./scripts/wallet/api.sh alice  # http://127.0.0.1:3030
  ./scripts/wallet/api.sh bob    # http://127.0.0.1:3031
  ./scripts/wallet/api.sh carol  # http://127.0.0.1:3032
EOF
    exit 0
    ;;
  check)
    exec python3 "$root/scripts/demo/check.py"
    ;;
  *) usage ;;
esac

if [[ ! -f "$root/.var/regtest/wallet.env" ]]; then
  echo 'Missing .var/regtest/wallet.env. Provision the disposable environment explicitly before launching a wallet.' >&2
  exit 1
fi

exec "$root/scripts/wallet/api.sh" "$role"
