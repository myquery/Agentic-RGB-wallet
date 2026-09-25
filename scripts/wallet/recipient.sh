#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
source .var/regtest/wallet.env
if [[ ! -f .var/regtest/recipient-domain ]]; then
  echo 'Recipient service startup failed: .var/regtest/recipient-domain is missing' >&2
  exit 1
fi
IFS= read -r recipient_domain < .var/regtest/recipient-domain
if [[ -z "$recipient_domain" ]]; then
  echo 'Recipient service startup failed: recipient domain is empty' >&2
  exit 1
fi
export RECIPIENT_DOMAIN="$recipient_domain"
export RECIPIENT_ASSET_ID="$ALLOWED_ASSET_IDS"
export CAROL_WALLET_ENABLED=true
exec cargo run -p merchant-server --bin recipient-service
