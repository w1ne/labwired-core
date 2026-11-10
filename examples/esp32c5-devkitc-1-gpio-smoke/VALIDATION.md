# Validation record

## Demand and selection

ESP32 is demand rank **#1** in the JetBrains State of Embedded 2025 survey
(38% of MCU use; Espressif was written for by 22% of respondents). The survey
does not rank the C5 separately. This is an ESP32 family-depth pick for the
newly listed ESP32-C5-DevKitC-1-N8R8 board, not a claim that the model represents
the C5's full feature set.

## Smoke scope

The firmware is built for `riscv32imac-unknown-none-elf`, loaded directly into
HP SRAM at `0x40800000`, sends `C5_UART_OK` through UART0's FIFO, and toggles
GPIO5. The CLI assertion requires the UART marker and at least four observed
edges on `gpio:5`, then confirms the configured instruction limit stopped the
loop. Committed build and simulation evidence is under
[`evidence/`](evidence/).

The C5-aware GPIO adapter covers the direct-output subset for GPIO0..25,
including C5-specific ENABLE and input-register offsets. UART0 uses the shared
Espressif FIFO/register model with C5's 240 MHz pacing and interrupt source ID.
The PCR clock gate and IOMUX descriptor are not enforced, so the twin captures
UART bytes through the host console sink without claiming that the U0TXD pin
reaches the board bridge. The system deliberately leaves unmodeled or
unverified:

- ROM boot, flash/XIP behavior, and PSRAM access;
- the ESP32-C5 LP core, CLIC/interrupt behavior, and C5-specific CSRs;
- Wi-Fi 6, Bluetooth LE, Zigbee, Thread, USB, and UART0 pin routing to the
  USB-to-UART bridge;
- GPIO26..GPIO28 matrix behavior and the GPIO27 addressable RGB LED;
- physical-board confirmation.

The RISC-V smoke uses base IMAC instructions and does not exercise the C5
interrupt or low-power subsystems. The UART assertion proves bytes leave the
modeled TX FIFO for the host sink; the GPIO assertion proves model output
edges. Neither is physical-board confirmation.

## Reproduce

```sh
scripts/onboarding_smoke.sh \
  --target-id esp32c5-devkitc-1-n8r8-gpio-uart \
  --crate firmware-esp32c5-devkitc-1-gpio-smoke \
  --target riscv32imac-unknown-none-elf \
  --script examples/esp32c5-devkitc-1-gpio-smoke/io-smoke.yaml \
  --system configs/systems/esp32c5-devkitc-1-n8r8-gpio.yaml \
  --out-dir out/onboarding-smoke/esp32c5-devkitc-1-n8r8-gpio-uart
```

This local simulator smoke is not hardware confirmation.
