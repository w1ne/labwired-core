#!/usr/bin/env bash
set -euo pipefail

TARGET_ID=""
CRATE=""
TARGET=""
SCRIPT=""
SYSTEM=""
OUT_DIR=""
PROFILE="release"
THRESHOLD_SECONDS="${ONBOARDING_SOFT_THRESHOLD_SECONDS:-3600}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target-id)
      TARGET_ID="$2"
      shift 2
      ;;
    --crate)
      CRATE="$2"
      shift 2
      ;;
    --target)
      TARGET="$2"
      shift 2
      ;;
    --script)
      SCRIPT="$2"
      shift 2
      ;;
    --system)
      SYSTEM="$2"
      shift 2
      ;;
    --out-dir)
      OUT_DIR="$2"
      shift 2
      ;;
    --profile)
      PROFILE="$2"
      shift 2
      ;;
    --threshold-seconds)
      THRESHOLD_SECONDS="$2"
      shift 2
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

for required in TARGET_ID CRATE TARGET SCRIPT SYSTEM OUT_DIR; do
  if [[ -z "${!required}" ]]; then
    echo "Missing required argument: ${required}" >&2
    exit 2
  fi
done

mkdir -p "${OUT_DIR}/logs"

status="pass"
failure_stage=""
first_error_signature=""

build_cli_ms=""
build_firmware_ms=""
run_smoke_ms=""

stage_start_epoch=0
run_started_epoch="$(date +%s)"
run_started_iso="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

capture_signature() {
  local log_file="$1"
  awk 'NF {print; exit}' "${log_file}" | tr -d '\r' | sed -E 's/\x1B\[[0-9;]*[mK]//g' | cut -c1-240
}

run_stage() {
  local stage="$1"
  shift
  local log_file="${OUT_DIR}/logs/${stage}.log"
  stage_start_epoch="$(date +%s)"

  if "$@" >"${log_file}" 2>&1; then
    local stage_end_epoch
    stage_end_epoch="$(date +%s)"
    local elapsed=$((stage_end_epoch - stage_start_epoch))
    case "${stage}" in
      build_cli) build_cli_ms="${elapsed}" ;;
      build_firmware) build_firmware_ms="${elapsed}" ;;
      run_smoke) run_smoke_ms="${elapsed}" ;;
    esac
    return 0
  fi

  local stage_end_epoch
  stage_end_epoch="$(date +%s)"
  local elapsed=$((stage_end_epoch - stage_start_epoch))
  case "${stage}" in
    build_cli) build_cli_ms="${elapsed}" ;;
    build_firmware) build_firmware_ms="${elapsed}" ;;
    run_smoke) run_smoke_ms="${elapsed}" ;;
  esac

  status="fail"
  failure_stage="${stage}"
  first_error_signature="$(capture_signature "${log_file}")"
  if [[ -z "${first_error_signature}" ]]; then
    first_error_signature="no-log-output"
  fi
  return 1
}

run_smoke_output_dir="${OUT_DIR}/simulation-output"
mkdir -p "${run_smoke_output_dir}"

if ! run_stage build_cli cargo build -p labwired-cli; then
  :
elif [[ "${PROFILE}" == "release" ]]; then
  if ! run_stage build_firmware cargo build -p "${CRATE}" --release --target "${TARGET}"; then
    :
  elif ! run_stage run_smoke cargo run -q -p labwired-cli -- test --script "${SCRIPT}" --output-dir "${run_smoke_output_dir}" --no-uart-stdout; then
    :
  fi
else
  if ! run_stage build_firmware cargo build -p "${CRATE}" --target "${TARGET}"; then
    :
  elif ! run_stage run_smoke cargo run -q -p labwired-cli -- test --script "${SCRIPT}" --output-dir "${run_smoke_output_dir}" --no-uart-stdout; then
    :
  fi
fi

run_finished_epoch="$(date +%s)"
run_finished_iso="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
elapsed_seconds=$((run_finished_epoch - run_started_epoch))
threshold_met=false
if [[ "${elapsed_seconds}" -le "${THRESHOLD_SECONDS}" ]]; then
  threshold_met=true
fi

export TARGET_ID CRATE TARGET SCRIPT SYSTEM OUT_DIR PROFILE
export run_started_iso run_finished_iso elapsed_seconds
export status failure_stage first_error_signature threshold_met THRESHOLD_SECONDS
export build_cli_ms build_firmware_ms run_smoke_ms

python3 - <<'PY'
import json
import os
from pathlib import Path

out_dir = Path(os.environ["OUT_DIR"])
status = os.environ["status"]
threshold_met = os.environ["threshold_met"].lower() == "true"

def parse_int(name: str):
    value = os.environ.get(name, "")
    return int(value) if value.strip() else None

metrics = {
    "target_id": os.environ["TARGET_ID"],
    "status": status,
    "crate": os.environ["CRATE"],
    "target_triple": os.environ["TARGET"],
    "script": os.environ["SCRIPT"],
    "system": os.environ["SYSTEM"],
    "profile": os.environ["PROFILE"],
    "started_at_utc": os.environ["run_started_iso"],
    "finished_at_utc": os.environ["run_finished_iso"],
    "elapsed_seconds": int(os.environ["elapsed_seconds"]),
    "failure_stage": os.environ.get("failure_stage", ""),
    "first_error_signature": os.environ.get("first_error_signature", ""),
    "threshold_seconds": int(os.environ["THRESHOLD_SECONDS"]),
    "threshold_met": threshold_met,
    "stages_seconds": {
        "build_cli": parse_int("build_cli_ms"),
        "build_firmware": parse_int("build_firmware_ms"),
        "run_smoke": parse_int("run_smoke_ms"),
    },
}

summary_lines = [
    f"# Onboarding Smoke: {metrics['target_id']}",
    "",
    f"- status: `{metrics['status']}`",
    f"- elapsed_seconds: `{metrics['elapsed_seconds']}`",
    f"- threshold_seconds: `{metrics['threshold_seconds']}`",
    f"- threshold_met: `{metrics['threshold_met']}`",
    f"- failure_stage: `{metrics['failure_stage'] or 'n/a'}`",
    f"- first_error_signature: `{metrics['first_error_signature'] or 'n/a'}`",
]

out_dir.mkdir(parents=True, exist_ok=True)
(out_dir / "onboarding-metrics.json").write_text(json.dumps(metrics, indent=2), encoding="utf-8")
(out_dir / "onboarding-summary.md").write_text("\n".join(summary_lines) + "\n", encoding="utf-8")
PY

if [[ "${status}" != "pass" ]]; then
  echo "Onboarding smoke failed at stage: ${failure_stage}" >&2
  echo "Signature: ${first_error_signature}" >&2
  exit 1
fi
