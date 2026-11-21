# ESP32-C2 Embassy async UART (esp-rtos) fixture

`esp32c2-esp-hal-uart-async.elf` is the `examples/esp32c2/esp-hal-uart-async`
crate (esp-hal 1.2.2, esp-rtos 0.4.0 with `embassy`, embassy-executor
0.10.0, embassy-time 0.5.1, embassy-sync 0.8.0, esp-println 0.17 on UART0,
esp-backtrace; versions pinned by its `Cargo.lock`), used by
`crates/core/tests/vendor/esp32c2_compiled_firmware.rs` and
`examples/esp32c2/esp-hal-uart-async-smoke.yaml`.

What it exercises on the bare-ELF fast boot:

* `Uart::new(UART1)` at 115200 baud. esp-hal writes `UART_ID.REG_UPDATE`
  (bit 31) and spins until hardware clears it; the model clears it on the
  write.
* UART1 in loopback (`UART_CONF0.LOOPBACK`, bit 14, set with a raw register
  write because esp-hal has no API for it): each byte that leaves the TX
  shift register lands in the UART1 RX FIFO.
* `split()` into `UartRx`/`UartTx`. A spawned task waits 1 ms after each
  request, then sends a 16-byte printable frame with `write_async` and
  `flush_async`. main awaits `read_exact_async`, woken by the esp-hal UART1
  async handler (matrix source 18), checks the bytes and prints the wait.
  Three frames.
* `with_timeout(2 ms)` on `read_async` with nothing sent: the timeout must
  fire.
* The verdict needs every wait to be at least 1 ms plus 16 frames of 10
  bits at 115200 baud (1389 us).

Expected UART0 output (measured 2026-11-16, vendor test, `Machine::step`).
The capture also holds UART1, so the 16 loopback bytes sit in front of each
`frame` line:

```
LW-C2 uart-async boot
adgjmpsvybehknqtLW-C2 frame 0 wait_us 2430 match true
filoruxadgjmpsvyLW-C2 frame 1 wait_us 2428 match true
knqtwzcfiloruxadLW-C2 frame 2 wait_us 2429 match true
LW-C2 frames 3 min_wait_us 2428
LW-C2 idle read timed out true
LW-C2 uart-async PASS
```

With the interrupt matrix not armed, the first 5 ms settle timer never
completes and the app prints only the boot line (the vendor negative
control).

Model changes this fixture needed (all in the shared `EspUart` /
`Systimer`, not C2-specific code):

* `UART_ID.REG_UPDATE` self-clears (S3/C3/C2 register map). Before it,
  `Uart::new(UART1)` never returned.
* `UART_CONF0.LOOPBACK` (bit 14; bit 12 in the C6 map) feeds TX into RX.
* C2 clocks in `configs/chips/esp32c2.yaml`: UART `cpu_clock_hz:
  120000000` and `uart_sclk_hz: 40000000` (esp-hal selects PLL_F40M on the
  C2), SYSTIMER `cpu_clock_hz: 120000000` with an exact 2/15 tick ratio.
  Before it a UART1 frame took 726 us of SYSTIMER time instead of 1389 us.
* Fast boot sets UART0 `CLKDIV` to the ROM's 115200 baud (347 + 3/16).

Known divergence: `labwired test` with idle fast-forward on (the default)
prints waits of 10015, 10016 and 10015 us, which is the 10 ms
`with_timeout` deadline. The UART1 handler runs about 190 cycles after the
16th byte in both modes. The timer-poll coalesce
(`take_timer_poll_coalesce_eligible`) then counts SYSTIMER snapshot reads in
the task wake path as a busy-poll and moves time to the next scheduler
event, which here is the timeout alarm. With the coalesce disabled (a local
debug build) or `LABWIRED_IDLE_FAST_FORWARD=0`, the CLI prints the values
above. The CLI smoke checks only the verdict; the vendor test checks the
per-frame timing.

Built with the pinned stable toolchain for `riscv32imc-unknown-none-elf`:

```sh
cd examples/esp32c2/esp-hal-uart-async
cargo +1.95.0 build --release
riscv32-esp-elf-objcopy --strip-all \
    target/riscv32imc-unknown-none-elf/release/esp32c2-esp-hal-uart-async \
    ../../../tests/fixtures/esp32c2-esp-hal-uart-async.elf
```

Fixture sha256: `8fbcfc9461185ed57a87e34e305892d443769ec29af6c64c60130565e4eb736f`
(two builds from different target directories gave the same hash).
