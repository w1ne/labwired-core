#!/usr/bin/env bash
# Compile examples/arduino-conformance with each board's REAL Arduino core and
# run the result on the model, reporting one line per chip.
#
# Why this exists: an Arduino core exercises far more of a chip than a
# hand-written Tier-1 register fixture does, because it goes through the vendor
# HAL the way user firmware does. Every model bug this sweep has found so far
# sat in a peripheral whose Tier-1 cell was already green — fixtures assert what
# the models implement, so they are structurally blind to a model that is
# self-consistent but wrong about silicon.
#
# NOT run in CI: it needs arduino-cli plus several vendor cores (~GB of
# toolchains) and takes minutes per board. Run it locally when touching a
# peripheral model.
#
# Prerequisites:
#   arduino-cli, plus the cores used below:
#     arduino-cli core install STMicroelectronics:stm32
#     arduino-cli core install esp32:esp32
#   with these board-manager URLs registered:
#     https://github.com/stm32duino/BoardManagerFiles/raw/main/package_stmicroelectronics_index.json
#     https://espressif.github.io/arduino-esp32/package_esp32_index.json
#
# Usage:  scripts/arduino-conformance-sweep.sh [chip ...]
#         (no args = every board below)
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CLI="${ARDUINO_CLI:-$HOME/.local/bin/arduino-cli}"
SIM="$ROOT/target/release/labwired"
OUT="${TMPDIR:-/tmp}/labwired-arduino-sweep"
SKETCH="$ROOT/examples/arduino-conformance"
MAX_STEPS="${MAX_STEPS:-80000000}"

# chip <TAB> fqbn.  The chip name must match configs/chips/<chip>.yaml.
BOARDS=$(cat <<'EOF'
stm32f103	STMicroelectronics:stm32:GenF1:pnum=BLUEPILL_F103C8
stm32f401	STMicroelectronics:stm32:Nucleo_64:pnum=NUCLEO_F401RE
stm32f407	STMicroelectronics:stm32:GenF4:pnum=DIYMORE_F407VGT
stm32l476	STMicroelectronics:stm32:Nucleo_64:pnum=NUCLEO_L476RG
stm32g474re	STMicroelectronics:stm32:Nucleo_64:pnum=NUCLEO_G474RE
stm32wb55	STMicroelectronics:stm32:Nucleo_64:pnum=P_NUCLEO_WB55RG
stm32l073	STMicroelectronics:stm32:Nucleo_64:pnum=NUCLEO_L073RZ
stm32h563	STMicroelectronics:stm32:Nucleo_144:pnum=NUCLEO_H563ZI
esp32c3	esp32:esp32:esp32c3
esp32s3	esp32:esp32:esp32s3
rp2040	rp2040:rp2040:rpipico
EOF
)

[ -x "$SIM" ] || { echo "build the sim first: cargo build --release -p labwired-cli"; exit 1; }
[ -x "$CLI" ] || { echo "arduino-cli not found at $CLI (set ARDUINO_CLI)"; exit 1; }

mkdir -p "$OUT"
want=("$@")

printf '%-14s %s\n' "CHIP" "RESULT"
while IFS=$'\t' read -r chip fqbn; do
  [ -z "$chip" ] && continue
  if [ ${#want[@]} -gt 0 ] && [[ ! " ${want[*]} " == *" $chip "* ]]; then continue; fi

  d="$OUT/$chip"; mkdir -p "$d"
  if ! timeout 900 "$CLI" compile -b "$fqbn" --output-dir "$d" "$SKETCH" >"$d/build.log" 2>&1; then
    printf '%-14s %s\n' "$chip" "COMPILE_FAIL (see $d/build.log)"; continue
  fi

  elf=$(ls "$d"/*.elf 2>/dev/null | head -1)
  [ -z "$elf" ] && { printf '%-14s %s\n' "$chip" "NO_ELF"; continue; }

  # ESP32 parts boot ROM -> 2nd-stage bootloader -> app out of a FLASH IMAGE; a
  # bare ELF never starts. arduino-cli already emits *.merged.bin with the
  # bootloader, partition table and app at the right offsets, so no custom
  # merge step is needed — hand it to the sim's faithful --rom-boot path.
  merged=$(ls "$d"/*.merged.bin 2>/dev/null | head -1)
  if [ -n "$merged" ]; then
    case "$chip" in
      esp32c3) flashvar=LABWIRED_ESP32C3_FLASH ;;
      esp32s3) flashvar=LABWIRED_ESP32S3_FLASH ;;
      *)       flashvar="" ;;
    esac
    if [ -n "$flashvar" ]; then
      res=$(env "$flashvar=$merged" timeout 500 "$SIM" run --chip "$ROOT/configs/chips/$chip.yaml" \
            --firmware "$elf" --rom-boot --max-steps "$MAX_STEPS" 2>/dev/null \
            | grep -oE 'LWCONF (uart|gpio|timer|i2c|spi|done)( (PASS|FAIL|SKIP))?' | tr '\n' ' ')
      printf '%-14s %s\n' "$chip" "${res:-NO_OUTPUT}"; continue
    fi
  fi

  res=$(timeout 500 "$SIM" run --chip "$ROOT/configs/chips/$chip.yaml" \
        --firmware "$elf" --max-steps "$MAX_STEPS" 2>/dev/null \
        | grep -oE 'LWCONF (uart|gpio|timer|i2c|spi|done)( (PASS|FAIL|SKIP))?' | tr '\n' ' ')
  printf '%-14s %s\n' "$chip" "${res:-NO_OUTPUT}"
done <<< "$BOARDS"
