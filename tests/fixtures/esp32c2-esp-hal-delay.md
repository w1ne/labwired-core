# ESP32-C2 esp-hal init + Delay fixture

`esp32c2-esp-hal-delay.elf` is the `examples/esp32c2/esp-hal` crate
(esp-hal 1.2.2, esp-println 0.17 on UART0, esp-backtrace; versions pinned by
its `Cargo.lock`), used by `crates/core/tests/vendor/esp32c2_compiled_firmware.rs`
and `examples/esp32c2/esp-hal-smoke.yaml`.

What it exercises on the bare-ELF fast boot:

* `esp_hal::init`: ROM `memcpy`, the BBPLL bring-up through the ROM
  `rom_i2c_readReg`/`rom_i2c_writeReg` helpers (they dispatch through
  `rom_phyFuns`, which the `_rom_store_table` replay fills), the
  `BBPLL_CAL_DONE` poll on `I2C_ANA_MST.ANA_CONF0` (0x6004_E840), and the
  radio power-up writes in `rtc_sleep_pu`.
* esp-println on UART0, GPIO8 output toggles, and `Delay::delay_millis(2)`
  measured with `Instant` on SYSTIMER.

Expected UART0 output (measured 2026-11-16, after the C2 clock fix:
120 MHz core, SYSTIMER 7.5 cycles per tick, UART0 at 115200 on the 40 MHz
source; before it the ticks read 2117/2052/2115 us):

```
LW-C2 boot
LW-C2 tick 0 2001us
LW-C2 tick 1 2001us
LW-C2 tick 2 2001us
LW-C2 done
```

Built with the pinned stable toolchain for `riscv32imc-unknown-none-elf`:

```sh
cd examples/esp32c2/esp-hal
cargo +1.95.0 build --release
riscv32-esp-elf-objcopy --strip-all \
    target/riscv32imc-unknown-none-elf/release/esp32c2-esp-hal-delay \
    ../../../tests/fixtures/esp32c2-esp-hal-delay.elf
```

Fixture sha256: `ae6d7762963b8e9563a13cfd71f0ef15d84768cd6e105de7a8fc2ad029862b15`.
