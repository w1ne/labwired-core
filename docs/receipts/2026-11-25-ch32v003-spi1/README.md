# CH32V003F4P6: SPI1 master, proven by the stock openwch `SPI` library

**Demand rank:** Tier C vendor bet (WCH), not a measured survey pick. The
JetBrains State of Embedded 2025 ranking does not list WCH; the standing brief
bets on CH32V003 for cheap RISC-V hobby momentum. This round adds the second of
the part's three serial blocks (USART1 and I2C1 already boot; SPI1 did not
exist in the descriptor). It is depth on an already-modelled part, not a new
part or a new vendor.

**Vendor status, read live 2026-11-25:**

- `https://www.wch-ic.com/products/CH32V003.html` returns HTTP 200 for the
  current product page. WCH publishes no lifecycle field for this part, so
  there is no NRND/EOL notice to read — and none is claimed here.
- `https://www.lcsc.com/product-detail/C5187096.html` returns HTTP 200 and
  describes `WCH CH32V003F4P6`, TSSOP-20, 48 MHz, RoHS. The same page reports
  `stockNumber: 0` and `availability: OutOfStock` on 2026-11-25 for that SKU.
  The order code is listed and buyable in general, but this exact LCSC SKU had
  no stock on the day of the check. That is a stock fact, not an obsolescence
  finding.
- The stock firmware is built from the official `openwch/arduino_core_ch32`
  core, which still ships the CH32V003F4 variant and its `SPI` library.

## What boots

`tests/fixtures/ch32v003f4p6-arduino-openwch-spi.elf` is the official openwch
`SPI` library, unmodified, on the CH32V003F4P6:

- `SPI.begin()` clocks SPI1 (`RCC_SPI1EN`, APB2PCENR bit 12) and programs
  `CTLR1` for master mode on the variant's default pads SCK = PC5, MOSI = PC6,
  MISO = PC7;
- `SPI.transfer()` clocks four dummy bytes out while the GPIO chip select
  (PC4) is low, and reads the MAX31855's 32-bit frame back off MISO;
- the sketch prints the raw frame and both junction temperatures in
  millidegrees, then `CH32V003_SPI_PASS`.

With the device model's defaults (25.00 degC hot, 22.00 degC cold, fault-free)
the frame is `0x01901600`, i.e. `tc_milli=25000`, `cj_milli=22000`. The exact
USART1 byte stream is `uart.log` in this directory.

The shipped board profile is `configs/systems/wch-ch32v003evt-max31855.yaml`
(CH32V003EVT + Adafruit MAX31855 breakout). The positive test loads that file,
not an inline manifest.

## Negative control

The same firmware with no device on SPI1 completes its four frames (the model
answers `0x00`), prints `raw=0x0` and `CH32V003_SPI_FAIL`. The pass token
therefore requires a real slave answer, not merely a transfer that returned.

## Evidence

    cargo test -p labwired-core --test vendor \
        'ch32v003_compiled_firmware::' -- --test-threads=1
    # 36 passed; 0 failed
    # full output: cargo-test.log

Rebuild of the guest:

    cd tests/fixtures/ch32v003f4p6-arduino-openwch-spi
    pio run
    ~/.platformio/packages/toolchain-riscv/bin/riscv-wch-elf-objcopy \
        --strip-debug .pio/build/sim/firmware.elf \
        ../ch32v003f4p6-arduino-openwch-spi.elf

## Coverage and limits

Modelled: SPI1 at `0x40013000` (APB2PCENR bit 12, `SPI1_IRQn` 33) on the STM32
F1 register layout: `CTLR1`, `CTLR2`, `STATR`, `DATAR`, plus the CRCR/RCRCR/
TCRCR register file. Master mode 0-3, 8-bit and 16-bit frames, MSB/LSB first,
baud prescaler, the TXE/RXNE/BSY/OVR polled path, and GPIO chip-select edges.

Not modelled, and not claimed by the descriptor:

- the AFIO SPI1 remap (`AFIO_PCFR1` bit 0); only the default PC5/PC6/PC7 pads
  are wired;
- hardware NSS output (`CTLR2.SSOE`); firmware must use a GPIO chip select;
- the CRC engine (`CTLR1.CRCEN/CRCNEXT`, `CRCR`/`RCRCR`/`TCRCR` arithmetic);
- the CH32-only `HSCR` high-speed register at offset `0x24`;
- the MAX31855's conversion time and analog front end; the device model is the
  existing declarative `max31855` kit, which reports temperature directly.

The SPI1 block is declared on all four CH32V003 package descriptors because it
is one die. SPI pin functions are claimed only on the F4P6 (TSSOP20) and F4U6
(QFN20), which bond PC5/PC6/PC7; the A4M6 (SOP16) has no PC5 pad and the J4M6
(SOP8) bonds none of the three.
