# ESP32-C2 Embassy (esp-rtos) fixture

`esp32c2-esp-hal-embassy.elf` is the `examples/esp32c2/esp-hal-embassy`
crate (esp-hal 1.2.2, esp-rtos 0.4.0 with `embassy`, embassy-executor
0.10.0, embassy-time 0.5.1, embassy-sync 0.8.0, esp-println 0.17 on UART0,
esp-backtrace; versions pinned by its `Cargo.lock`), used by
`crates/core/tests/vendor/esp32c2_compiled_firmware.rs` and
`examples/esp32c2/esp-hal-embassy-smoke.yaml`.

What it exercises on the bare-ELF fast boot:

* `#[esp_rtos::main]` async entry; `esp_rtos::start` with SYSTIMER alarm 0
  as the Embassy time driver and `FROM_CPU_INTR0` for task switches.
* A spawned `#[embassy_executor::task]` that awaits three 1 ms
  `Timer::after` delays and sends the count through an embassy-sync
  `Signal`.
* `Instant::elapsed` across the three timers.
* `with_timeout(1 ms)` on a `Signal` nobody sends: the timeout must fire.

Expected UART0 output (measured 2026-11-16, `labwired test` and the
vendor test):

```
LW-C2 embassy boot
LW-C2 ticks 3 elapsed_us 3029
LW-C2 timeout fired true
LW-C2 embassy PASS
```

With the interrupt matrix not armed, neither the SYSTIMER alarm nor the
FROM_CPU doorbell reaches a CPU line, and the app prints only the boot
line (the vendor negative control).

Built with the pinned stable toolchain for `riscv32imc-unknown-none-elf`:

```sh
cd examples/esp32c2/esp-hal-embassy
cargo +1.95.0 build --release
riscv32-esp-elf-objcopy --strip-all \
    target/riscv32imc-unknown-none-elf/release/esp32c2-esp-hal-embassy \
    ../../../tests/fixtures/esp32c2-esp-hal-embassy.elf
```
