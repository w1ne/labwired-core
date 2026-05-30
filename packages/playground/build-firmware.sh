#!/usr/bin/env bash
# Build all firmware binaries for the playground.
# Outputs ELF files to public/wasm/.
#
# Requirements:
#   - wasm-pack (cargo install wasm-pack)
#   - arduino-cli with STMicroelectronics:stm32 core
#   - arm-none-eabi-gcc (for bare-metal fallbacks)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
CORE_DIR="$REPO_ROOT/core"
OUT_DIR="$SCRIPT_DIR/public/wasm"

mkdir -p "$OUT_DIR"

# ── 1. WASM simulator module ──────────────────────────────────────────────────
echo "Building labwired-wasm..."
# NOTE: `labwired-wasm` has no `wasm-bindgen` *feature* (wasm-bindgen is a
# plain dependency), so the old `-- --features wasm-bindgen` errored and left
# this wasm stale. The event-scheduler perf path ships via the labwired-core
# dependency's own `event-scheduler` feature (always on for the wasm crate),
# so a plain release build picks it up.
(cd "$CORE_DIR" && ~/.cargo/bin/wasm-pack build crates/wasm \
  --target web \
  --release \
  --out-dir "$OUT_DIR")

echo "WASM module → $OUT_DIR/labwired_wasm.js"

# ── 2. STM32F103 simulator-native blink demo ─────────────────────────────────
echo "Copying known-good STM32F103 simulator blink firmware..."
(cd "$CORE_DIR" && cargo build -p demo-blinky --release --target thumbv7m-none-eabi)
cp "$CORE_DIR/target/thumbv7m-none-eabi/release/demo-blinky" "$OUT_DIR/demo-blinky.bin"
echo "STM32F103 firmware → $OUT_DIR/demo-blinky.bin"

# ── 3. STM32F401RE Nucleo Arduino Blink + Button ─────────────────────────────
echo "Compiling Arduino Blink for STM32F401RE (Nucleo-F401RE)..."
SKETCH_DIR_F401="$(mktemp -d)/blink_f401"
mkdir -p "$SKETCH_DIR_F401"
cat > "$SKETCH_DIR_F401/blink_f401.ino" << 'SKETCH'
// Arduino Blink + Button for STM32F401RE Nucleo.
// LD2 (LED) = PA5, B1 (USER button) = PC13.
// Prints to Serial (USART2).

const int BTN = PC13;

void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  pinMode(BTN, INPUT);
  Serial.begin(115200);
  Serial.println("LabWired Playground: STM32F401RE Nucleo (Arduino)");
}

void loop() {
  bool pressed = (digitalRead(BTN) == LOW);
  digitalWrite(LED_BUILTIN, pressed ? HIGH : (millis() / 500) % 2);
  if (pressed) {
    Serial.println("BTN pressed");
    delay(50);
  }
}
SKETCH

BUILD_TMP_F401="$(mktemp -d)"
arduino-cli compile \
  --fqbn "STMicroelectronics:stm32:Nucleo_64:pnum=NUCLEO_F401RE" \
  --output-dir "$BUILD_TMP_F401" \
  "$SKETCH_DIR_F401"
cp "$BUILD_TMP_F401"/*.ino.elf "$OUT_DIR/demo-nucleo-f401.elf"
echo "STM32F401RE firmware → $OUT_DIR/demo-nucleo-f401.elf"

# ── STM32L476 Nokia 5110 Breakout + HC-SR04 ──────────────────────────────────
# Same .elf runs in the sim AND flashes to a real NUCLEO-L476RG (HW-validated).
echo "Building STM32L476 Nokia 5110 Breakout firmware..."
(cd "$CORE_DIR" && cargo build -p nokia5110-invaders-lab --release --target thumbv7em-none-eabihf)
cp "$CORE_DIR/target/thumbv7em-none-eabihf/release/nokia5110-invaders-lab" "$OUT_DIR/demo-nokia5110-invaders-lab.elf"
echo "STM32L476 Nokia 5110 firmware → $OUT_DIR/demo-nokia5110-invaders-lab.elf"

# ── 4. nRF52840 BLE two-radio demo (sensor + collector) ──────────────────────
# Same ELFs run in the sim AND flash to real nRF silicon (parity-proven).
# The sensor broadcasts an incrementing reading; the collector receives it.
# They talk over the engine's shared virtual-air BLE registry.
echo "Building nRF52840 BLE sensor + collector firmware..."
(cd "$CORE_DIR" && cargo build --release --target thumbv7em-none-eabihf \
  -p firmware-nrf52840-ble-sensor -p firmware-nrf52840-ble-collector)
cp "$CORE_DIR/target/thumbv7em-none-eabihf/release/firmware-nrf52840-ble-sensor" \
  "$OUT_DIR/demo-nrf52840-ble-sensor.elf"
cp "$CORE_DIR/target/thumbv7em-none-eabihf/release/firmware-nrf52840-ble-collector" \
  "$OUT_DIR/demo-nrf52840-ble-collector.elf"
echo "nRF BLE firmware → $OUT_DIR/demo-nrf52840-ble-{sensor,collector}.elf"

echo ""
echo "Done. Firmware in $OUT_DIR:"
ls -lh "$OUT_DIR"/*.{bin,elf,wasm,js} 2>/dev/null || true
