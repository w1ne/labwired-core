# ESP32-C5-DevKitC-1 v1.2

The ESP32-C5-DevKitC-1 v1.2 is a current Espressif development board built
around the ESP32-C5-WROOM-1 module. The N8R8 variant carries 8 MB flash and
8 MB PSRAM. ESP32 is demand rank **#1** in the 2025 JetBrains MCU-use survey
(38%). This round extends the existing ESP32-C5 board coverage with its
modeled GPIO27 RGB LED path.

**Live availability checked 2026-11-05:** [Espressif's v1.2 board guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c5/esp32-c5-devkitc-1/user_guide.html)
lists the N8R8 variant and retail/wholesale order paths. DigiKey's [exact
ESP32-C5-DEVKITC-1-N8R8 listing](https://www.digikey.com/en/products/detail/espressif-systems/ESP32-C5-DEVKITC-1-N8R8/28718162)
reported **Part Status: Active**, **373 in stock**, and **US$15.00** for one
unit when checked. Inventory changes over time. The older v1.1 guide revision
is outside this profile.

## Profile and smoke

The profile models the HP RV32IMAC core, flash/SRAM/ROM address windows, a
narrow PCR slice for UART0 clock setup, UART0's register/FIFO behavior, and
GPIO27's direct output plus IO_MUX function-select path. The committed
bare-metal firmware selects the fixed PLL_F80M UART source and sends
`ESP32_C5_UART0_OK` through UART0, which the board guide maps to GPIO11/U0TXD
on J3 pin 2; GPIO12/U0RXD is J3 pin 3. It also bit-bangs one red GRB
WS2812 frame to the onboard RGB LED on GPIO27 using the modeled GPIO path. The
smoke and evidence are in
[`examples/esp32c5-devkitc/`](../../examples/esp32c5-devkitc/).

| Aspect | Profile |
|---|---|
| Chip descriptor | [`configs/chips/esp32c5.yaml`](../../configs/chips/esp32c5.yaml) |
| Board system | [`configs/systems/esp32c5-devkitc.yaml`](../../configs/systems/esp32c5-devkitc.yaml) |
| Smoke firmware | [`crates/firmware-esp32c5-devkitc-smoke/`](../../crates/firmware-esp32c5-devkitc-smoke/) |
| Board pins used | UART0 TX GPIO11 / J3-2; UART0 RX GPIO12 / J3-3; RGB LED GPIO27 |
| Evidence | [`examples/esp32c5-devkitc/evidence/`](../../examples/esp32c5-devkitc/evidence/) |
| Modeling level | Local deterministic UART/GPIO/WS2812 smoke; no silicon capture or ROM boot claim |

The RGB LED model exercises GPIO27 directly and does not model RMT or other
GPIO pads. The USB-UART bridge's electrical pad routing, PSRAM, radios, native
USB, LP core and dynamic clock tree are also outside the profile. See
[known limitations](../../examples/esp32c5-devkitc/KNOWN_LIMITATIONS.md).

The checked-in fixture proves that the local runner accepts UART output, GPIO27
edges and the red LED frame. The external `labwired_verify` service does not
currently list ESP32-C5 as a supported target, so this is not a hosted
`model_verified` result or a physical-board test.

## Run the example

```bash
rustup target add riscv32imc-unknown-none-elf
cargo build -p firmware-esp32c5-devkitc-smoke --release --target riscv32imc-unknown-none-elf
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c5-devkitc/onboarding-smoke.yaml \
  --output-dir out/esp32c5-devkitc \
  --no-uart-stdout
```
