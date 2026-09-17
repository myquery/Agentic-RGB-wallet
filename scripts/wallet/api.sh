#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
role="${1:-}"
case "$role" in
  alice|bob) ;;
  *) echo 'Usage: scripts/wallet/api.sh alice|bob' >&2; exit 2 ;;
esac
source .var/regtest/wallet.env
if [[ "$role" == alice ]]; then
  export WALLET_API_BIND=127.0.0.1:3030 WALLET_NAME='Alice Wallet'
  export RGB_NODE_URL=http://127.0.0.1:3101
  export WALLET_STATE_PATH="$PWD/.var/regtest/wallet-state.json"
  export MACHINE_STATE_PATH="$PWD/.var/regtest/machine-state.jsonl"
else
  export WALLET_API_BIND=127.0.0.1:3031 WALLET_NAME='Bob Wallet'
  export RGB_NODE_URL=http://127.0.0.1:3102
  export WALLET_STATE_PATH="$PWD/.var/regtest/bob-wallet-state.json"
  export MACHINE_STATE_PATH="$PWD/.var/regtest/bob-machine-state.jsonl"
fi
exec cargo run -p buyer-agent --bin api
