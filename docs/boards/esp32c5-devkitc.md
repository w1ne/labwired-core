# ESP32-C5-DevKitC-1 v1.2

The ESP32-C5-DevKitC-1 v1.2 is a current Espressif development board built
around the ESP32-C5-WROOM-1 module. The N8R8 variant carries 8 MB flash and
8 MB PSRAM. ESP32 is demand rank **#1** in the 2025 JetBrains MCU-use survey
(38%); this adds a newly modeled ESP32-C5 board profile.

**Live availability checked 2026-11-05:** [Espressif's v1.2 board guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c5/esp32-c5-devkitc-1/user_guide.html)
lists the N8R8 variant and retail/wholesale order paths. [Mouser Europe lists
the exact ESP32-C5-DevKitC-1-N8R8](https://eu.mouser.com/en/ProductDetail/Espressif-Systems/ESP32-C5-DevKitC-1-N8R8?qs=sGAEpiMZZMv6ooMj3OUeOcfessnCMo2lVVrnr4OuMYfdALqi8VUu1g%3D%3D)
with 632 in stock for immediate dispatch when checked. DigiKey lists the same
part as **Active**, though its checked stock was zero. Inventory changes over
time. The older v1.1 guide revision is outside this profile.

## Profile and smoke

The profile models the HP RV32IMAC core, flash/SRAM/ROM address windows, a
narrow PCR slice for UART0 clock setup, and UART0's register/FIFO behavior. The
committed bare-metal firmware selects the fixed PLL_F80M UART source and sends
`ESP32_C5_UART0_OK` through UART0, which the board guide maps to GPIO11/U0TXD
on J3 pin 2; GPIO12/U0RXD is J3 pin 3. The smoke and evidence are in
[`examples/esp32c5-devkitc/`](../../examples/esp32c5-devkitc/).

| Aspect | Profile |
|---|---|
| Chip descriptor | [`configs/chips/esp32c5.yaml`](../../configs/chips/esp32c5.yaml) |
| Board system | [`configs/systems/esp32c5-devkitc.yaml`](../../configs/systems/esp32c5-devkitc.yaml) |
| Smoke firmware | [`crates/firmware-esp32c5-devkitc-smoke/`](../../crates/firmware-esp32c5-devkitc-smoke/) |
| Board pins used | UART0 TX GPIO11 / J3-2; UART0 RX GPIO12 / J3-3 |
| Evidence | [`examples/esp32c5-devkitc/evidence/`](../../examples/esp32c5-devkitc/evidence/) |
| Modeling level | L1, SIM-DERIVED; no silicon capture or ROM boot claim |

The onboard addressable RGB LED is wired to GPIO27, but the C5-specific GPIO
and RMT paths are not part of this descriptor. The USB-UART bridge's electrical
pad routing, PSRAM, radios, native USB, LP core and dynamic clock tree are also
outside the profile. See [known limitations](../../examples/esp32c5-devkitc/KNOWN_LIMITATIONS.md).

## Run the example

```bash
rustup target add riscv32imc-unknown-none-elf
cargo build -p firmware-esp32c5-devkitc-smoke --release --target riscv32imc-unknown-none-elf
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c5-devkitc/onboarding-smoke.yaml \
  --output-dir out/esp32c5-devkitc \
  --no-uart-stdout
```
