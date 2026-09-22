#!/usr/bin/env bash
# Recreate the disposable local regtest chain and funded Alice/Bob/Carol nodes.
# Usage: scripts/regtest/recreate.sh --yes
set -euo pipefail
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

if [[ "${1:-}" != --yes || "$#" -ne 1 ]]; then
  echo 'This deletes .var/regtest, including demo wallets and payment journals.' >&2
  echo 'Usage: scripts/regtest/recreate.sh --yes' >&2
  exit 2
fi

if ! command -v lsof >/dev/null; then
  echo 'lsof is required to stop existing wallet services safely.' >&2
  exit 1
fi

# Never signal an unrelated process that happens to occupy a demo port.
declare -A seen=()
for port in 3030 3031 3032 3040 3050 3051 3052 3053; do
  while IFS= read -r pid; do
    [[ -n "$pid" && -z "${seen[$pid]:-}" ]] || continue
    executable="$(readlink -f "/proc/$pid/exe" 2>/dev/null || true)"
    case "$executable" in
      "$root/target/debug/api"|"$root/target/debug/recipient-service"|"$root/target/debug/l402-merchant")
        seen[$pid]=1 ;;
      *) echo "Port $port belongs to another process (PID $pid); stop it manually first." >&2; exit 1 ;;
    esac
  done < <(lsof -nP -tiTCP:"$port" -sTCP:LISTEN 2>/dev/null || true)
done
for pid in "${!seen[@]}"; do
  kill -TERM "$pid"
done
for attempt in {1..30}; do
  running=false
  for pid in "${!seen[@]}"; do
    if kill -0 "$pid" 2>/dev/null && [[ "$(ps -o stat= -p "$pid" 2>/dev/null)" != Z* ]]; then
      running=true
    fi
  done
  if [[ "$running" == false ]]; then break; fi
  sleep 1
done
if [[ "$running" == true ]]; then
  echo 'A wallet service did not exit cleanly; refusing to delete its state.' >&2
  exit 1
fi

domain=''
if [[ -f .var/regtest/recipient-domain ]]; then
  IFS= read -r domain < .var/regtest/recipient-domain
fi
settings_backup=''
if [[ -f .var/regtest/carol-orders.settings.json ]]; then
  settings_backup="$(mktemp "$root/.var/carol-settings-recreate.XXXXXXXX.json")"
  cp -- .var/regtest/carol-orders.settings.json "$settings_backup"
fi

./scripts/regtest/reset.sh --yes
./scripts/regtest/bootstrap.sh
python3 scripts/regtest/carol.py --apply

if [[ -n "$domain" ]]; then
  printf '%s\n' "$domain" > .var/regtest/recipient-domain
fi
if [[ -n "$settings_backup" ]]; then
  python3 - "$settings_backup" <<'PY'
import json
import os
from pathlib import Path
import sys

root = Path('.var/regtest')
settings = json.loads(Path(sys.argv[1]).read_text())
asset = json.loads((root / 'bootstrap.json').read_text())['asset_id']
settings['accepted_assets'] = [asset]
for product in settings.get('products', []):
    product['asset_id'] = asset
path = root / 'carol-orders.settings.json'
with path.open('x') as stream:
    json.dump(settings, stream, indent=2)
    stream.flush()
    os.fsync(stream.fileno())
PY
fi

./scripts/regtest/status.sh
echo 'Demo nodes recreated. Restart the local wallet APIs and recipient service with the new wallet.env.'
