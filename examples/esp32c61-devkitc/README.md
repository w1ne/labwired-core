# ESP32-C61-DevKitC-1 v2.0 (N8R2)

**Demand rank:** ESP32 family, rank 1 (38% of respondents use an ESP32). This
onboards the active ESP32-C61 Wi-Fi 6 / BLE RISC-V SoC on Espressif's current
DevKitC-1 board, extending coverage of the survey's highest-demand MCU family.

The board smoke runs bare-metal firmware built for `riscv32imc` (an instruction
subset of the C61's RV32IMAC core) and checks UART0 output from the TX FIFO.
UART0 is the real peripheral exercised by the example; the board routes
GPIO11/U0TXD and GPIO10/U0RXD to its USB-to-UART bridge. The engine profile
covers the HP core, flash/SRAM address windows, UART0, and the UART0 PCR clock
registers.

## Arduino sketch example

The official [Arduino-ESP32 core supports C61](https://github.com/espressif/arduino-esp32/blob/master/docs/en/getting_started.rst), but currently only when used as an ESP-IDF component or with rebuilt core libraries; it is not a stock Arduino IDE board package. In that setup, this sketch prints through UART0:

```cpp
void setup() {
  Serial.begin(115200);
  Serial.println("ESP32_C61_UART0_OK");
}

void loop() {
  delay(1000);
}
```

## Run the committed smoke

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id esp32c61-devkitc \
  --crate firmware-esp32c61-devkitc-smoke \
  --target riscv32imc-unknown-none-elf \
  --script examples/esp32c61-devkitc/onboarding-smoke.yaml \
  --system configs/systems/esp32c61-devkitc.yaml \
  --out-dir examples/esp32c61-devkitc/evidence
```

The model smoke loads an ELF directly; ROM and second-stage boot are outside
this profile. Committed build and simulator evidence is under
[`evidence/`](evidence/), and live vendor availability is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model boundary

The profile covers the HP RISC-V core, 8 MB flash and 320 KiB SRAM windows,
UART0's register path and TX FIFO, and the PCR UART0 clock bits. Out of profile:
ROM boot, PSRAM, GPIO/IO_MUX routing, the addressable RGB LED on GPIO8, Wi-Fi,
Bluetooth LE, USB Serial/JTAG, and the LP subsystem. The header pin names are
board documentation; pad-level electrical routing is outside the model. The
evidence is a local deterministic core smoke. There is no hosted
`model_verified` verdict or physical-board confirmation.
