# Validation record

## Demand and scope

ESP32 is demand rank **#1** in the 2025 embedded MCU survey (38%). This round
adds a smoke for the currently listed ESP32-C3-DevKitM-1-N4X board and its
BME280 I²C sensor. The C3 core and I²C model already existed; this is a
board/system coverage addition, not a new chip core.

The system exercises UART0 and reads BME280 over I²C0 using header GPIO4/5.
The descriptor omits the board's addressable RGB LED, Wi-Fi, Bluetooth, USB,
and buttons because this smoke does not model/exercise those functions.

## Reproduce

```sh
LABWIRED_ESP32C3_FLASH=examples/esp32c3-devkitm-1-n4x-bme280/firmware/.pio/build/devkitm1_n4x/merged.bin \
  scripts/onboarding_smoke.sh \
    --target-id esp32c3-devkitm-1-n4x-bme280 \
    --firmware-build-script examples/esp32c3-devkitm-1-n4x-bme280/firmware/build-firmware.sh \
    --script examples/esp32c3-devkitm-1-n4x-bme280/bme280-smoke.yaml \
    --system configs/systems/esp32c3-devkitm-1-n4x-bme280.yaml \
    --out-dir examples/esp32c3-devkitm-1-n4x-bme280/evidence \
    --threshold-seconds 3600 \
    --rom-boot
```

The committed transcript, structured result, and metrics are under
[`evidence/`](evidence/). GPIO4/5 I²C routing is based on the board header and
the LabWired C3 matrix model; the assignment is not silicon-compared.

## Claim boundary

The smoke passes when firmware reads BME280 `CHIP_ID=0x60`, obtains raw
temperature `0x082DC0` at the model's default 25 °C, emits its UART0 marker,
then stops at the configured instruction limit in the idle loop. This is local
digital-twin evidence using the C3 mask-ROM boot path with the merged
bootloader, partition table, and application image. It is not a physical board
test. The board guide lists the N4X variant and its retailer links; no purchase
or silicon test is claimed.
