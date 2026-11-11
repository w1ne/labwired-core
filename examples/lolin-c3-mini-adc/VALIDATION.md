# Validation record

## Demand and target

ESP32 is demand rank **#1** in the JetBrains State of Embedded 2025 survey
(38%). This is ADC coverage for the already-onboarded LOLIN C3 Mini V2.1.0,
not a new board or chip variant. The chip-level SAR ADC model already existed;
this smoke makes a centered potentiometer input part of a real Arduino firmware
path.

The vendor check in [`vendor-status.md`](vendor-status.md) records WEMOS's live
V2.1.0 page and a current retailer stock listing, and Espressif's live C3
datasheet. The board uses ESP32-C3FH4, which the datasheet does not mark EOL or
NRND.

## Smoke scope

PlatformIO builds Arduino firmware for the LOLIN C3 Mini. It boots through the
ESP32-C3 mask ROM, samples GPIO0 / ADC1_CH0 using `analogRead`, and prints
`LOLIN_C3_ADC_OK` only when the sample is in range for the descriptor's centered
1.65 V potentiometer input. The smoke requires that UART marker and the expected
stop at the instruction limit.

The descriptor models only the C3 and an analog potentiometer source on ADC
channel 0. It does not claim graph-level voltage solving, Wi-Fi, Bluetooth,
USB, the onboard WS2812B, or physical pin-routing enforcement.

## Reproduce

```sh
LABWIRED_ESP32C3_FLASH=examples/lolin-c3-mini-adc/firmware/.pio/build/lolin_c3_mini/merged.bin \
  scripts/onboarding_smoke.sh \
    --target-id lolin-c3-mini-adc \
    --firmware-build-script examples/lolin-c3-mini-adc/firmware/build-firmware.sh \
    --script examples/lolin-c3-mini-adc/adc-smoke.yaml \
    --system configs/systems/lolin-c3-mini-adc.yaml \
    --out-dir examples/lolin-c3-mini-adc/evidence \
    --threshold-seconds 3600 \
    --rom-boot
```

The committed transcript, structured result, and metrics are in
[`evidence/`](evidence/). This is local digital-twin smoke evidence, not a
physical board test.

## Hosted oracle cross-check

On 2026-11-11, `labwired_verify` returned `status: model_verified` for the same
ADC sketch on the hosted catalog's **ESP32-C3 Super Mini** model, with a
potentiometer wired between 3V3/GND and GPIO0 and an oracle requiring
`LOLIN_C3_ADC_OK`. The only reported approximation was the catalog run's ideal
unpowered VDD rail, so power-on and brown-out reset are not simulated. That
catalog board is not the WEMOS carrier; the exact LOLIN system is covered by the
local smoke above, and neither result is a hardware test.

## Remaining model limits

This firmware touches no zero-answering peripheral in
`known_stubs.rs`. The C3 descriptor still has register-only `declarative`
windows with no behavioral engine for blocks including the USB Serial/JTAG
device, I2S, UHCI, crypto, and parts of the radio/control estate. This profile
does not exercise Wi-Fi, Bluetooth LE, USB, or the onboard WS2812B on GPIO7.
The potentiometer is injected into ADC channel 0; the simulator does not solve
analog circuit voltage or enforce physical wiper routing.
