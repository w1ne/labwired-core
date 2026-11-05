# ESP32-C61-DevKitC-1 v2.0 (N8R2)

**Demand rank:** ESP32 family is #1 in the measured MCU-use survey (38%). This
round deepens the existing C61 DevKitC-1 profile with its onboard RGB LED; it
does not add a new chip model.

**Live availability checked 2026-11-05:** Espressif lists the chip in mass
production and the current board guide lists the N8R2 variant with retail sample
orders. DigiKey's exact [ESP32-C61-DevKitC-1-N8R2 listing](https://www.digikey.ca/en/products/detail/espressif-systems/ESP32-C61-DEVKITC-1-N8R2/26763163)
showed Espressif Systems, **Active**, 229 in stock and C$14.25 for one when
checked. See the complete [vendor status evidence](../../examples/esp32c61-devkitc/evidence/vendor-status.md).

## Profile and smoke

The committed bare-metal firmware runs on the HP RISC-V core, sends
`ESP32_C61_UART0_OK` through UART0's TX FIFO, and bit-bangs a red WS2812B frame
on GPIO8. The official board guide connects UART0 TX to GPIO11/U0TXD at J3-2
and UART0 RX to GPIO10/U0RXD at J3-3, where the USB-to-UART bridge is attached.
The board guide assigns the addressable RGB LED to GPIO8, and the official
schematic identifies it as D6 WS2812B. Register offsets in the added GPIO8
path match Espressif's C61 ESP-IDF headers; only the UART model reuses C6 code.

| Aspect | Profile |
|---|---|
| Chip descriptor | [`configs/chips/esp32c61.yaml`](../../configs/chips/esp32c61.yaml) |
| Board system | [`configs/systems/esp32c61-devkitc.yaml`](../../configs/systems/esp32c61-devkitc.yaml) |
| Smoke firmware | [`crates/firmware-esp32c61-devkitc-smoke/`](../../crates/firmware-esp32c61-devkitc-smoke/) |
| Smoke and copy-paste example | [`examples/esp32c61-devkitc/`](../../examples/esp32c61-devkitc/) |
| Evidence | [`examples/esp32c61-devkitc/evidence/`](../../examples/esp32c61-devkitc/evidence/) |
| Modeling level | Local deterministic UART + GPIO8/WS2812B smoke; no silicon capture or ROM boot claim |

The profile exposes the HP core, flash/SRAM windows, UART0 and its PCR clock
path, GPIO8 OUT/W1TS/W1TC, ENABLE_W1TS, GPIO_FUNC8_OUT_SEL_CFG/OE_SEL, and the
GPIO8 IO_MUX register. Its GPIO model is deliberately limited to the direct GPIO8
output path; it does not model the C61 matrix or other pads. The WS2812B model
decodes the bit-banged GRB output and the smoke checks for red. PSRAM, ROM boot,
Wi-Fi, Bluetooth LE, USB Serial/JTAG and the LP subsystem remain out of profile.
The local smoke does not provide a hosted `model_verified` verdict or
physical-board confirmation.

## Run the smoke

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32c61-devkitc \
  --crate firmware-esp32c61-devkitc-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c61-devkitc/onboarding-smoke.yaml \
  --system configs/systems/esp32c61-devkitc.yaml \
  --out-dir examples/esp32c61-devkitc/evidence
```
