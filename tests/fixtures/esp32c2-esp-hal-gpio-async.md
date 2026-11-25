# ESP32-C2 Embassy async GPIO (esp-rtos) fixture

`esp32c2-esp-hal-gpio-async.elf` is the `examples/esp32c2/esp-hal-gpio-async`
crate (esp-hal 1.2.2, esp-rtos 0.4.0 with `embassy`, embassy-executor
0.10.0, embassy-time 0.5.1, embassy-sync 0.8.0, esp-println 0.17 on UART0,
esp-backtrace; versions pinned by its `Cargo.lock`), used by
`crates/core/tests/vendor/esp32c2_compiled_firmware.rs` and
`examples/esp32c2/esp-hal-gpio-async-smoke.yaml`.

What it exercises on the bare-ELF fast boot:

* GPIO2 as a `Flex` output, split with `split_into_drivers` into an `Input`
  (main) and an `Output` (a spawned task) on the same pad.
* `Input::wait_for_rising_edge`, woken by the esp-hal default GPIO handler
  (matrix source 13) when the task drives the pad high 1 ms after each
  request. Three requests; main prints each wait.
* `with_timeout(2 ms)` on `wait_for_rising_edge` with no pulse requested:
  the timeout must fire.

Expected UART0 output (measured 2026-11-16, vendor test, `Machine::step`):

```
LW-C2 gpio-async boot
LW-C2 edge wait_us 1033
LW-C2 edge wait_us 1035
LW-C2 edge wait_us 1035
LW-C2 rising edges 3 min_wait_us 1033
LW-C2 idle wait timed out true
LW-C2 gpio-async PASS
```

With the interrupt matrix not armed, no edge wait ends and the app prints
only the boot line (the vendor negative control).

Known divergence: `labwired test` with idle fast-forward on (the default)
prints waits of 1287, 4968 and 6347 us. The timer-poll coalesce
(`take_timer_poll_coalesce_eligible`) counts the four SYSTIMER snapshot
accesses of one `Instant::now()` as a busy-poll and moves time to the next
scheduler event (here a UART0 byte drain, about 87 us away). With
`LABWIRED_IDLE_FAST_FORWARD=0` the CLI prints the values above.
(Before the C2 clock fix the core ran on C3 timing and the waits read
1024/1026/1026 us; the 9 us more is the same instruction count at 120 MHz.) The CLI
smoke checks only the verdict, which holds in both modes; the vendor test
checks the per-wait timing.

Built with the pinned stable toolchain for `riscv32imc-unknown-none-elf`:

```sh
cd examples/esp32c2/esp-hal-gpio-async
cargo +1.95.0 build --release
riscv32-esp-elf-objcopy --strip-all \
    target/riscv32imc-unknown-none-elf/release/esp32c2-esp-hal-gpio-async \
    ../../../tests/fixtures/esp32c2-esp-hal-gpio-async.elf
```

Fixture sha256: `4eaafc01cb4b4e906834d93cb071d15672e95185ea12afa96d8c679cca37663e`
(two builds from different target directories gave the same hash).
