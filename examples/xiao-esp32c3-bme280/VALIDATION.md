# Validation record

## Demand and availability

ESP32 is demand rank **#1** in the JetBrains State of Embedded 2025 survey
(38%). This change adds a board profile for the Seeed XIAO ESP32-C3, using the
existing ESP32-C3 model. It does not add a new chip model. The exact
ESP32-C3FH4 and board availability check is recorded in
[`vendor-status.md`](vendor-status.md).

## Smoke scope

PlatformIO builds the exact `seeed_xiao_esp32c3` Arduino target and merges its
bootloader, partition table, and app at the actual flash offsets. LabWired then
boots the merged image from the C3 reset vector. Firmware reads BME280 ID
register `0xD0` and requires `0x60`, reads temperature registers `0xFA` through
`0xFC` and requires raw value `0x082DC0`, then toggles D10/GPIO10. The smoke
requires at least four D10 edges and a stop at the configured instruction
limit. If either I²C result is missing or different, the proof pin remains low.

The descriptor wires the modeled BME280 to I²C0 on D4/SDA GPIO6 and D5/SCL
GPIO7. It does not claim USB Serial/JTAG, Wi-Fi, Bluetooth LE, battery charging,
or the power-only charge indicator. The smoke result also records that the
simulator assumes an ideal VDD rail; power sequencing and brown-out reset are
not simulated. This evidence is from the local digital twin, not a physical
board test.

## Reproduce

    LABWIRED_ESP32C3_FLASH=examples/xiao-esp32c3-bme280/firmware/.pio/build/xiao_c3/merged.bin \
      scripts/onboarding_smoke.sh \
        --target-id xiao-esp32c3-bme280 \
        --firmware-build-script examples/xiao-esp32c3-bme280/firmware/build-firmware.sh \
        --script examples/xiao-esp32c3-bme280/bme280-smoke.yaml \
        --system configs/systems/xiao-esp32c3-bme280.yaml \
        --out-dir examples/xiao-esp32c3-bme280/evidence \
        --rom-boot

The committed logs, structured result, and metrics are in `evidence/`.
