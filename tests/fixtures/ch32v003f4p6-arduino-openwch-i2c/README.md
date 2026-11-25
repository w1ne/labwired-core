# CH32V003F4P6 stock openwch Arduino `Wire` fixture

Built with the PlatformIO `ch32v` platform 1.1.0 (Community-PIO-CH32V),
`board = ch32v003f4p6_evt_r0`, `framework = arduino`, `board_build.core =
openwch`: the official WCH `arduino_core_ch32` with its bundled `Wire` library.
No source line is changed to suit the simulator.

`Wire.begin()` enables the I2C1 clock (`APB1PCENR` bit 21), programs `CKCFGR`,
and runs the EV/ER interrupt path in `twi.c`. The sketch writes the BME280
CHIP_ID register pointer (`0xD0`), reads one byte, and prints `PASS` only when
the address phase succeeds and the byte is `0x60`. The default (non-remapped)
pads are SDA = PC1 and SCL = PC2.

Rebuild:

    cd tests/fixtures/ch32v003f4p6-arduino-openwch-i2c
    pio run
    ~/.platformio/packages/toolchain-riscv/bin/riscv-wch-elf-objcopy \
        --strip-debug .pio/build/sim/firmware.elf \
        ../ch32v003f4p6-arduino-openwch-i2c.elf

Test:
`crates/core/tests/vendor/ch32v003_compiled_firmware.rs`
(`ch32v003_openwch_wire_reads_a_bme280_chip_id_over_i2c1`).
