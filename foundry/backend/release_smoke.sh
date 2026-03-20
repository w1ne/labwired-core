#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT_DIR"

if [[ -x "${HOME}/opt/go1.24.0-bin/bin/go" ]]; then
  GO_BIN="${HOME}/opt/go1.24.0-bin/bin/go"
else
  GO_BIN="$(command -v go)"
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required for release_smoke.sh" >&2
  exit 1
fi

WORK_DIR="$(mktemp -d /tmp/foundry-release-smoke.XXXXXX)"
PORT="${PORT:-18081}"
DB_PATH="${WORK_DIR}/foundry_release_smoke.db"
ARTIFACTS_DIR="${WORK_DIR}/artifacts"
DATA_DIR="${WORK_DIR}/data"
LOG_PATH="${WORK_DIR}/server.log"
HEALTH_URL="http://127.0.0.1:${PORT}/v1/health"
BASE_URL="http://127.0.0.1:${PORT}/v1"
SERVER_PID=""

cleanup() {
  if [[ -n "${SERVER_PID}" ]] && kill -0 "${SERVER_PID}" >/dev/null 2>&1; then
    kill "${SERVER_PID}" >/dev/null 2>&1 || true
    wait "${SERVER_PID}" >/dev/null 2>&1 || true
  fi
  rm -rf "${WORK_DIR}"
}
trap cleanup EXIT

mkdir -p "${ARTIFACTS_DIR}" "${DATA_DIR}"

echo "[release-smoke] Starting backend on port ${PORT}"
APP_ENV=development \
PORT="${PORT}" \
LABWIRED_PATH="${LABWIRED_PATH:-sh}" \
DB_PATH="${DB_PATH}" \
ARTIFACTS_DIR="${ARTIFACTS_DIR}" \
DATA_DIR="${DATA_DIR}" \
HARDWARE_JSON_PATH="${ROOT_DIR}/configs/hardware.json" \
CORE_CONFIGS_DIR="$(cd "${ROOT_DIR}/../../core/configs" && pwd)" \
"${GO_BIN}" run ./cmd/server >"${LOG_PATH}" 2>&1 &
SERVER_PID=$!

for _ in $(seq 1 60); do
  if curl -fsS "${HEALTH_URL}" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done

if ! curl -fsS "${HEALTH_URL}" >/tmp/foundry_release_health.json; then
  echo "[release-smoke] backend failed to become healthy" >&2
  cat "${LOG_PATH}" >&2
  exit 1
fi

echo "[release-smoke] Generating API key"
KEY_OUTPUT="$("${GO_BIN}" run ./cmd/addkey -workspace release-smoke -db "${DB_PATH}")"
API_KEY="$(echo "${KEY_OUTPUT}" | awk -F': ' '/Your API Key/{print $2}' | tr -d '[:space:]')"
if [[ -z "${API_KEY}" ]]; then
  echo "[release-smoke] failed to parse generated API key" >&2
  echo "${KEY_OUTPUT}" >&2
  exit 1
fi

echo "[release-smoke] Verifying public endpoints"
curl -fsS "${BASE_URL}/info" >/tmp/foundry_release_info.json
curl -fsS "${BASE_URL}/catalog" >/tmp/foundry_release_catalog.json

python3 - <<'PY'
import json
from pathlib import Path

health = json.loads(Path("/tmp/foundry_release_health.json").read_text())
catalog = json.loads(Path("/tmp/foundry_release_catalog.json").read_text())

if health.get("status") != "healthy":
    raise SystemExit(f"unexpected health payload: {health}")
if not isinstance(catalog, list) or not catalog:
    raise SystemExit("catalog payload is empty")
PY

echo "[release-smoke] Verifying auth boundary"
unauth_code="$(curl -sS -o /dev/null -w "%{http_code}" "${BASE_URL}/usage")"
if [[ "${unauth_code}" != "401" ]]; then
  echo "expected /v1/usage without auth to return 401, got ${unauth_code}" >&2
  exit 1
fi

curl -fsS -H "Authorization: Bearer ${API_KEY}" "${BASE_URL}/usage" >/tmp/foundry_release_usage_before.json

python3 - <<'PY'
import json
from pathlib import Path

usage = json.loads(Path("/tmp/foundry_release_usage_before.json").read_text())
if usage.get("runs_used_this_month") != 0:
    raise SystemExit(f"expected zero usage before verify, got {usage}")
if usage.get("quota") != 1000:
    raise SystemExit(f"expected builder quota 1000, got {usage}")
PY

echo "[release-smoke] Verifying authenticated model run"
curl -fsS -X POST \
  -H "Authorization: Bearer ${API_KEY}" \
  -H "Content-Type: application/json" \
  -d '{"chip_yaml":"registers: []"}' \
  "${BASE_URL}/models/verify" >/tmp/foundry_release_verify.json

python3 - <<'PY'
import json
from pathlib import Path

resp = json.loads(Path("/tmp/foundry_release_verify.json").read_text())
run_id = resp.get("run_id", "")
if not run_id.startswith("run-model-"):
    raise SystemExit(f"unexpected verify response: {resp}")
PY

curl -fsS -H "Authorization: Bearer ${API_KEY}" "${BASE_URL}/usage" >/tmp/foundry_release_usage_after.json

python3 - <<'PY'
import json
from pathlib import Path

usage = json.loads(Path("/tmp/foundry_release_usage_after.json").read_text())
if usage.get("runs_used_this_month") != 1:
    raise SystemExit(f"expected usage to increment after verify, got {usage}")
PY

echo "[release-smoke] Passed"
