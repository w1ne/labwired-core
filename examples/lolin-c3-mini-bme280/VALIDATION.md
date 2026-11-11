# Validation record

## Demand and target

ESP32 is demand rank **#1** in the JetBrains State of Embedded 2025 survey
(38%). This round adds an exact LOLIN C3 Mini V2.1.0 system and a BME280 I2C
smoke. The ESP32-C3 core and I2C model already existed; this is incremental
board coverage.

WEMOS's live page identifies ESP32-C3FH4 and links to a current purchase
listing. Espressif's current datasheet does not mark C3FH4 EOL or NRND. See
vendor-status.md for the dated URLs and qualification.

## Smoke scope

The PlatformIO firmware boots through the ESP32-C3 mask ROM, reads BME280 ID
register 0xD0 and requires 0x60, then reads temperature registers 0xFA-0xFC and
requires raw value 0x082DC0. Only after both checks does it toggle exposed
GPIO6. The smoke requires at least eight GPIO6 edges and a stop at the
configured instruction limit.

The descriptor includes only this I2C sensor and the C3 model's chip-level
peripherals. It does not claim the onboard WS2812B, USB CDC, Wi-Fi, Bluetooth
LE, or physical-board behavior.

## Reproduce

    LABWIRED_ESP32C3_FLASH=examples/lolin-c3-mini-bme280/firmware/.pio/build/lolin_c3_mini/merged.bin \
      scripts/onboarding_smoke.sh \
        --target-id lolin-c3-mini-bme280 \
        --firmware-build-script examples/lolin-c3-mini-bme280/firmware/build-firmware.sh \
        --script examples/lolin-c3-mini-bme280/bme280-smoke.yaml \
        --system configs/systems/lolin-c3-mini-bme280.yaml \
        --out-dir examples/lolin-c3-mini-bme280/evidence \
        --threshold-seconds 3600 \
        --rom-boot

The committed transcript, structured result, and metrics are in evidence/.
This is local digital-twin smoke evidence, not a physical board test.
