#!/usr/bin/env bash
# Build the STM32F103 I2C sensor-lab demo firmware FROM SOURCE and run each
# lab's committed smoke scripts against the simulator under test.
#
# Why this exists: labwired-core#1346 made STM32F1 I2C master-receive require
# clearing ADDR, as silicon does. Seven demo firmwares never cleared it and read
# garbage; core CI was green because nothing here built those labs and ran them
# against the simulator in the PR. The breakage surfaced weeks later, in the
# superproject's playground deploy. This script is the missing lane.
#
# Usage: scripts/ci/lab-smoke-from-source.sh [labwired-cli]
#   LABWIRED_CLI    path to the CLI (or first argument); defaults to
#                   target/{debug,release}/labwired
#   LAB_SMOKE_LABS  space-separated override of the lab list (for bisecting)
#
# Every failure is collected and reported; the exit code is non-zero if any lab
# failed to build or any smoke script failed. Nothing is skipped silently.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

TARGET=thumbv7m-none-eabi
# The F103 I2C sensor labs shipped as playground demos. Add a lab here when it
# gets an io-smoke.yaml and a demo release asset.
DEFAULT_LABS="ads1115-adc-lab adxl345-sensor-lab bme280-weather-lab ds3231-rtc-lab ina219-power-lab mpu6050-sensor-lab vl53l1x-tof-lab"
LABS="${LAB_SMOKE_LABS:-$DEFAULT_LABS}"

CLI="${1:-${LABWIRED_CLI:-}}"
if [ -z "$CLI" ]; then
  for p in target/debug/labwired target/release/labwired; do
    [ -x "$p" ] && CLI="$p" && break
  done
fi
if [ -z "$CLI" ] || [ ! -x "$CLI" ]; then
  echo "lab-smoke: no labwired CLI found (build -p labwired-cli --features event-scheduler, or set LABWIRED_CLI)" >&2
  exit 2
fi
CLI="$(cd "$(dirname "$CLI")" && pwd)/$(basename "$CLI")"

pkgs=()
for lab in $LABS; do pkgs+=(-p "$lab"); done
echo "lab-smoke: building ${LABS} for $TARGET"
if ! cargo build --release --target "$TARGET" "${pkgs[@]}"; then
  echo "lab-smoke: FIRMWARE BUILD FAILED" >&2
  exit 1
fi

failed=()
ran=0
for lab in $LABS; do
  for script in io-smoke.yaml stimuli-smoke.yaml; do
    [ -f "examples/$lab/$script" ] || continue
    ran=$((ran + 1))
    if out="$(cd "examples/$lab" && "$CLI" test --script "$script" --no-uart-stdout --output-dir "$ROOT/target/lab-smoke/$lab-${script%.yaml}" 2>&1)"; then
      echo "ok    $lab/$script"
    else
      echo "FAIL  $lab/$script"
      printf '%s\n' "$out" | grep -E "Assertion failed|ERROR|error" | sed 's/^/        /' | head -8
      failed+=("$lab/$script")
    fi
  done
done

if [ "$ran" -eq 0 ]; then
  echo "lab-smoke: no smoke scripts ran" >&2
  exit 1
fi
if [ "${#failed[@]}" -gt 0 ]; then
  echo "lab-smoke: ${#failed[@]} of $ran smoke script(s) FAILED: ${failed[*]}" >&2
  echo "lab-smoke: a simulator change broke a shipped demo lab (or the lab's expectation is stale)." >&2
  exit 1
fi
echo "lab-smoke: $ran smoke script(s) passed across $(echo "$LABS" | wc -w) lab(s)"
