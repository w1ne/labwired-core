# ESP32-C3 esp-hal stack-guard + Delay fixture

`esp32c3-esp-hal-guard-delay.elf` is a bare esp-hal 1.1 application for the
ESP32-C3, used by `crates/core/tests/esp32c3_trigger_systimer.rs` on the
bare-ELF fast boot (with the vendored mask ROM injected).

It is the `examples/esp32c3-hello-world` crate (same `Cargo.toml` /
`Cargo.lock`, esp-hal 1.1.1, esp-println with `jtag-serial`, esp-backtrace)
with the `main.rs` below, built with esp-hal's **default** configuration — so
the stack guard is on. What it exercises:

* `esp_hal::init` programs debug trigger 0 as a store watchpoint on
  `__stack_chk_guard` (`csrw tselect` / `tcontrol` / `tdata1` / `tdata2`).
  Without the trigger CSRs the first `csrw tselect` is an illegal instruction
  and the app never reaches `main`.
* `Delay::delay_millis(2)` and `Instant::elapsed` read SYSTIMER UNIT0 through
  the UPDATE / VALUE_VALID handshake. With the chip YAML's register stub the
  counter never moves and the first delay never returns; with the real timer
  each delay measures ~2000 us.
* The final volatile store to `__stack_chk_guard` is what a stack overflow
  into the guard does: the watchpoint raises a breakpoint exception before the
  store retires, and esp-hal's `ExceptionHandler` (mtval == guard address)
  panics with "Detected a write to the main stack's guard value".

Built with the workspace's pinned stable toolchain (1.95.0) for
`riscv32imc-unknown-none-elf`, with the example's `[unstable] build-std` table
and its `rust-toolchain.toml` (`channel = "esp"`) removed:

```sh
cargo +1.95.0 build --release
llvm-objcopy --strip-all target/riscv32imc-unknown-none-elf/release/esp32c3-guard-delay \
    esp32c3-esp-hal-guard-delay.elf
```

```rust
#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::{delay::Delay, main, time::Instant};
use esp_println::println;

unsafe extern "C" {
    static mut __stack_chk_guard: u32;
}

#[main]
fn main() -> ! {
    let _peripherals = esp_hal::init(esp_hal::Config::default());
    println!("C3 GUARD BOOT");
    let delay = Delay::new();
    for i in 0..3u32 {
        let t0 = Instant::now();
        delay.delay_millis(2);
        let us = t0.elapsed().as_micros();
        println!("C3 delay {} elapsed_us={}", i + 1, us);
    }
    println!("C3 WRITING GUARD");
    unsafe {
        core::ptr::write_volatile(core::ptr::addr_of_mut!(__stack_chk_guard), 0xDEAD_BEEF);
    }
    println!("C3 GUARD WRITE NOT CAUGHT");
    loop {
        delay.delay_millis(1000);
    }
}
```

Committed file sha256:
`d944dd7865bfaf615ddb4ec0609489da3997414e243eeb7ee22fbd4a6ff22a09`.
