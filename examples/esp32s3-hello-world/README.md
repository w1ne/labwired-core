# esp32s3-hello-world

Canonical esp-hal hello-world for the LabWired ESP32-S3 simulator (Plan 2).

## Build

Requires the ESP Rust toolchain. Install via [`espup`](https://docs.esp-rs.org/book/installation/index.html):

```sh
cargo install espup
espup install
. ~/export-esp.sh   # exports PATH + LIBCLANG_PATH
```

Build:

```sh
cd examples/esp32s3-hello-world
cargo +esp build --release
```

The resulting ELF is at `target/xtensa-esp32s3-none-elf/release/esp32s3-hello-world`.

## Run in the simulator

From the workspace root:

```sh
cargo run -p labwired-cli -- run \
    --chip configs/chips/esp32s3-zero.yaml \
    --firmware examples/esp32s3-hello-world/target/xtensa-esp32s3-none-elf/release/esp32s3-hello-world
```

The simulator should print `Hello world!` to stdout once per second.

## Run on real hardware

With the ESP32-S3-Zero connected via USB:

```sh
cd examples/esp32s3-hello-world
cargo +esp run --release
# … in another terminal:
cat /dev/ttyACM0
```

Output should be identical to the simulator's.
