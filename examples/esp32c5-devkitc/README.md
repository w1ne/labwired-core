# ESP32-C5-DevKitC-1 v1.2

This focused board profile runs a bare-metal RISC-V smoke on the ESP32-C5 HP
core, writes `ESP32_C5_UART0_OK` through the modeled UART0 TX FIFO, and sends a
red GRB WS2812 frame to the onboard RGB LED through GPIO27. Firmware selects
UART0's fixed PLL_F80M source through the narrow PCR model. Espressif's board
guide identifies UART0 TX/RX as GPIO11/GPIO12 on J3 pins 2/3 and the RGB LED as
GPIO27.

Demand rank: **ESP32 family #1** in the 2025 JetBrains MCU-use survey (38%).
This extends the existing ESP32-C5-DevKitC-1 v1.2 coverage with the board's
GPIO27-driven RGB LED.

## Quick run

From the repository root:

```bash
rustup target add riscv32imc-unknown-none-elf
cargo build -p firmware-esp32c5-devkitc-smoke --release --target riscv32imc-unknown-none-elf
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c5-devkitc/onboarding-smoke.yaml \
  --output-dir out/esp32c5-devkitc \
  --no-uart-stdout
```

The committed [`evidence/`](evidence/) directory contains the onboarding
runner's build logs, smoke transcript, simulation output, and metrics. To copy
and run the complete firmware example, use
[`src/main.rs`](../../crates/firmware-esp32c5-devkitc-smoke/src/main.rs) with
the Cargo command above; the smoke checks UART output, GPIO27 edges, and the
decoded red LED frame.

The GPIO model is limited to the direct GPIO27 output path needed by the
onboard LED. RMT, other GPIO pads, and the USB-UART bridge's electrical pin
propagation are not modeled. This repository smoke is local runner evidence;
the external `labwired_verify` service does not yet list ESP32-C5 as a
supported target. See [known limitations](KNOWN_LIMITATIONS.md).
