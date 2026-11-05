# ESP32-C5-DevKitC-1 v1.2

This focused board profile runs a bare-metal RISC-V smoke on the ESP32-C5 HP
core and writes `ESP32_C5_UART0_OK` through the modeled UART0 TX FIFO. Firmware
selects UART0's fixed PLL_F80M source through the narrow PCR model. The
board guide identifies UART0 TX/RX as GPIO11/GPIO12 on J3 pins 2/3 and says the
USB-to-UART port is used to communicate with the chip.

Demand rank: **ESP32 family #1** in the 2025 JetBrains MCU-use survey (38%).
This adds model coverage for the current ESP32-C5-DevKitC-1 v1.2 board and its
240 MHz RISC-V core.

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
runner's build logs, smoke transcript, and metrics.

The profile models UART0's register behavior and TX FIFO capture. The USB-UART
bridge's electrical pin propagation, GPIO/RMT and the board's RGB LED are not
modeled. See [known limitations](KNOWN_LIMITATIONS.md).
