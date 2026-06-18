#!/usr/bin/env bash
# ROLLBACK — restore the old systemd builder and stop the containerized stack.
# Run on Europa from this dir.
set -euo pipefail
cd "$(dirname "$0")"

C="docker compose -f docker-compose.yml -f docker-compose.europa.yml"

echo "==> stopping containerized stack"
$C down || true

echo "==> restarting old systemd builder"
sudo systemctl start labwired-builder

echo "==> waiting for health..."
for i in $(seq 1 15); do
  if curl -fsS --max-time 4 http://127.0.0.1:18080/healthz >/dev/null 2>&1; then break; fi
  sleep 2
done
echo -n "==> local  healthz: "; curl -fsS --max-time 5 http://127.0.0.1:18080/healthz || echo "FAIL"; echo
echo -n "==> public healthz: "; curl -fsS --max-time 8 https://builder.labwired.com/healthz || echo "FAIL"; echo
echo "Rolled back to systemd builder."
