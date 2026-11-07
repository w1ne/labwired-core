# Adafruit PyBadge example

Bare-register io-smoke for the PyBadge (ATSAMD51J19A) twin.
Source: `crates/firmware-atsamd51-pybadge-demo`; the committed build is
`tests/fixtures/atsamd51-pybadge-smoke.elf`.

The image is linked at `0x4000` behind the resident UF2 bootloader and sets
`SCB.VTOR` to its own vector table. It enables SERCOM1 (MCLK `APBAMASK` bit 13,
GCLK `PCHCTRL[8]`), prints `OK\n`, then toggles the red D13 LED on PA23.

The onboarding smoke rebuilds that firmware, asserts both the SERCOM1 output
and at least four PA23 transitions, and commits the stage logs and simulator
output under [`evidence/`](evidence/). The live vendor check is in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Run

```bash
cargo build -p firmware-atsamd51-pybadge-demo --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test --script examples/pybadge/io-smoke.yaml --no-uart-stdout
```

To reproduce the full onboarding evidence from the repository root:

```bash
scripts/onboarding_smoke.sh \
  --target-id adafruit-pybadge \
  --crate firmware-atsamd51-pybadge-demo \
  --target thumbv7em-none-eabi \
  --script examples/pybadge/onboarding-smoke.yaml \
  --system examples/pybadge/system.yaml \
  --out-dir examples/pybadge/evidence
python3 scripts/generate_onboarding_scoreboard.py \
  --metrics-root examples/pybadge/evidence \
  --markdown-out examples/pybadge/evidence/onboarding-scoreboard.md \
  --json-out examples/pybadge/evidence/onboarding-scoreboard.json
```

The machine-run test is `crates/core/tests/pybadge_firmware.rs`.

## Sources

- LED: Adafruit ArduinoCore-samd `variants/pybadge_m4/variant.cpp`
  (pin 13 "LED" = PORTA 23) and `variant.h` (`LED_BUILTIN` = 13).
- 0x4000 application start and CF2 pin table: Microsoft uf2-samdx1
  `boards/arcade_pybadge/board_config.h`.
- Register layout: SAM D5x/E5x datasheet DS60001507.
- Product and MCU availability: Adafruit product 4200 and Microchip
  ATSAMD51J19A live status, recorded in `evidence/vendor-status.md`.

## Known limits

- The console is SERCOM1 on PA16/PA17 as wired in `configs/systems/pybadge.yaml`.
  Adafruit's Arduino variant puts Serial1 on SERCOM5 (PB16/PB17); the twin
  descriptor does not model that mapping.
- Display (ST7735 over SERCOM4 SPI), button shift register, NeoPixels' timing
  against firmware, QSPI and USB are not exercised here.
- The acceptance smoke checks GPIO edges on PA23 directly; UART `OK` is an
  additional boot marker. It does not claim display, button, NeoPixel, QSPI,
  USB CDC or silicon behavior.
