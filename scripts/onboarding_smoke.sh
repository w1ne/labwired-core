#!/usr/bin/env bash
set -euo pipefail

TARGET_ID=""
CRATE=""
CRATE_FEATURES=""
TARGET=""
SCRIPT=""
SYSTEM=""
OUT_DIR=""
PREBUILT_FIRMWARE=""
PROFILE="release"
THRESHOLD_SECONDS="${ONBOARDING_SOFT_THRESHOLD_SECONDS:-3600}"
INVENTORY_ONLY="false"
NOT_RUN_REASON=""

usage() {
  cat <<'EOF'
Usage: scripts/onboarding_smoke.sh --target-id ID --system PATH --out-dir DIR [options]

Smoke mode options:
  --crate NAME --target TRIPLE --script PATH
  --prebuilt-firmware PATH --script PATH
  --features NAME[,NAME...]
  --profile release|debug
  --threshold-seconds N

Inventory-only mode (records an explicit not_run result when no runnable
firmware/smoke mapping exists):
  --inventory-only [--not-run-reason TEXT]

Common options:
  --help, -h
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --help|-h)
      usage
      exit 0
      ;;
    --inventory-only)
      INVENTORY_ONLY="true"
      shift
      ;;
    --not-run-reason)
      NOT_RUN_REASON="$2"
      shift 2
      ;;
    --target-id)
      TARGET_ID="$2"
      shift 2
      ;;
    --crate)
      CRATE="$2"
      shift 2
      ;;
    --features)
      CRATE_FEATURES="$2"
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
    --prebuilt-firmware)
      PREBUILT_FIRMWARE="$2"
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

for required in TARGET_ID SYSTEM OUT_DIR; do
  if [[ -z "${!required}" ]]; then
    echo "Missing required argument: ${required}" >&2
    exit 2
  fi
done

if [[ "${INVENTORY_ONLY}" != "true" ]]; then
  if [[ -z "${SCRIPT}" ]]; then
    echo "Missing required argument: SCRIPT" >&2
    exit 2
  fi
  if [[ -z "${PREBUILT_FIRMWARE}" && ( -z "${CRATE}" || -z "${TARGET}" ) ]]; then
    echo "Smoke mode requires --crate and --target, or --prebuilt-firmware." >&2
    exit 2
  fi
  if [[ -n "${PREBUILT_FIRMWARE}" && ! -f "${PREBUILT_FIRMWARE}" ]]; then
    echo "Prebuilt firmware does not exist: ${PREBUILT_FIRMWARE}" >&2
    exit 2
  fi
  if [[ -n "${PREBUILT_FIRMWARE}" ]]; then
    python3 - "${SCRIPT}" "${PREBUILT_FIRMWARE}" <<'PY'
import sys
from pathlib import Path

script = Path(sys.argv[1])
firmware = Path(sys.argv[2]).resolve()
in_inputs = False
script_firmware = None
for line in script.read_text(encoding="utf-8").splitlines():
    if not line.strip() or line.lstrip().startswith("#"):
        continue
    if not line.startswith(" "):
        in_inputs = line.strip() == "inputs:"
        continue
    if in_inputs and line.strip().startswith("firmware:"):
        value = line.split(":", 1)[1].strip().strip("\"'")
        script_firmware = (script.parent / value).resolve()
        break
if script_firmware is None:
    raise SystemExit(f"Smoke script has no inputs.firmware: {script}")
if script_firmware != firmware:
    raise SystemExit(
        f"Prebuilt firmware {firmware} does not match smoke script input {script_firmware}"
    )
PY
  fi
fi

if [[ "${INVENTORY_ONLY}" == "true" && -z "${NOT_RUN_REASON}" ]]; then
  NOT_RUN_REASON="No platform-specific firmware crate and smoke script are mapped for this imported descriptor."
fi

mkdir -p "${OUT_DIR}/logs"

status="pass"
failure_stage=""
first_error_signature=""
failure_hint=""
run_attempted="false"

build_cli_ms=""
build_firmware_ms=""
run_smoke_ms=""

stage_start_ms=0
run_started_ms="$(date +%s%3N)"
run_started_iso="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

capture_signature() {
  local log_file="$1"
  local stage="${2:-}"
  if [[ "${stage}" == "run_smoke" ]]; then
    local result_file="${OUT_DIR}/simulation-output/result.json"
    if [[ -f "${result_file}" ]]; then
      local result_message
      result_message="$(python3 - "${result_file}" <<'PY'
import json
import sys
from pathlib import Path

try:
    result = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
except (OSError, json.JSONDecodeError):
    result = {}
message = result.get("message", "")
if message:
    print(str(message).replace("\r", " ").replace("\n", " "))
PY
)"
      if [[ -n "${result_message}" ]]; then
        printf '%s\n' "${result_message}" | cut -c1-240
        return
      fi
    fi
  fi
  awk '
    NF && first == "" { first = $0 }
    tolower($0) ~ /error|fail|assertion failed/ { print; found = 1; exit }
    END { if (!found && first != "") print first }
  ' "${log_file}" | tr -d '\r' | sed -E 's/\x1B\[[0-9;]*[mK]//g' | cut -c1-240
}

run_stage() {
  local stage="$1"
  shift
  local log_file="${OUT_DIR}/logs/${stage}.log"
  stage_start_ms="$(date +%s%3N)"

  if "$@" >"${log_file}" 2>&1; then
    local stage_end_ms
    stage_end_ms="$(date +%s%3N)"
    local elapsed=$((stage_end_ms - stage_start_ms))
    case "${stage}" in
      build_cli) build_cli_ms="${elapsed}" ;;
      build_firmware) build_firmware_ms="${elapsed}" ;;
      run_smoke) run_smoke_ms="${elapsed}" ;;
    esac
    return 0
  fi

  local stage_end_ms
  stage_end_ms="$(date +%s%3N)"
  local elapsed=$((stage_end_ms - stage_start_ms))
  case "${stage}" in
    build_cli) build_cli_ms="${elapsed}" ;;
    build_firmware) build_firmware_ms="${elapsed}" ;;
    run_smoke) run_smoke_ms="${elapsed}" ;;
  esac

  status="fail"
  failure_stage="${stage}"
  first_error_signature="$(capture_signature "${log_file}" "${stage}")"
  if [[ -z "${first_error_signature}" ]]; then
    first_error_signature="no-log-output"
  fi
  return 1
}

infer_failure_hint() {
  local stage="$1"
  local signature="$2"
  local sig_lc
  sig_lc="$(printf '%s' "${signature}" | tr '[:upper:]' '[:lower:]')"

  if [[ "${stage}" == "build_cli" ]]; then
    echo "Fix host toolchain first: run \`rustup show\` and ensure CLI dependencies build on this runner."
    return
  fi

  if [[ "${stage}" == "build_firmware" ]]; then
    if [[ "${sig_lc}" == *"can't find crate"* ]] || [[ "${sig_lc}" == *"cannot find crate"* ]]; then
      echo "Firmware dependency missing. Verify crate features and Cargo.toml dependencies for this target."
      return
    fi
    if [[ "${sig_lc}" == *"linker"* ]] || [[ "${sig_lc}" == *"linking with"* ]]; then
      echo "Linker/target mismatch. Check target triple, linker script, and memory layout for this board."
      return
    fi
    if [[ "${sig_lc}" == *"target"* ]] && [[ "${sig_lc}" == *"not installed"* ]]; then
      echo "Rust target missing. Add it in CI toolchain install step for this board target."
      return
    fi
    echo "Firmware build failed. Start with the first compiler error in logs/build_firmware.log."
    return
  fi

  if [[ "${stage}" == "run_smoke" ]]; then
    if [[ "${sig_lc}" == *"unsupported arm classic core"* ]]; then
      echo "CPU model gap. Add a real ARM7 adapter before claiming boot support."
      return
    fi
    if [[ "${sig_lc}" == *"unsupported arm real-time core"* ]]; then
      echo "CPU model gap. Add a real adapter for this core before claiming boot support."
      return
    fi
    if [[ "${sig_lc}" == *"no such file or directory"* ]] || [[ "${sig_lc}" == *"not found"* ]]; then
      echo "Path mismatch. Verify script/system paths in workflow matrix and repository layout."
      return
    fi
    if [[ "${sig_lc}" == *"unknown peripheral"* ]] || [[ "${sig_lc}" == *"unmapped"* ]] || [[ "${sig_lc}" == *"unsupported"* ]]; then
      echo "Model gap. Check unsupported access logs and implement/fix the first missing peripheral behavior."
      return
    fi
    if [[ "${sig_lc}" == *"assertion"* ]] || [[ "${sig_lc}" == *"expected"* ]]; then
      echo "Smoke assertion mismatch. Compare expected stop/assertions with current deterministic output."
      return
    fi
    echo "Simulation failed. Inspect logs/run_smoke.log and fix the first failing operation only."
    return
  fi

  echo "Unknown failure stage. Start from first non-empty line in stage log."
}

run_smoke_output_dir="${OUT_DIR}/simulation-output"
mkdir -p "${run_smoke_output_dir}"

firmware_build_args=(cargo build -p "${CRATE}")
if [[ -n "${CRATE_FEATURES}" ]]; then
  firmware_build_args+=(--features "${CRATE_FEATURES}")
fi

# No RUSTFLAGS here. A firmware crate passes its own linker script from its
# build.rs, so `cargo build -p <crate> --target <t>` links on its own.
#
# ⚠️ Exactly one place may pass `-Tlink.x`. This block used to add it for every
# thumbv6m target; when firmware-rp2040-pio-onboarding's build.rs started
# passing its own (labwired-core#1064), the script included link.x — and the
# memory.x it consumes — twice, and the build died with "region 'FLASH'
# already defined". This gate does not run on pull requests, so that break
# only appeared on main.

if [[ "${INVENTORY_ONLY}" == "true" ]]; then
  status="not_run"
  failure_stage="not_configured"
  first_error_signature="${NOT_RUN_REASON}"
  failure_hint="Add a compatible firmware build and smoke script before claiming a boot result."
elif ! run_stage build_cli cargo build -p labwired-cli; then
  :
elif [[ -n "${PREBUILT_FIRMWARE}" ]]; then
  run_attempted="true"
  if ! run_stage run_smoke cargo run -q -p labwired-cli -- test --script "${SCRIPT}" --output-dir "${run_smoke_output_dir}" --no-uart-stdout; then
    :
  fi
elif [[ "${PROFILE}" == "release" ]]; then
  if ! run_stage build_firmware "${firmware_build_args[@]}" --release --target "${TARGET}"; then
    :
  else
    run_attempted="true"
    if ! run_stage run_smoke cargo run -q -p labwired-cli -- test --script "${SCRIPT}" --output-dir "${run_smoke_output_dir}" --no-uart-stdout; then
      :
    fi
  fi
else
  if ! run_stage build_firmware "${firmware_build_args[@]}" --target "${TARGET}"; then
    :
  else
    run_attempted="true"
    if ! run_stage run_smoke cargo run -q -p labwired-cli -- test --script "${SCRIPT}" --output-dir "${run_smoke_output_dir}" --no-uart-stdout; then
      :
    fi
  fi
fi

run_finished_ms="$(date +%s%3N)"
run_finished_iso="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
elapsed_ms=$((run_finished_ms - run_started_ms))
elapsed_seconds=$(((elapsed_ms + 999) / 1000))
threshold_met=false
if [[ "${INVENTORY_ONLY}" == "true" ]]; then
  elapsed_ms=0
  elapsed_seconds=0
elif [[ "${elapsed_seconds}" -le "${THRESHOLD_SECONDS}" ]]; then
  threshold_met=true
fi

if [[ "${status}" == "fail" ]]; then
  failure_hint="$(infer_failure_hint "${failure_stage}" "${first_error_signature}")"
fi

export TARGET_ID CRATE TARGET SCRIPT SYSTEM OUT_DIR PREBUILT_FIRMWARE PROFILE INVENTORY_ONLY run_attempted
export run_started_iso run_finished_iso elapsed_seconds elapsed_ms
export status failure_stage first_error_signature threshold_met THRESHOLD_SECONDS
export build_cli_ms build_firmware_ms run_smoke_ms
export failure_hint

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

def parse_seconds(name: str):
    value = parse_int(name)
    return round(value / 1000.0, 3) if value is not None else None

run_attempted = os.environ.get("run_attempted", "false").lower() == "true"
result_path = out_dir / "simulation-output" / "result.json"
booted = None
boot_evidence = None
if run_attempted:
    try:
        result = json.loads(result_path.read_text(encoding="utf-8"))
        instructions = int(result.get("instructions", 0))
        booted = instructions > 0
        boot_evidence = {
            "result_path": str(result_path.relative_to(out_dir)),
            "instructions": instructions,
            "stop_reason": result.get("stop_reason"),
        }
    except (OSError, json.JSONDecodeError, TypeError, ValueError):
        booted = False

metrics = {
    "target_id": os.environ["TARGET_ID"],
    "status": status,
    "measurement_mode": "inventory_only" if os.environ.get("INVENTORY_ONLY", "false").lower() == "true" else "smoke",
    "run_attempted": run_attempted,
    "booted": booted,
    "boot_evidence": boot_evidence,
    "smoke_passed": True if status == "pass" and run_attempted else False if run_attempted else None,
    "crate": os.environ["CRATE"] or None,
    "target_triple": os.environ["TARGET"] or None,
    "script": os.environ["SCRIPT"],
    "system": os.environ["SYSTEM"],
    "profile": os.environ["PROFILE"],
    "started_at_utc": os.environ["run_started_iso"],
    "finished_at_utc": os.environ["run_finished_iso"],
    "elapsed_seconds": int(os.environ["elapsed_seconds"]),
    "elapsed_ms": int(os.environ["elapsed_ms"]),
    "failure_stage": os.environ.get("failure_stage", ""),
    "first_error_signature": os.environ.get("first_error_signature", ""),
    "failure_hint": os.environ.get("failure_hint", ""),
    "threshold_seconds": int(os.environ["THRESHOLD_SECONDS"]),
    "threshold_met": threshold_met,
    "stages_seconds": {
        "build_cli": parse_seconds("build_cli_ms"),
        "build_firmware": parse_seconds("build_firmware_ms"),
        "run_smoke": parse_seconds("run_smoke_ms"),
    },
    "stages_ms": {
        "build_cli": parse_int("build_cli_ms"),
        "build_firmware": parse_int("build_firmware_ms"),
        "run_smoke": parse_int("run_smoke_ms"),
    },
}
if os.environ.get("PREBUILT_FIRMWARE"):
    metrics["prebuilt_firmware"] = os.environ["PREBUILT_FIRMWARE"]

summary_lines = [
    f"# Onboarding Smoke: {metrics['target_id']}",
    "",
    f"- status: `{metrics['status']}`",
    f"- run_attempted: `{metrics['run_attempted']}`",
    f"- booted: `{metrics['booted']}`",
    f"- smoke_passed: `{metrics['smoke_passed']}`",
    f"- elapsed_seconds: `{metrics['elapsed_seconds']}`",
    f"- elapsed_ms: `{metrics['elapsed_ms']}`",
    f"- threshold_seconds: `{metrics['threshold_seconds']}`",
    f"- threshold_met: `{metrics['threshold_met']}`",
    f"- failure_stage: `{metrics['failure_stage'] or 'n/a'}`",
    f"- first_error_signature: `{metrics['first_error_signature'] or 'n/a'}`",
    f"- failure_hint: `{metrics['failure_hint'] or 'n/a'}`",
]

out_dir.mkdir(parents=True, exist_ok=True)
(out_dir / "onboarding-metrics.json").write_text(json.dumps(metrics, indent=2), encoding="utf-8")
summary_path = out_dir / "onboarding-summary.md"
if metrics["measurement_mode"] == "inventory_only":
    summary_path.unlink(missing_ok=True)
else:
    summary_path.write_text("\n".join(summary_lines) + "\n", encoding="utf-8")
PY

if [[ "${status}" == "fail" ]]; then
  echo "Onboarding smoke failed at stage: ${failure_stage}" >&2
  echo "Signature: ${first_error_signature}" >&2
  echo "Hint: ${failure_hint}" >&2
  exit 1
fi
