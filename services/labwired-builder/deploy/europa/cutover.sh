#!/usr/bin/env bash
# CUTOVER — switch builder.labwired.com from the old systemd service to the
# containerized stack. Reversible: see rollback.sh. Run on Europa from this dir.
#
# Brief downtime window: between stopping the systemd builder and the container
# passing health (a few seconds). The host cloudflared and the public hostname
# are untouched — only the process behind 127.0.0.1:18080 changes.
set -euo pipefail
cd "$(dirname "$0")"

C="docker compose -f docker-compose.yml -f docker-compose.europa.yml"

echo "==> pre-flight: images present"
docker image inspect ghcr.io/w1ne/labwired-compile:dev >/dev/null
docker image inspect ghcr.io/w1ne/labwired-builder:dev >/dev/null

echo "==> stopping old systemd builder (frees 127.0.0.1:18080)"
sudo systemctl stop labwired-builder

echo "==> starting containerized stack (compile + compile-net + builder; host cloudflared reused)"
$C up -d compile compile-net builder

echo "==> waiting for health..."
for i in $(seq 1 20); do
  if curl -fsS --max-time 4 http://127.0.0.1:18080/healthz >/dev/null 2>&1; then break; fi
  sleep 2
done

echo "==> status"; $C ps
echo -n "==> local  healthz: "; curl -fsS --max-time 5 http://127.0.0.1:18080/healthz || echo "FAIL"; echo
echo -n "==> public healthz: "; curl -fsS --max-time 8 https://builder.labwired.com/healthz || echo "FAIL"; echo

echo
echo "If healthy: also disable the old unit so it won't fight on reboot:"
echo "    sudo systemctl disable labwired-builder"
echo "If broken: ./rollback.sh"
