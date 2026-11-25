# CH32V003F4P6 openwch Arduino `SPI` fixture

Built exactly as `../ch32v003f4p6-arduino-openwch/` (PlatformIO `ch32v`
platform 1.1.0, `board = ch32v003f4p6_evt_r0`, `framework = arduino`,
`board_build.core = openwch`, `framework-arduino-openwch-ch32` at
`3820604346d0a9f0ee9a72cab8b7583e488e6674`). The sketch is not modified for
the simulator.

What the sketch exercises:

- `SPI.begin()` and `SPI.transfer()` from the official openwch `SPI` library.
  The core's `spi_init()` enables `RCC_SPI1EN` (APB2PCENR bit 12), programs
  `CTLR1` (`SPE | MSTR | CPOL/CPHA | BR`), writes `CRCR`, and its
  `spi_transfer()` polls `STATR.TXE`, `STATR.RXNE` and `STATR.BSY`;
- the default, non-remapped SPI1 pads from the `CH32V003F4` variant:
  SCK = PC5, MOSI = PC6, MISO = PC7, with a GPIO chip select on PC4;
- a MAX31855 K-type thermocouple converter: four dummy bytes clocked while CS
  is low return its read-only 32-bit frame. The sketch decodes the hot
  junction (`[31:18]`, 0.25 degC/LSB) and the cold junction (`[15:4]`,
  0.0625 degC/LSB) as integer millidegrees.

The sketch prints `raw=0x<hex>`, `tc_milli=<n>`, `cj_milli=<n>`, then
`CH32V003_SPI_PASS` only when the frame is neither all-zero nor all-ones and
the fault bit (`[16]`) is clear.

With the simulator's device defaults (25.00 degC hot, 22.00 degC cold,
fault-free) the UART stream is
`docs/receipts/2026-11-25-ch32v003-spi1/uart.log`.

Rebuild:

    cd tests/fixtures/ch32v003f4p6-arduino-openwch-spi
    pio run
    ~/.platformio/packages/toolchain-riscv/bin/riscv-wch-elf-objcopy \
        --strip-debug .pio/build/sim/firmware.elf \
        ../ch32v003f4p6-arduino-openwch-spi.elf

Test: `crates/core/tests/vendor/ch32v003_compiled_firmware.rs`
(`ch32v003_openwch_spi_reads_a_max31855_frame_over_spi1`, the no-slave
negative control `ch32v003_spi_transfer_without_a_slave_reports_no_frame`,
and the package/pad declaration check).
