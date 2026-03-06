#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
DEPLOY_DIR="$ROOT_DIR/foundry/deploy"
FRONTEND_DIR="$ROOT_DIR/foundry/frontend"
COMPOSE_FILE="$DEPLOY_DIR/docker-compose.smoke.yml"
BASE_URL="${PLAYWRIGHT_BASE_URL:-http://127.0.0.1:8088}"

if docker compose version >/dev/null 2>&1; then
  DOCKER_COMPOSE=(docker compose)
elif command -v docker-compose >/dev/null 2>&1; then
  DOCKER_COMPOSE=(docker-compose)
else
  echo "docker compose / docker-compose is required"
  exit 1
fi

cleanup() {
  "${DOCKER_COMPOSE[@]}" -f "$COMPOSE_FILE" down -v >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "[smoke] Starting Docker stack from $COMPOSE_FILE"
"${DOCKER_COMPOSE[@]}" -f "$COMPOSE_FILE" up --build -d

echo "[smoke] Waiting for backend health endpoint..."
for _ in $(seq 1 60); do
  if curl -fsS "$BASE_URL/v1/health" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
curl -fsS "$BASE_URL/v1/health" >/dev/null

echo "[smoke] Installing Playwright browser runtime (chromium)..."
cd "$FRONTEND_DIR"
npx playwright install chromium

echo "[smoke] Running frontend e2e smoke against $BASE_URL"
PLAYWRIGHT_BASE_URL="$BASE_URL" npm run test:e2e

echo "[smoke] Smoke checks passed."
