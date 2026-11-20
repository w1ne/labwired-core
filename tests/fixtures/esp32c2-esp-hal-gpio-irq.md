# ESP32-C2 esp-hal GPIO interrupt fixture

`esp32c2-esp-hal-gpio-irq.elf` is the `examples/esp32c2/esp-hal-gpio-irq`
crate (esp-hal 1.2.2, esp-println 0.17 on UART0, esp-backtrace,
critical-section; versions pinned by its `Cargo.lock`), used by
`crates/core/tests/vendor/esp32c2_compiled_firmware.rs` and
`examples/esp32c2/esp-hal-gpio-irq-smoke.yaml`.

What it exercises on the bare-ELF fast boot:

* GPIO2 as a `Flex` pin with output and input enabled: the pad drives its
  own input buffer, so each `set_high`/`set_low` is an input edge.
* `Io::set_interrupt_handler` with an `#[handler]` (matrix source 13). The
  handler clears `GPIO_STATUS` for the pin and counts.
* `GPIO_PINn.INT_TYPE` per `listen()`: RisingEdge (3 pulses, 3 runs),
  AnyEdge (2 pulses, 4 runs), and `unlisten()` (2 pulses, 0 runs).

Expected UART0 output (measured 2026-11-16):

```
LW-C2 gpio-irq boot
LW-C2 rising hits 3
LW-C2 any-edge hits 4
LW-C2 unlistened hits 0
LW-C2 gpio-irq PASS
```

With the interrupt matrix not armed, the handler never runs and the app
prints `rising hits 0` and `gpio-irq FAIL` (the vendor negative control).

Built with the pinned stable toolchain for `riscv32imc-unknown-none-elf`:

```sh
cd examples/esp32c2/esp-hal-gpio-irq
cargo +1.95.0 build --release
riscv32-esp-elf-objcopy --strip-all \
    target/riscv32imc-unknown-none-elf/release/esp32c2-esp-hal-gpio-irq \
    ../../../tests/fixtures/esp32c2-esp-hal-gpio-irq.elf
```

Fixture sha256: `2e9ad397399de7f5fc0ecdbce6d1fe1731a59072b9bffecab727420fe7f3b77e`
(two builds from different target directories gave the same hash).
