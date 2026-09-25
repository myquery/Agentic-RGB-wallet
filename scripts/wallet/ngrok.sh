#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
ngrok_config="$HOME/.ngrok2/ngrok.yml"
if [[ ! -f "$ngrok_config" ]]; then
  ngrok_config="$HOME/.config/ngrok/ngrok.yml"
fi
if [[ ! -f "$ngrok_config" ]]; then
  echo 'Ngrok startup failed: no ngrok configuration file found' >&2
  exit 1
fi
ngrok_bin="$(command -v ngrok || true)"
if [[ -z "$ngrok_bin" && -x /snap/bin/ngrok ]]; then
  ngrok_bin=/snap/bin/ngrok
fi
if [[ -z "$ngrok_bin" ]]; then
  echo 'Ngrok startup failed: ngrok executable not found' >&2
  exit 1
fi
mkdir -p .var/regtest/logs
ngrok_args=(http 127.0.0.1:3050 --config="$ngrok_config" --inspect=false --log=.var/regtest/logs/ngrok.log)
if [[ -f .var/regtest/ngrok-domain ]]; then
  IFS= read -r reserved_domain < .var/regtest/ngrok-domain
  if [[ -n "$reserved_domain" ]]; then
    ngrok_args+=(--url="https://$reserved_domain")
  fi
fi
"$ngrok_bin" "${ngrok_args[@]}" &
ngrok_pid=$!
cleanup() {
  kill "$ngrok_pid" 2>/dev/null || true
  wait "$ngrok_pid" 2>/dev/null || true
}
trap cleanup EXIT INT TERM
active_domain="$(python3 - <<'PY'
import json
import sys
import time
import urllib.parse
import urllib.request
for _ in range(30):
    try:
        with urllib.request.urlopen('http://127.0.0.1:4040/api/tunnels', timeout=2) as response:
            tunnels = json.load(response).get('tunnels', [])
        urls = [item.get('public_url', '') for item in tunnels]
        urls = [url for url in urls if url.startswith('https://')]
        if urls:
            host = urllib.parse.urlparse(urls[0]).hostname
            if host:
                print(host)
                raise SystemExit(0)
    except Exception:
        pass
    time.sleep(1)
raise SystemExit('Ngrok startup failed: active HTTPS endpoint was not reported')
PY
)"
previous_domain=''
if [[ -f .var/regtest/recipient-domain ]]; then
  IFS= read -r previous_domain < .var/regtest/recipient-domain
fi
if [[ "$active_domain" != "$previous_domain" ]]; then
  active_generation="$(printf '%s' "$active_domain" | sha256sum | cut -c1-12)"
  previous_generation=''
  if [[ -n "$previous_domain" ]]; then
    previous_generation="$(printf '%s' "$previous_domain" | sha256sum | cut -c1-12)"
  fi
  for merchant_prefix in alice-merchant-orders bob-merchant-orders carol-orders; do
    settings_source=".var/regtest/${merchant_prefix}.settings.json"
    if [[ -n "$previous_generation" && -f ".var/regtest/${merchant_prefix}-${previous_generation}.settings.json" ]]; then
      settings_source=".var/regtest/${merchant_prefix}-${previous_generation}.settings.json"
    fi
    settings_target=".var/regtest/${merchant_prefix}-${active_generation}.settings.json"
    if [[ -f "$settings_source" && ! -e "$settings_target" ]]; then
      cp "$settings_source" "$settings_target"
      chmod 600 "$settings_target"
    fi
  done
  temporary_domain=".var/regtest/recipient-domain.tmp.$$"
  printf '%s\n' "$active_domain" > "$temporary_domain"
  mv "$temporary_domain" .var/regtest/recipient-domain
  systemctl --user restart rgb402-recipient.service \
    rgb402-wallet@alice.service rgb402-wallet@bob.service rgb402-wallet@carol.service
fi
trap - EXIT
wait "$ngrok_pid"
