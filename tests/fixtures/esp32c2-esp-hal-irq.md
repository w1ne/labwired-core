# ESP32-C2 esp-hal interrupt fixture

`esp32c2-esp-hal-irq.elf` is the `examples/esp32c2/esp-hal-irq` crate
(esp-hal 1.2.2, esp-println 0.17 on UART0, esp-backtrace, critical-section;
versions pinned by its `Cargo.lock`), used by
`crates/core/tests/vendor/esp32c2_compiled_firmware.rs` and
`examples/esp32c2/esp-hal-irq-smoke.yaml`.

What it exercises on the bare-ELF fast boot:

* A FROM_CPU software interrupt (`SoftwareInterrupt<0>`, matrix source 36)
  with an `#[handler]`, raised three times. `raise` polls
  `INTR_STATUS_REG_1` until the doorbell is visible.
* A 1 ms `PeriodicTimer` on SYSTIMER alarm 0 (matrix source 26) with an
  `#[handler]` that clears the alarm, counted over a 5 ms `Delay`.
* esp-hal's CPU-line dispatch: priority, threshold and the INTR_STATUS
  source lookup in `handle_interrupts`.

Expected UART0 output (measured 2026-11-16):

```
LW-C2 irq boot
LW-C2 swi hits 3
LW-C2 alarm hits 5
LW-C2 irq PASS
```

Built with the pinned stable toolchain for `riscv32imc-unknown-none-elf`:

```sh
cd examples/esp32c2/esp-hal-irq
cargo +1.95.0 build --release
riscv32-esp-elf-objcopy --strip-all \
    target/riscv32imc-unknown-none-elf/release/esp32c2-esp-hal-irq \
    ../../../tests/fixtures/esp32c2-esp-hal-irq.elf
```

Fixture sha256: `a5ec09141b59c6c322e5b6584cf6e5d927952c257c20d1e6f2347f26d6f9d49d`
(two builds from different target directories gave the same hash).
