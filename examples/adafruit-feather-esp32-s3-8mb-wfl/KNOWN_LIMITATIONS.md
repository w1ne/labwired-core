# Known limitations — Adafruit ESP32-S3 Feather 8 MB w.FL

The profile covers the ESP32-S3 bare-metal execution path, UART0 output, I2C0
routing on GPIO3/GPIO4, and two MAX17048 registers.

- MAX17048 voltage and SOC values are fixed by configurable inputs. The
  nonlinear ModelGauge estimator, battery insertion debounce, hibernate,
  voltage history, alerts, reset/sleep behavior, and timing are not modelled.
- Unmapped MAX17048 bytes return zero; only VCELL (0x02) and SOC (0x04) are
  described and tested.
- The ESP32-S3 SYSTEM window at 0x600C0000 is `system_stub`. Firmware writes
  its I2C clock-enable and reset registers, but the twin does not model those
  bits or gate I2C0 from them. The smoke proves the I2C command/register path
  in the current engine, not clock/reset gating behavior.
- GPIO7's STEMMA QT power switch is not needed for the onboard MAX17048 and
  is outside this profile. The optional QT connector's switched power is not
  modelled.
- Wi-Fi, BLE, native USB, the charger, battery power-path behavior, analog
  inputs, buttons, NeoPixel, and general-purpose FeatherWing peripherals are
  outside the profile.
- The local smoke enters the raw ELF directly; it does not exercise the ROM,
  bootloader, Arduino/ESP-IDF startup, external antenna, or physical hardware.
- The module's 8 MiB flash is described. Flash protocol/boot behavior is not
  exercised by this smoke, and PSRAM is absent on this SKU.
