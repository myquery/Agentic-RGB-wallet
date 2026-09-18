#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
role="${1:-}"
case "$role" in
  alice|bob|carol) ;;
  *) echo 'Usage: scripts/wallet/api.sh alice|bob|carol' >&2; exit 2 ;;
esac
source .var/regtest/wallet.env
if [[ "$role" == alice ]]; then
  export MERCHANT_STATE_PATH="$PWD/.var/regtest/alice-merchant-orders.jsonl"
  export MERCHANT_BIND=127.0.0.1:3053 MERCHANT_ENABLED=false
  export WALLET_API_BIND=127.0.0.1:3030 WALLET_NAME='Alice Wallet'
  export RGB_NODE_URL=http://127.0.0.1:3101
  export WALLET_STATE_PATH="$PWD/.var/regtest/wallet-state.json"
  export MACHINE_STATE_PATH="$PWD/.var/regtest/machine-state.jsonl"
elif [[ "$role" == bob ]]; then
  export MERCHANT_STATE_PATH="$PWD/.var/regtest/bob-merchant-orders.jsonl"
  export MERCHANT_BIND=127.0.0.1:3052 MERCHANT_ENABLED=false
  export WALLET_API_BIND=127.0.0.1:3031 WALLET_NAME='Bob Wallet'
  export RGB_NODE_URL=http://127.0.0.1:3102
  export WALLET_STATE_PATH="$PWD/.var/regtest/bob-wallet-state.json"
  export MACHINE_STATE_PATH="$PWD/.var/regtest/bob-machine-state.jsonl"
else
  export WALLET_API_BIND=127.0.0.1:3032 WALLET_NAME='Carol Wallet'
  export RGB_NODE_URL=http://127.0.0.1:3103
  export WALLET_STATE_PATH="$PWD/.var/regtest/carol-wallet-state.json"
  export MACHINE_STATE_PATH="$PWD/.var/regtest/carol-machine-state.jsonl"
  export MERCHANT_ENABLED=true MERCHANT_BIND=127.0.0.1:3051
  export MERCHANT_STATE_PATH="$PWD/.var/regtest/carol-orders.jsonl"
fi
# Public identity only; no credentials are stored in this file.
if [[ -z "${RECIPIENT_DOMAIN:-}" && -f .var/regtest/recipient-domain ]]; then
  IFS= read -r RECIPIENT_DOMAIN < .var/regtest/recipient-domain
fi
if [[ -n "${RECIPIENT_DOMAIN:-}" ]]; then
  export WALLET_RECIPIENT_ADDRESS="$role@$RECIPIENT_DOMAIN"
else
  unset MERCHANT_BIND
fi
exec cargo run -p buyer-agent --bin api
