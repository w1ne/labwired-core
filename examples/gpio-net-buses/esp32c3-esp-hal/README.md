# esp-hal 1.x masters for the gpio-net-buses example

Two ESP32-C3 applications on esp-hal 1.1 (`esp-hal 1.1.1`, `esp-println
0.17`, `esp-backtrace 0.19`, the lock file of `examples/esp32c3-hello-world`),
used by `crates/core/tests/world_gpio_net_buses.rs` to run esp-hal's own
blocking SPI and I2C master drivers against an STM32G071 over gpio nets:

| Binary | Committed ELF | Environment |
|--------|---------------|-------------|
| `spi_master` | `../firmware/esp32c3-esp-hal-spi-master.elf` | `env-spi-esp32c3.yaml` with this firmware |
| `i2c_master` | `../firmware/esp32c3-esp-hal-i2c-master.elf` | `env-i2c-esp32c3.yaml` with this firmware |

Built offline with the workspace's pinned stable toolchain (1.95.0) for
`riscv32imc-unknown-none-elf` (no `build-std`, no `esp` toolchain):

```sh
cd examples/gpio-net-buses/esp32c3-esp-hal
cargo +1.95.0 build --release --offline
for b in spi_master i2c_master; do
    llvm-objcopy --strip-all target/riscv32imc-unknown-none-elf/release/$b \
        ../firmware/esp32c3-esp-hal-${b//_/-}.elf
done
```

The pads are routed through the GPIO matrix: none of them has an IO_MUX
function for its signal, so esp-hal picks the matrix as well (it uses an
IO_MUX function when the pad has one, which the simulator does not follow).
