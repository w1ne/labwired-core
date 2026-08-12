# MAX31855 Thermocouple Lab

This firmware reads a MAX31855 cold-junction-compensated K-type thermocouple
converter over LabWired's simulated SPI1 path on STM32F103. Each cycle clocks
out the 32-bit MAX31855 frame and decodes the 14-bit thermocouple temperature
(Q4 °C), the 12-bit internal/cold-junction temperature, and the fault bits.

Run from the repo root:

```bash
cargo build -p max31855-thermocouple-lab --release --target thumbv7m-none-eabi
cargo run -q -p labwired-cli -- test --script examples/max31855-thermocouple-lab/io-smoke.yaml
```

Expected UART begins with:

```text
MAX31855 Thermocouple Lab
word=0x... TC_q4=... INT_q12=... FAULT=...
```

The thermocouple model attaches over SPI1 with chip-select on `PA4`
(`device_type: max31855` in `system.yaml`).

## Optional: edge-accurate (bit-level) slave sampling

By default an attached SPI device is consulted once per frame, at the frame
boundary, and never sees a clock edge — so a controller programmed for the
wrong CPOL/CPHA still exchanges perfectly good bytes. Add `spi_mode` to the
device's `config:` to strap the part for edge-accurate sampling instead:

```yaml
external_devices:
  - id: "tc1"
    type: "max31855"
    connection: "spi1"
    config:
      cs_pin: "PA4"
      spi_mode: 1   # the mode THIS part is strapped for (0..=3)
```

With `spi_mode` present the bit engine latches MOSI into the device, and clocks
MISO out of it, on the physical SCK edges that mode selects. Leave the firmware
in mode 0 against `spi_mode: 1` and the 32-bit frame comes back shifted one bit
(`0x01901600` → `0x00C80B00`, i.e. 25.0 °C reads as 12.5 °C) — the same
symptom the mismatch produces on real hardware. Omit `spi_mode` and nothing
about the lab changes: it is the byte-level path, unmodified.

Only a controller that models the clock mode can honour it: the STM32
classic/FIFO SPI and the ESP32-C3 GP-SPI. Asking for `spi_mode` on a controller
that exchanges whole bytes (ESP32 classic, ESP32-S3, nRF52 SPIM, STM32H5 SPIv3,
Kinetis DSPI) fails at config time with a message naming that controller, rather
than silently doing nothing.
