# Adafruit PyBadge example

Bare-register io-smoke for the PyBadge (ATSAMD51J19A) twin.
Source: `crates/firmware-atsamd51-pybadge-demo`; the committed build is
`tests/fixtures/atsamd51-pybadge-smoke.elf`.

The image is linked at `0x4000` behind the resident UF2 bootloader and sets
`SCB.VTOR` to its own vector table. It enables SERCOM1 (MCLK `APBAMASK` bit 13,
GCLK `PCHCTRL[8]`), prints `OK\n`, then toggles the red D13 LED on PA23.

## Run

```bash
cargo build -p firmware-atsamd51-pybadge-demo --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test --script examples/pybadge/io-smoke.yaml --no-uart-stdout
```

The machine-run test is `crates/core/tests/pybadge_firmware.rs`.

## Mixed-signal coverage

`crates/core/tests/cosim_pybadge_analog.rs` reuses the generic Beta NTC divider and
Shockley diode netlists from the Metro M4 analog example. It routes them through the
PyBadge's A0/PA02 and A2/PB08 pads, enables each SAMD51 peripheral mux, and asserts the
solved voltages and ADC0 results. The test also checks the board descriptor's A0–A5
pad-to-channel mappings.

## Sources

- LED: Adafruit ArduinoCore-samd `variants/pybadge_m4/variant.cpp`
  (pin 13 "LED" = PORTA 23) and `variant.h` (`LED_BUILTIN` = 13).
- 0x4000 application start and CF2 pin table: Microsoft uf2-samdx1
  `boards/arcade_pybadge/board_config.h`.
- Register layout: SAM D5x/E5x datasheet DS60001507.

## Known limits

- The console is SERCOM1 on PA16/PA17 as wired in `configs/systems/pybadge.yaml`.
  Adafruit's Arduino variant puts Serial1 on SERCOM5 (PB16/PB17); the twin
  descriptor does not model that mapping.
- Display (ST7735 over SERCOM4 SPI), button shift register, NeoPixels' timing
  against firmware, QSPI and USB are not exercised here.
