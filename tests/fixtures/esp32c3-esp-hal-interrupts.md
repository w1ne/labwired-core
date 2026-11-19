# ESP32-C3 esp-hal interrupt and Embassy fixtures

Five images, each an `examples/esp32c2/esp-hal-*` crate built for the
ESP32-C3: the same `main.rs` with the `LW-C2` markers renamed `LW-C3`, and
the `esp32c2` crate features replaced by `esp32c3` (versions pinned by each
crate's `Cargo.lock`: esp-hal 1.2.2, esp-rtos 0.4.0, embassy-executor
0.10.0, esp-println 0.17.0 with `uart`, esp-backtrace 0.19). UART1 base and
CONF0 offset (0x6001_0000 + 0x20) and GPIO2 are the same on both chips.

| Fixture | Source crate | What it exercises |
|---|---|---|
| `esp32c3-esp-hal-irq.elf` | `esp-hal-irq` | `#[handler]` on FROM_CPU_INTR0 and a 1 ms periodic SYSTIMER alarm |
| `esp32c3-esp-hal-gpio-irq.elf` | `esp-hal-gpio-irq` | GPIO2 rising / any-edge interrupts, an unlistened pin |
| `esp32c3-esp-hal-embassy.elf` | `esp-hal-embassy` | esp-rtos Embassy executor, `Timer::after`, `with_timeout` |
| `esp32c3-esp-hal-gpio-async.elf` | `esp-hal-gpio-async` | `wait_for_rising_edge` on a self-driven pad |
| `esp32c3-esp-hal-uart-async.elf` | `esp-hal-uart-async` | async UART1 loopback, idle read timeout |

Used by `crates/core/tests/esp/esp32c3_esp_hal_interrupts.rs` and
`examples/esp32c3-hello-world/esp-hal-*-smoke.yaml`.

Why they exist: the bare-ELF fast boot skips the mask ROM's
`ets_intr_unlock` (jump table 0x4000_05FC -> 0x4004_7A42), whose
`csrrsi a5, mstatus, 8` at 0x4004_7A52 is the only code that sets
`mstatus.MIE` before the app. esp-hal 1.2 never sets MIE from zero. Before
`boot::esp32c3_rom::prepare_fast_boot_cpu`, all five images booted and
printed, but no handler ran (`swi hits 0`, `alarm hits 0`, `rising hits 0`)
and no Embassy task woke.

`esp32c3-esp-hal-uart-async.elf` also found a UART clock defect. esp-hal 1.2
selects XTAL (40 MHz) as the C3 UART function clock (`CLK_CONF.SCLK_SEL` = 3)
and programs CLKDIV for it; the model paced every C3 UART from a fixed
80 MHz, so a 16-byte frame took 725 us instead of 1389 us and the app's
physical lower bound failed. The chip YAML now gives `uart_sclk_sel_hz`, and
the shared `EspUart` paces from the selected source once firmware writes
`CLK_CONF`.

`labwired test` output measured 2026-11-16 (`esp-hal-irq`):

```
LW-C3 irq boot
LW-C3 swi hits 3
LW-C3 alarm hits 5
LW-C3 irq PASS
```

Build, with the workspace's pinned stable toolchain (1.95.0) for
`riscv32imc-unknown-none-elf`, for each `<name>` above:

```sh
cp -r examples/esp32c2/esp-hal-<name> /tmp/c3 && cd /tmp/c3
sed -i 's/esp32c2/esp32c3/g' Cargo.toml
sed -i 's/LW-C2/LW-C3/g' src/main.rs
cargo +1.95.0 build --release
riscv32-esp-elf-objcopy --strip-all \
    target/riscv32imc-unknown-none-elf/release/esp32c3-esp-hal-<name> \
    esp32c3-esp-hal-<name>.elf
```

SHA-256:

```
6135d0b7c91e105b0ba77c8efb491b1431da3fee2859ab4f9b067e7adf34aa48  esp32c3-esp-hal-irq.elf
ef5372dc2417d47aea189f6547f005e56cc48648a0eb1a2509ad1c7dbb4491b0  esp32c3-esp-hal-gpio-irq.elf
98ede0c47d3710c2bece0fc43ea32375ef06d686502c1c99936c46db779997af  esp32c3-esp-hal-embassy.elf
317999255f3d789eacf8d870421e86669165396872011e48b7a849bd3009c6d2  esp32c3-esp-hal-gpio-async.elf
e951b7ae90dd53e9b8c7638a598036a28908eeedbb6cc309193fe1d943bd2d81  esp32c3-esp-hal-uart-async.elf
```
