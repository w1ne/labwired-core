# ESP32-C3 esp-hal pulse fixture

`esp32c3-esp-hal-pulses.elf` is a bare esp-hal 1.1 application for the
ESP32-C3, used by `crates/core/tests/world_esp_nodes.rs` and the
`labwired-wasm` world tests as a C3 bare-ELF world node.

It is the `examples/esp32c3-hello-world` crate (same `Cargo.toml` /
`Cargo.lock`, esp-hal 1.1.1, esp-println with `jtag-serial`) with the
`main.rs` below. What it exercises:

* `esp_hal::init` brings the clocks up through the mask ROM's helpers, so the
  node only boots when the real ROM is in its windows with its `.data` copy
  replayed (without that it faults with a memory violation);
* `esp-println` prints over USB-Serial-JTAG, so the console needs the
  behavioural USB-Serial-JTAG model, not the chip YAML's register stub;
* `Output` drives GPIO4 through the GPIO matrix: ten pulses, one console line
  per pulse, then `C3 PULSES DONE`.

The waits are busy loops, not `esp_hal::delay::Delay`. When this fixture was
built, the bare-ELF fast boot had only the chip YAML's SYSTIMER register stub,
so `Delay` never saw time pass. The C3 YAML now carries the behavioural
SYSTIMER (`esp32c3_systimer`), so `Delay` works on fast boot and in worlds; see
`tests/fixtures/esp32c3-esp-hal-guard-delay.md`. The fixture is kept as built.

Built with the workspace's pinned stable toolchain (1.95.0) for
`riscv32imc-unknown-none-elf`, with the example's `[unstable] build-std` table
removed and esp-hal's stack-guard watchpoint off. At the time the RISC-V model
did not implement the debug trigger CSRs (`tselect` / `tdata1`) the watchpoint
programs; it does now, so a default esp-hal build also runs:

```sh
ESP_HAL_CONFIG_STACK_GUARD_MONITORING=false cargo +1.95.0 build --release
llvm-objcopy --strip-all target/riscv32imc-unknown-none-elf/release/esp32c3-hello-world \
    esp32c3-esp-hal-pulses.elf
```

```rust
#![no_std]
#![no_main]

use core::hint::black_box;
use esp_backtrace as _;
use esp_hal::{
    gpio::{Level, Output, OutputConfig},
    main,
};
use esp_println::println;

#[inline(never)]
fn spin(n: u32) {
    for i in 0..n {
        black_box(i);
    }
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut pin = Output::new(peripherals.GPIO4, Level::Low, OutputConfig::default());
    println!("C3 ESP-HAL BOOT");
    for i in 0..10u32 {
        pin.set_high();
        spin(2_000);
        pin.set_low();
        spin(2_000);
        println!("C3 pulse {}", i + 1);
    }
    println!("C3 PULSES DONE");
    loop {
        spin(1_000_000);
    }
}
```

Committed file sha256:
`7dad183c939edeab8faa417c3468dcb90c4d1c3eea3a5d97df67c142c6d1b48a`.
