# ST NUCLEO-G491RE quick start

## Demand rank

The STM32G491RE falls under MCU demand rank **#4**, “other STM32” (13%). The
Nucleo/Discovery kit class is also rank **#4** among kits (23%). This is a new,
buyable G4 board on top of the existing G4 simulator support.

## Copy-paste firmware example

The no-std Rust smoke blinks the board's green LD2 LED on PA5. Build and run it
from the repository root:

```bash
cargo build -p firmware-nucleo-g491re-smoke --release --target thumbv7em-none-eabi
cargo run -q -p labwired-cli -- test \
  --script examples/nucleo-g491re/io-smoke.yaml \
  --output-dir out/nucleo-g491re/io-smoke --no-uart-stdout
```

The smoke requires at least eight observed PA5 transitions. This base system
uses the G4 RCC, GPIOA/GPIOC, LD2 and B1 mapping. The chip descriptor also
declares GPIOB/I2C1 for the separate [BME280 add-on profile](../nucleo-g491re-bme280/);
USB, ST-LINK and analog behavior remain outside these profiles.

## Files

- `system.yaml`: standalone board wiring mirror.
- `io-smoke.yaml`: reproducible PA5 transition smoke.
- `vendor-status.md`: live vendor status recorded for this onboarding.
- `VALIDATION.md`: committed simulator evidence and scope limits.
