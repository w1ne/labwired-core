# ESP32-C61-DevKitC-1 v2.0 (N8R2)

**Demand rank:** ESP32 family is #1 in the measured MCU-use survey (38%). This
adds Espressif's current ESP32-C61 Wi-Fi 6 / BLE RISC-V SoC on its official
DevKitC board.

**Live availability checked 2026-11-05:** Espressif lists the chip in mass
production and the current board guide lists the N8R2 variant with retail sample
orders. DigiKey's exact [ESP32-C61-DevKitC-1-N8R2 listing](https://www.digikey.ca/en/products/detail/espressif-systems/ESP32-C61-DEVKITC-1-N8R2/26763163)
showed Espressif Systems, **Active**, 229 in stock and C$14.25 for one when
checked. See the complete [vendor status evidence](../../examples/esp32c61-devkitc/evidence/vendor-status.md).

## Profile and smoke

The committed bare-metal firmware runs on the HP RISC-V core and sends
`ESP32_C61_UART0_OK` through UART0's TX FIFO. The official board guide connects
UART0 TX to GPIO11/U0TXD at J3-2 and UART0 RX to GPIO10/U0RXD at J3-3, where the
USB-to-UART bridge is attached. The register offsets and PCR clock bits used by
the firmware match Espressif's C61 ESP-IDF headers; the UART model reuses the
C6 implementation for that shared register map.

| Aspect | Profile |
|---|---|
| Chip descriptor | [`configs/chips/esp32c61.yaml`](../../configs/chips/esp32c61.yaml) |
| Board system | [`configs/systems/esp32c61-devkitc.yaml`](../../configs/systems/esp32c61-devkitc.yaml) |
| Smoke firmware | [`crates/firmware-esp32c61-devkitc-smoke/`](../../crates/firmware-esp32c61-devkitc-smoke/) |
| Smoke and copy-paste example | [`examples/esp32c61-devkitc/`](../../examples/esp32c61-devkitc/) |
| Evidence | [`examples/esp32c61-devkitc/evidence/`](../../examples/esp32c61-devkitc/evidence/) |
| Modeling level | Local deterministic UART smoke; no silicon capture or ROM boot claim |

The profile exposes the HP core, flash/SRAM windows, UART0 and the UART0 PCR
clock path. GPIO/IO_MUX routing, the addressable RGB LED on GPIO8, PSRAM, ROM
boot, Wi-Fi, Bluetooth LE, USB Serial/JTAG and the LP subsystem are out of
profile. The local UART smoke does not provide a hosted `model_verified` verdict
or physical-board confirmation.

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
