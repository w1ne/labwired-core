#!/usr/bin/env bash
set -euo pipefail

# Usage:
#   ./verify_foundry.sh https://foundry.labwired.com

BASE_URL="${1:-https://foundry.labwired.com}"
HEALTH_URL="${BASE_URL%/}/v1/health"
CATALOG_URL="${BASE_URL%/}/v1/catalog"

echo "[1/4] Checking backend health: ${HEALTH_URL}"
health_code="$(curl -sS -o /tmp/foundry_health.json -w "%{http_code}" "${HEALTH_URL}")"
if [[ "${health_code}" != "200" ]]; then
  echo "Health endpoint failed with HTTP ${health_code}" >&2
  exit 1
fi
echo "Health OK"

echo "[2/4] Checking catalog endpoint: ${CATALOG_URL}"
catalog_code="$(curl -sS -o /tmp/foundry_catalog.json -w "%{http_code}" "${CATALOG_URL}")"
if [[ "${catalog_code}" != "200" ]]; then
  echo "Catalog endpoint failed with HTTP ${catalog_code}" >&2
  exit 1
fi
echo "Catalog OK"

echo "[3/4] Confirming healthy status payload..."
if ! grep -q '"status":"healthy"' /tmp/foundry_health.json; then
  echo "Health payload does not contain status=healthy" >&2
  cat /tmp/foundry_health.json >&2
  exit 1
fi

echo "[4/4] Confirming catalog payload is non-empty..."
if ! grep -q '"id":"' /tmp/foundry_catalog.json; then
  echo "Catalog payload appears empty or invalid" >&2
  cat /tmp/foundry_catalog.json >&2
  exit 1
fi

echo "Foundry verification passed."
