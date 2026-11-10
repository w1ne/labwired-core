# Validation record

## Demand and selection

ESP32 is demand rank **#1** in the JetBrains State of Embedded 2025 survey
(38% of MCU use; Espressif was written for by 22% of respondents). The survey
does not rank the C5 separately. This is an ESP32 family-depth pick for the
newly listed ESP32-C5-DevKitC-1-N8R8 board, not a claim that the model represents
the C5's full feature set.

## Smoke scope

The firmware is built for `riscv32imac-unknown-none-elf`, loaded directly into
HP SRAM at `0x40800000`, configures the C5 I2C0 GPIO matrix on SDA GPIO8 and SCL
GPIO9, reads BME280 registers `0xD0` and `0xFA`–`0xFC`, and reports the silicon
model's ID and raw temperature through UART0. It also toggles GPIO5. The CLI
assertion requires the success marker emitted only after checking `ID=0x60`
and raw temperature `0x082DC0`, at least four observed edges on `gpio:5`, and
the configured instruction-limit stop. Committed build and simulation evidence
is under [`evidence/`](evidence/).

The C5-aware GPIO adapter covers direct GPIO0..25 plus the C5 input/output
matrix register windows needed for I2C0 signals 46/47. I2C0 uses the shared
Espressif command-list bit engine, paced at C5's 240 MHz core rate and gated on
the firmware's live GPIO8/9 input/output routes. UART0 uses the shared FIFO
register model. The PCR clock gate and IOMUX pad electrical behavior are not
enforced; UART bytes reach the host console sink without claiming U0TXD reaches
the board bridge. The system deliberately leaves unmodeled or unverified:

- ROM boot, flash/XIP behavior, and PSRAM access;
- the ESP32-C5 LP core, CLIC/interrupt behavior, and C5-specific CSRs;
- Wi-Fi 6, Bluetooth LE, Zigbee, Thread, USB, and UART0 pin routing to the
  USB-to-UART bridge;
- matrix functions beyond I2C0 (GPIO26..GPIO28, UART pin routing) and the GPIO27
  addressable RGB LED;
- the C5 LP core, PCR clock/reset gating, boot ROM, flash/XIP, and PSRAM;
- physical-board confirmation.

No C5 UART, GPIO, or I2C window in this system is a zero-answering stub. The
IO_MUX remains declarative register storage, PCR is absent from the descriptor,
and no electrical IO_MUX/pad-level behavior is claimed. GPIO7 is avoided
because it is a board strapping pin. The RISC-V smoke uses base IMAC
instructions and does not exercise C5 interrupts or low-power subsystems. This
local CLI pass is simulator evidence, not model-oracle verification or
physical-board confirmation.

## Reproduce

```sh
scripts/onboarding_smoke.sh \
  --target-id esp32c5-devkitc-1-n8r8-bme280 \
  --crate firmware-esp32c5-devkitc-1-bme280-smoke \
  --target riscv32imac-unknown-none-elf \
  --script examples/esp32c5-devkitc-1-bme280-smoke/io-smoke.yaml \
  --system configs/systems/esp32c5-devkitc-1-n8r8-bme280.yaml \
  --out-dir out/onboarding-smoke/esp32c5-devkitc-1-n8r8-bme280
```

This local simulator smoke is not hardware confirmation.
