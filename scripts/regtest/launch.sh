#!/usr/bin/env bash
# Full demo launcher: build → recreate regtest → start all services.
# Usage: scripts/launch.sh [--fresh]
#   --fresh  reset and reprovision the disposable regtest state (required on first run)
#   (no arg) skip reset, just start services against existing state
set -euo pipefail
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

FRESH=false
if [[ "${1:-}" == --fresh ]]; then
  FRESH=true
fi

log() { echo "==> $*"; }

wait_http() {
  local url="$1" label="$2"
  for i in {1..30}; do
    if curl -sf --max-time 2 "$url" >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
  echo "ERROR: $label did not become ready at $url" >&2
  return 1
}

stop_on_port() {
  local port="$1"
  local pid
  pid="$(lsof -nP -tiTCP:"$port" -sTCP:LISTEN 2>/dev/null || true)"
  if [[ -n "$pid" ]]; then
    local exe
    exe="$(readlink -f "/proc/$pid/exe" 2>/dev/null || true)"
    case "$exe" in
      "$root/target/debug/api"|"$root/target/debug/recipient-service"|"$root/target/debug/l402-merchant")
        kill -TERM "$pid" 2>/dev/null || true
        for i in {1..10}; do kill -0 "$pid" 2>/dev/null || break; sleep 1; done
        ;;
      *)
        echo "Port $port is held by an unrelated process (PID $pid $exe); stop it manually." >&2
        exit 1
        ;;
    esac
  fi
}

# ── 1. Stop existing Luma services ────────────────────────────────────────
log "Stopping any running Luma services..."
for port in 3030 3031 3032 3050 3051 3052 3053; do
  stop_on_port "$port"
done
rm -f "$root"/.var/regtest/*.lock

# ── 2. Build binaries ─────────────────────────────────────────────────────
log "Building binaries..."
cargo build -p buyer-agent --bin api \
            -p merchant-server --bin recipient-service \
            2>&1 | tail -5

# ── 3. Provision or reuse regtest state ───────────────────────────────────
if [[ "$FRESH" == true ]]; then
  log "Provisioning fresh regtest (reset + bootstrap + carol)..."
  "$root/scripts/regtest/recreate.sh" --yes
else
  if [[ ! -f .var/regtest/wallet.env ]]; then
    echo "ERROR: No existing state found. Run: scripts/launch.sh --fresh" >&2
    exit 1
  fi
  log "Reusing existing regtest state..."
  "$root/scripts/regtest/bootstrap.sh"
fi

# ── 4. Load environment ───────────────────────────────────────────────────
source .var/regtest/wallet.env
ASSET_ID="$ALLOWED_ASSET_IDS"

DOMAIN=''
if [[ -n "${RECIPIENT_DOMAIN:-}" ]]; then
  DOMAIN="$RECIPIENT_DOMAIN"
elif [[ -f .var/regtest/recipient-domain ]]; then
  IFS= read -r DOMAIN < .var/regtest/recipient-domain
fi

# ── 5. Start recipient service ────────────────────────────────────────────
if [[ -n "$DOMAIN" ]]; then
  log "Starting recipient service (port 3050)..."
  RECIPIENT_DOMAIN="$DOMAIN" \
  RECIPIENT_ASSET_ID="$ASSET_ID" \
  CAROL_WALLET_ENABLED=true \
  "$root/target/debug/recipient-service" >> "$root/.var/regtest/logs/recipient-service.log" 2>&1 &
  for i in {1..10}; do
    code=$(curl -s -o /dev/null -w '%{http_code}' --max-time 2 'http://127.0.0.1:3050/.well-known/webfinger' 2>/dev/null || true)
    if [[ "$code" =~ ^[234] ]]; then break; fi
    sleep 1
  done
  if ! [[ "$code" =~ ^[234] ]]; then
    echo "ERROR: recipient service did not become ready" >&2
    cat "$root/.var/regtest/logs/recipient-service.log" >&2
    exit 1
  fi
  log "Recipient service ready."
else
  log "WARN: No RECIPIENT_DOMAIN — skipping recipient service."
  log "      Set it: echo 'your.ngrok-free.dev' > .var/regtest/recipient-domain"
fi

# ── 6. Start wallet APIs ──────────────────────────────────────────────────
log "Starting Alice wallet (port 3030)..."
env -i HOME="$HOME" PATH="$PATH" \
  RGB_NODE_URL=http://127.0.0.1:3101 RGB_NODE_TOKEN='' \
  ALLOWED_ASSET_IDS="$ASSET_ID" \
  AUTO_APPROVE_BELOW="$AUTO_APPROVE_BELOW" MAX_SINGLE_PAYMENT="$MAX_SINGLE_PAYMENT" \
  MAX_DAILY_SPEND="$MAX_DAILY_SPEND" MAX_CARRIER_MSAT="$MAX_CARRIER_MSAT" \
  WALLET_API_BIND=127.0.0.1:3030 WALLET_NAME='Alice Wallet' \
  WALLET_STATE_PATH="$root/.var/regtest/wallet-state.json" \
  MACHINE_STATE_PATH="$root/.var/regtest/alice-machine-state.jsonl" \
  MERCHANT_STATE_PATH="$root/.var/regtest/alice-merchant-orders.jsonl" \
  MERCHANT_BIND=127.0.0.1:3053 MERCHANT_ENABLED=false \
  ${DOMAIN:+WALLET_RECIPIENT_ADDRESS="alice@$DOMAIN"} \
  "$root/target/debug/api" >> "$root/.var/regtest/logs/alice-wallet.log" 2>&1 &

log "Starting Bob wallet (port 3031)..."
env -i HOME="$HOME" PATH="$PATH" \
  RGB_NODE_URL=http://127.0.0.1:3102 RGB_NODE_TOKEN='' \
  ALLOWED_ASSET_IDS="$ASSET_ID" \
  AUTO_APPROVE_BELOW="$AUTO_APPROVE_BELOW" MAX_SINGLE_PAYMENT="$MAX_SINGLE_PAYMENT" \
  MAX_DAILY_SPEND="$MAX_DAILY_SPEND" MAX_CARRIER_MSAT="$MAX_CARRIER_MSAT" \
  WALLET_API_BIND=127.0.0.1:3031 WALLET_NAME='Bob Wallet' \
  WALLET_STATE_PATH="$root/.var/regtest/bob-wallet-state.json" \
  MACHINE_STATE_PATH="$root/.var/regtest/bob-machine-state.jsonl" \
  MERCHANT_STATE_PATH="$root/.var/regtest/bob-merchant-orders.jsonl" \
  MERCHANT_BIND=127.0.0.1:3052 MERCHANT_ENABLED=false \
  ${DOMAIN:+WALLET_RECIPIENT_ADDRESS="bob@$DOMAIN"} \
  "$root/target/debug/api" >> "$root/.var/regtest/logs/bob-wallet.log" 2>&1 &

log "Starting Carol wallet (port 3032)..."
env -i HOME="$HOME" PATH="$PATH" \
  RGB_NODE_URL=http://127.0.0.1:3103 RGB_NODE_TOKEN='' \
  ALLOWED_ASSET_IDS="$ASSET_ID" \
  AUTO_APPROVE_BELOW="$AUTO_APPROVE_BELOW" MAX_SINGLE_PAYMENT="$MAX_SINGLE_PAYMENT" \
  MAX_DAILY_SPEND="$MAX_DAILY_SPEND" MAX_CARRIER_MSAT="$MAX_CARRIER_MSAT" \
  WALLET_API_BIND=127.0.0.1:3032 WALLET_NAME='Carol Wallet' \
  WALLET_STATE_PATH="$root/.var/regtest/carol-wallet-state.json" \
  MACHINE_STATE_PATH="$root/.var/regtest/carol-machine-state.jsonl" \
  MERCHANT_STATE_PATH="$root/.var/regtest/carol-orders.jsonl" \
  MERCHANT_BIND=127.0.0.1:3051 MERCHANT_ENABLED=true \
  ${DOMAIN:+WALLET_RECIPIENT_ADDRESS="carol@$DOMAIN"} \
  "$root/target/debug/api" >> "$root/.var/regtest/logs/carol-wallet.log" 2>&1 &

# ── 7. Wait for wallet APIs ───────────────────────────────────────────────
wait_http http://127.0.0.1:3030/api/wallet "alice wallet"
wait_http http://127.0.0.1:3031/api/wallet "bob wallet"
wait_http http://127.0.0.1:3032/api/wallet "carol wallet"

# ── 8. Summary ────────────────────────────────────────────────────────────
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Luma demo is live"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Alice  http://127.0.0.1:3030"
echo "  Bob    http://127.0.0.1:3031"
echo "  Carol  http://127.0.0.1:3032"
if [[ -n "$DOMAIN" ]]; then
echo ""
echo "  WebFinger (start ngrok → port 3050 if not running):"
echo "  alice@$DOMAIN"
echo "  bob@$DOMAIN"
echo ""
echo "  ngrok http 127.0.0.1:3050 --url=$DOMAIN --inspect=false"
fi
echo ""
echo "  Logs: .var/regtest/logs/"
echo "  Stop: kill \$(lsof -nP -tiTCP:3030,3031,3032,3050 -sTCP:LISTEN 2>/dev/null)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
