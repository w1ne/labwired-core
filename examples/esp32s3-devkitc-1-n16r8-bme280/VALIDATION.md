# Validation record

## Scope

Demand rank: **ESP32 #1 (38%)**. This is a newly represented board system for
an already-modelled ESP32-S3, with an I2C BME280 read as the acceptance behavior.

## Reproduce

```sh
examples/esp32s3-devkitc-1-n16r8-bme280/build-firmware.sh
cargo run -q -p labwired-cli -- test \
  --script examples/esp32s3-devkitc-1-n16r8-bme280/bme280-smoke.yaml \
  --output-dir out/esp32s3-devkitc-1-n16r8-bme280 \
  --no-uart-stdout
```

## Result

The committed evidence is under `evidence/`. The smoke observes
`CHIP_ID=0x60`, raw temperature `0x082DC0`, and the `max_steps` stop after the
firmware reaches its idle loop. The boot log also records the configured
16 MiB flash capacity. This is digital-twin evidence; it does not confirm a
physical board or its electrical I2C bus.

The generic result footprint reports 585,681 bytes of static RAM against the
chip catalog's 524,288-byte total. The PlatformIO linker summary reports 23,024
bytes of internal RAM used (7% of 327,680 bytes). The ELF also contains linker
dummy/reservation sections (`.dram0.dummy`, `.ext_ram.dummy`, and
`.flash_rodata_dummy`) that the generic footprint total includes, so treat that
aggregate as an analyzer caveat rather than a reliable runtime RAM-occupancy
measurement.
