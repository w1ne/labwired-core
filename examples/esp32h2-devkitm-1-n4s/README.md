# ESP32-H2-DevKitM-1-N4S GPIO8 smoke

ESP32 is the rank 1 MCU family in the current onboarding order (38% usage;
ESP32 kits were used by 42% of respondents in the JetBrains State of Embedded
survey for 2025). This board adds specific H2 coverage within that high-demand
family; the survey does not rank H2 separately.

This narrow profile boots a direct-entry RV32IMC firmware image and drives the
board's onboard addressable RGB LED on GPIO8. The committed smoke checks GPIO
edges and the decoded red pixel against LabWired Core's local model. It does not
claim H2 ROM boot, UART/USB, Bluetooth LE, Thread, Zigbee, RMT, or other H2
peripherals. See [`KNOWN_LIMITATIONS.md`](KNOWN_LIMITATIONS.md) and the
[board notes](../../docs/boards/esp32h2-devkitm-1-n4s.md).

## Quick run

From the repository root:

```bash
cargo build -p firmware-esp32h2-demo --release --target riscv32imc-unknown-none-elf
cargo run -q -p labwired-cli -- test \
  --script examples/esp32h2-devkitm-1-n4s/io-smoke.yaml \
  --output-dir out/esp32h2-devkitm-1-n4s/io-smoke \
  --no-uart-stdout
```

## Files

- [`configs/chips/esp32h2-devkitm-1-n4s-smoke.yaml`](../../configs/chips/esp32h2-devkitm-1-n4s-smoke.yaml): H2 HP core and the single GPIO register path used by this smoke.
- [`configs/systems/esp32h2-devkitm-1-n4s.yaml`](../../configs/systems/esp32h2-devkitm-1-n4s.yaml): board's GPIO8 NeoPixel wiring.
- [`io-smoke.yaml`](io-smoke.yaml): edge, pixel-color, and stop-reason assertions.
- [`evidence/`](evidence/): local smoke output, build logs, and live vendor status.
