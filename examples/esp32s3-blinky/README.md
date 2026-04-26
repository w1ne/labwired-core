# esp32s3-blinky

ESP32-S3 blinky demo for the LabWired simulator (Plan 3).

Toggles GPIO2 from a SYSTIMER alarm ISR every 500 ms. Runs identically on the simulator and on a connected ESP32-S3-Zero.

## Build

Requires the ESP Rust toolchain (see `examples/esp32s3-hello-world/README.md` for setup).

```sh
cd examples/esp32s3-blinky
cargo +esp build --release
```

The resulting ELF is at `target/xtensa-esp32s3-none-elf/release/esp32s3-blinky`.

## Run in the simulator

From the workspace root:

```sh
cargo run -p labwired-cli --release -- run \
    --chip configs/chips/esp32s3-zero.yaml \
    --firmware examples/esp32s3-blinky/target/xtensa-esp32s3-none-elf/release/esp32s3-blinky \
    --gpio-trace /tmp/blinky.jsonl
```

The simulator emits a `tracing::info!` event each time GPIO2 toggles. With `--gpio-trace`, every transition is also written to the JSONL file.

## Run on real hardware

With the ESP32-S3-Zero connected via USB:

```sh
cd examples/esp32s3-blinky
cargo +esp run --release
# … probe GPIO2 with a logic analyzer or multimeter to see the toggle.
```
