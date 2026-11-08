# ESP32-H2-DevKitM-1-N4S

**Demand rank:** ESP32 family, rank 1 (38% MCU use; 42% ESP32-kit use in the
JetBrains State of Embedded 2025 figures recorded in the demand-ordered
onboarding brief). This is H2 family coverage, not maintenance work on an
already modeled die.

## Modelled slice

The smoke runs a direct-entry RV32IMC image and bit-bangs one red frame to the
board's addressable RGB LED on GPIO8. Its assertions require at least 48 GPIO8
edges, a decoded red pixel, and the expected max-step stop. The chip descriptor
exposes only the HP core, its flash/RAM ranges, and the GPIO block used by this
smoke.

The H2 GPIO model alias reuses the existing Espressif GPIO implementation for
the H2 SVD's `GPIO` base `0x60091000` and the matched `OUT`, `OUT_W1TS`,
`OUT_W1TC`, `ENABLE`, `ENABLE_W1TS`, and `IN` register offsets. The smoke
exercises the direct GPIO output path on GPIO8; it does not claim H2-specific
matrix, interrupt, or other-pin behavior. Sources: [Espressif H2 SVD](https://github.com/espressif/svd/releases/tag/be20aa1),
[ESP32-H2 datasheet](https://documentation.espressif.com/esp32-h2_datasheet_en.html),
and [board guide](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32h2/esp32-h2-devkitm-1/user_guide.html).

## Availability

The profile targets **ESP32-H2-DevKitM-1-N4S**. Espressif's current guide lists
the 4 MiB N4S variant and marks the older N4 variant NRND; the exact N4S
DigiKey listing reports Active and in-stock. The live check is recorded in
[`examples/esp32h2-devkitm-1-n4s/evidence/vendor-status.md`](../../examples/esp32h2-devkitm-1-n4s/evidence/vendor-status.md).

## Run

```bash
cargo build -p firmware-esp32h2-demo --release --target riscv32imc-unknown-none-elf
cargo run -q -p labwired-cli -- test \
  --script examples/esp32h2-devkitm-1-n4s/io-smoke.yaml \
  --output-dir out/esp32h2-devkitm-1-n4s/io-smoke \
  --no-uart-stdout
```

## Not covered

H2 mask-ROM boot, UART0 and the USB-UART bridge, native USB, Bluetooth LE,
802.15.4/Thread/Zigbee, RMT, timers, interrupts, analog behavior, power, and
electrical tolerances remain outside this profile. See the example's
[`KNOWN_LIMITATIONS.md`](../../examples/esp32h2-devkitm-1-n4s/KNOWN_LIMITATIONS.md).
