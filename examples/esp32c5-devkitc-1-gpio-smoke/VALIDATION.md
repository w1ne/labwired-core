# Validation record

## Demand and selection

ESP32 is demand rank **#1** in the JetBrains State of Embedded 2025 survey
(38% of MCU use; Espressif was written for by 22% of respondents). The survey
does not rank the C5 separately. This is an ESP32 family-depth pick for the
newly listed ESP32-C5-DevKitC-1-N8R8 board, not a claim that the model represents
the C5's full feature set.

## Smoke scope

The firmware is built for `riscv32imac-unknown-none-elf`, loaded directly into
HP SRAM at `0x40800000`, and writes GPIO5's IOMUX function and GPIO output
registers. The CLI assertion requires at least four observed edges on
`gpio:5`, then confirms the configured instruction limit stopped the loop.
Committed build and simulation evidence is under [`evidence/`](evidence/).

The C5-aware GPIO adapter covers the direct-output subset for GPIO0..25,
including C5-specific ENABLE and input-register offsets. The IOMUX descriptor
is declarative storage, so pad routing is not electrically enforced in the
twin. The system deliberately leaves unmodeled or unverified:

- ROM boot, flash/XIP behavior, and PSRAM access;
- the ESP32-C5 LP core, CLIC/interrupt behavior, and C5-specific CSRs;
- Wi-Fi 6, Bluetooth LE, Zigbee, Thread, USB, and the USB-to-UART bridge;
- GPIO26..GPIO28 matrix behavior and the GPIO27 addressable RGB LED;
- physical-board confirmation.

The RISC-V smoke uses base IMAC instructions and does not exercise the C5
interrupt or low-power subsystems. The output-edge assertion proves the modeled
GPIO register path only.

## Reproduce

```sh
scripts/onboarding_smoke.sh \
  --target-id esp32c5-devkitc-1-n8r8-gpio \
  --crate firmware-esp32c5-devkitc-1-gpio-smoke \
  --target riscv32imac-unknown-none-elf \
  --script examples/esp32c5-devkitc-1-gpio-smoke/io-smoke.yaml \
  --system configs/systems/esp32c5-devkitc-1-n8r8-gpio.yaml \
  --out-dir out/onboarding-smoke/esp32c5-devkitc-1-n8r8-gpio
```

This local simulator smoke is not hardware confirmation.
