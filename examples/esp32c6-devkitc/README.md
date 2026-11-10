# ESP32-C6-DevKitC-1 Onboarding Example

Run all commands from the repository root.

## Purpose

**Demand rank:** ESP32 family is #1 in the JetBrains 2025 MCU-use survey
(38%). This round extends the existing **ESP32-C6-DevKitC-1-N8** board smoke
with its active-low BOOT button on GPIO9; it is board-peripheral coverage, not
a new chip or board SKU. Espressif's live catalog lists the exact board for
purchase; see [`evidence/vendor-status.md`](evidence/vendor-status.md).

The deterministic L1 smoke covers the ESP32-C6 HP core (RV32IMAC @ 160 MHz),
UART0, the onboard WS2812B RGB LED, and the running-system GPIO9 button path:

1. `pcr` — UART0 APB + function clock gates (the C6's replacement for the C3's
   SYSTEM/APB_CTRL blocks)
2. `io_mux` — firmware writes GPIO16's U0TXD selection; the model records this
   register value but does not enforce pad routing
3. `gpio` — GPIO8 pad transitions from RMT TX and GPIO9's externally driven
   button input
4. `rgb_led` — the WS2812 model decodes the RMT GRB frame into solid red
5. `uart0` — Espressif UART IP clocked at the C6's rate, `OK` and button
   press/release markers to the console

The firmware prints `OK` through UART0, sends a red WS2812 frame through RMT
channel 0 on GPIO8, and polls the active-low GPIO9 BOOT button. The deterministic
stimulus presses then releases the button; the smoke checks both UART markers,
the GPIO8 edge count, and the decoded red pixel. The button input is driven by
the board contact model. IO_MUX records pad setup but does not enforce electrical
routing. The BOOT strapping effect at reset and the USB-to-UART bridge's
electrical propagation through GPIO16/GPIO17 are outside this profile.

## Copy-paste Arduino example

```cpp
constexpr int BOOT_BUTTON = 9;

void setup() {
  Serial.begin(115200);
  pinMode(BOOT_BUTTON, INPUT_PULLUP);
}

void loop() {
  if (digitalRead(BOOT_BUTTON) == LOW) {
    Serial.println("BOOT pressed");
  }
  delay(20);
}
```

**SIM-DERIVED.** Not silicon-verified. The LP core, ROM boot path, interrupt
matrix, every radio and every other peripheral are out of scope at this level —
see [`docs/boards/esp32c6-devkitc.md`](../../docs/boards/esp32c6-devkitc.md).

## Quick Run

```bash
cargo build -p firmware-esp32c6-demo --release --target riscv32imc-unknown-none-elf
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c6-devkitc/io-smoke.yaml \
  --output-dir out/esp32c6-devkitc/io-smoke \
  --no-uart-stdout
```

Expected result:

1. all six smoke assertions pass
2. UART contains `OK`, `BOOT_BUTTON_PRESSED`, and `BOOT_BUTTON_RELEASED`
3. GPIO8 carries the RMT frame and the modeled WS2812 pixel decodes as red

## Files

1. [`configs/systems/esp32c6-devkitc.yaml`](../../configs/systems/esp32c6-devkitc.yaml):
   UART0, the on-board WS2812 on GPIO8, and the active-low BOOT button on GPIO9.
2. `io-smoke.yaml`: deterministic button press/release stimulus plus UART,
   GPIO-edge, and decoded-pixel assertions.
3. `REQUIRED_DOCS.md`: source-grounding references (TRM, datasheet, DevKitC-1 guide, esp-idf headers, espressif/svd).
4. `EXTERNAL_COMPONENTS.md`: external component declaration.
5. `VALIDATION.md`: reproducible validation/audit commands.

The committed [`evidence/`](evidence/) directory contains the build and local
Core CLI smoke logs, a generated onboarding scoreboard, vendor availability,
and the hosted-verifier limitation.

The separate [MCPWM0 servo smoke](../esp32c6-devkitc-mcpwm-servo/README.md)
covers GPIO4/PWM0A at 50 Hz. It is an ESP32-family depth improvement (MCU
demand rank 1; ESP32 kits rank 2), not a new board SKU.
