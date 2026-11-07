# Arduino Nano R4

This profile boots an RA4M1 firmware smoke and toggles the Nano R4's
active-high `LED_BUILTIN`. The LED is wired to RA4M1 P204 (ArduinoCore-renesas
NANOR4 variant pin 22), through the modeled PORT2 GPIO.

The profile is deliberately limited to the one exercised board path:

- Board: Arduino Nano R4, SKU ABX00142.
- MCU: Renesas RA4M1, Cortex-M4 at 48 MHz, 256 KiB flash, 32 KiB SRAM.
- Pin map: `LED_BUILTIN` / Arduino pin 22 → P204 → PORT2 bit 4.
- Modeled peripheral used by the smoke: RA4M1 PORT2 GPIO.

USB programming/CDC, the UART header, Qwiic and header I²C, SPI, ADC, DAC,
CAN, timers, the RGB LED, power sequencing, and the board's 3.3 V supply are
outside this profile. No stub peripheral is needed by the smoke.

Reproduce from the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id arduino-nano-r4 \
  --crate firmware-arduino-nano-r4-smoke \
  --target thumbv7em-none-eabi \
  --script examples/arduino-nano-r4/onboarding-smoke.yaml \
  --system configs/systems/arduino-nano-r4.yaml \
  --out-dir examples/arduino-nano-r4/evidence
```

The committed build and simulator logs are in [`evidence/logs/`](evidence/logs/),
and the vendor lifecycle/orderability check is in
[`evidence/vendor-status.md`](evidence/vendor-status.md).
