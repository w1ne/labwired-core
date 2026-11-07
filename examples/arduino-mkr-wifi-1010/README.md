# Arduino MKR WiFi 1010

This profile boots ATSAMD21G18A firmware and exercises the board's onboard
yellow user LED: Arduino D6 / `LED_BUILTIN` maps to PA20. The smoke enables the
SAMD21 PORT clock through PM.APBBMASK, configures PA20 as an output and checks
for at least 16 LED edges.

## Reproduce

```bash
CARGO_BUILD_RUSTC_WRAPPER= scripts/onboarding_smoke.sh \
  --target-id arduino-mkr-wifi-1010 \
  --crate firmware-mkr-wifi-1010-smoke \
  --target thumbv6m-none-eabi \
  --script examples/arduino-mkr-wifi-1010/io-smoke.yaml \
  --system configs/systems/arduino-mkr-wifi-1010.yaml \
  --out-dir examples/arduino-mkr-wifi-1010/evidence
```

The current smoke log and simulation output are committed in `evidence/`. The
official store availability check is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model boundary

The profile includes the ATSAMD21G18A flash/RAM map, PA20, SAMD21 PM APBBMASK,
and the PORT group A GPIO model with its PORT clock gate. The Arduino pin map
and Q1 LED path follow the official [MKR WiFi 1010 pinout][pinout], [Arduino
SAMD variant][variant] and [board schematic][schematic]. The SAMD21 is at its
1 MHz reset clock because this smoke does not run the Arduino clock setup.
LabWired's `SamPm` currently resets the APBBMASK register to zero; the firmware
explicitly writes zero and then enables the PORT bit, so this smoke does not
depend on that reset value matching silicon.

The Wi-Fi/Bluetooth NINA-W102 module, the ECC508 secure element, the PMIC,
native USB, the RGB LED attached to NINA-W102, and the SAMD21's other
peripherals are omitted. This profile makes no Wi-Fi, BLE, crypto, USB, UART,
I²C, SPI, ADC, DAC, timer or interrupt claims.

[pinout]: https://content.arduino.cc/assets/Pinout-MKRwifi1010_latest.pdf
[variant]: https://github.com/arduino/ArduinoCore-samd/blob/master/variants/mkrwifi1010/variant.cpp
[schematic]: https://content.arduino.cc/assets/MKRWiFi1010V2.0_sch.pdf
