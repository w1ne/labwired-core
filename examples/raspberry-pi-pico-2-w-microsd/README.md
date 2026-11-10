# Raspberry Pi Pico 2 W + SPI microSD

This profile exercises SPI0 on the Raspberry Pi Pico 2 W (RP2350) and reads
`/README.TXT` from a modeled SDHC card. The copy-paste firmware is
[`src/main.rs`](src/main.rs); the repository's native smoke builds the same
register-level code and checks the file contents over the Pico's UART0.

## Wiring

Use the Adafruit Micro SD SPI/SDIO Card Breakout Board, PID 4682, or an
equivalent 3.3 V SPI socket:

| Breakout | Pico 2 W |
|---|---|
| 3V | 3V3(OUT) |
| GND | GND |
| SO / MISO | GP16 |
| CS | GP17 |
| CLK / SCK | GP18 |
| SI / MOSI | GP19 |

The PID 4682 breakout is 3 V-only and has no level shifter. Insert a separate
microSD card. The system uses the breakout's SPI mode; its optional SDIO mode
is outside this profile.

## Run the smoke

From the repository root:

```sh
scripts/onboarding_smoke.sh \
  --target-id raspberry-pi-pico-2-w-microsd \
  --crate firmware-pico2-w-microsd-smoke \
  --target thumbv8m.main-none-eabi \
  --script examples/raspberry-pi-pico-2-w-microsd/onboarding-smoke.yaml \
  --system configs/systems/raspberry-pi-pico-2-w-microsd.yaml \
  --firmware-build-dir crates/firmware-pico2-w-microsd-smoke \
  --out-dir examples/raspberry-pi-pico-2-w-microsd/evidence
```

The committed evidence includes compiler and simulation logs, UART output,
and the structured smoke result. Current part availability is recorded in
[`evidence/vendor-status.md`](evidence/vendor-status.md).

## Model scope

The system includes UART0, SPI0, GP16–GP19, and one microSD device. The board's
CYW43439 radio, wireless LED, USB, ADC, and other pins are omitted. The card is
a fixed 32 MiB read-only SDHC image with one small FAT16 file; it is not a
general filesystem or a custom card-image implementation. The RP2350 SPI model
delivers bytes to its single attached SPI device and does not gate those
transfers on the GP17 chip-select level. Firmware still asserts GP17 around
the card session, and the smoke checks the pin edges, but multi-device chip
select isolation is outside this profile.
