# ESP32-C5-DevKitC-1-N8R8 GPIO5 smoke

This example boots a small bare-metal RV32IMAC firmware image on the modeled
ESP32-C5 HP core, selects GPIO5, and toggles it through the GPIO output
registers. GPIO5 is available at J3 pin 9 on the current ESP32-C5-DevKitC-1
v1.2 board.

This is a focused new-chip smoke, not a full C5 system model. The ELF is loaded
directly into HP SRAM by the simulator; it does not use the C5 mask ROM, flash
boot path, or PSRAM. The smoke observes GPIO model edges. It does not claim the
IOMUX routing is electrically enforced or prove physical hardware behavior.

## Build and run

```sh
rustup target add riscv32imac-unknown-none-elf
cargo build -p firmware-esp32c5-devkitc-1-gpio-smoke \
  --release --target riscv32imac-unknown-none-elf
cargo run -q -p labwired-cli -- test \
  --script examples/esp32c5-devkitc-1-gpio-smoke/io-smoke.yaml \
  --output-dir out/esp32c5-devkitc-1-n8r8-gpio --no-uart-stdout
```

The assertion requires four or more transitions on `gpio:5`. For a physical
bench check, connect J3 pin 9 (GPIO5) through a current-limiting resistor to an
LED and ground. The board's onboard addressable RGB LED is on GPIO27 and is
not part of this smoke.

See [`VALIDATION.md`](VALIDATION.md) for the claim boundary and committed
evidence, and [`vendor-status.md`](vendor-status.md) for the current buyability
check.
