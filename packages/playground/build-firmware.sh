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
(cd "$CORE_DIR" && ~/.cargo/bin/wasm-pack build crates/wasm \
  --target web \
  --release \
  --out-dir "$OUT_DIR" \
  -- --features wasm-bindgen)

echo "WASM module → $OUT_DIR/labwired_wasm.js"

# ── 2. STM32F103 Nucleo-F103RB Arduino Blink ─────────────────────────────────
echo "Compiling Arduino Blink for STM32F103 (Nucleo-F103RB)..."
SKETCH_DIR="$(mktemp -d)/blink_f103"
mkdir -p "$SKETCH_DIR"
cat > "$SKETCH_DIR/blink_f103.ino" << 'SKETCH'
// Arduino Blink for STM32 Nucleo boards (LD2 on PA5).
// Prints to Serial (USART2) for the simulator's UART monitor.

void setup() {
  pinMode(LED_BUILTIN, OUTPUT);
  Serial.begin(115200);
  Serial.println("LabWired Playground: STM32F103 Blinky (Arduino)");
}

void loop() {
  digitalWrite(LED_BUILTIN, HIGH);
  Serial.println("ON");
  delay(500);
  digitalWrite(LED_BUILTIN, LOW);
  Serial.println("OFF");
  delay(500);
}
SKETCH

BUILD_TMP="$(mktemp -d)"
arduino-cli compile \
  --fqbn "STMicroelectronics:stm32:Nucleo_64:pnum=NUCLEO_F103RB" \
  --output-dir "$BUILD_TMP" \
  "$SKETCH_DIR"
cp "$BUILD_TMP"/*.ino.elf "$OUT_DIR/demo-blinky.bin"
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

echo ""
echo "Done. Firmware in $OUT_DIR:"
ls -lh "$OUT_DIR"/*.{bin,elf,wasm,js} 2>/dev/null || true
