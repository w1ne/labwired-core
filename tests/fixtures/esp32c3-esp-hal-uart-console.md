# ESP32-C3 esp-hal UART-console fixture

`esp32c3-esp-hal-uart-console.elf` is the `examples/esp32c2/esp-hal` crate
built for the ESP32-C3: the same `main.rs` with the `LW-C2` markers renamed
`LW-C3`, and the three `esp32c2` crate features replaced by `esp32c3`
(esp-hal 1.2.2, esp-println 0.17.0 with the `uart` feature, esp-backtrace
0.19). Used by `crates/core/tests/esp/esp32c3_trigger_systimer.rs`.

What it exercises: esp-println's `uart` backend prints through the mask-ROM
`uart_tx_one_char` (0x4000_0068). That routine returns without a write while
the ROM flag `g_uart_print` (0x3FCD_FFD1) is zero. The real ROM sets the flag
during its print init; the bare-ELF fast boot must seed it
(`boot::esp32c3_rom::seed_rom_console`), or the console stays empty while the
app runs normally.

Built with the workspace's pinned stable toolchain (1.95.0) for
`riscv32imc-unknown-none-elf`:

```sh
cp -r examples/esp32c2/esp-hal /tmp/c3 && cd /tmp/c3
sed -i 's/esp32c2/esp32c3/g' Cargo.toml
sed -i 's/LW-C2/LW-C3/g' src/main.rs
cargo +1.95.0 build --release
riscv32-esp-elf-objcopy --strip-all \
    target/riscv32imc-unknown-none-elf/release/esp32c3-esp-hal-delay \
    esp32c3-esp-hal-uart-console.elf
```

SHA-256: `0f5f710f5355341a81bb97e5dd4517130c78309137100ce318a2480f5ce8e6d2`
