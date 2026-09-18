# nRF54L15 Embassy board-demo fixture

This is the initial RoyalBlue54L-Feather import demo firmware, recovered from
the local nrf54-run evidence. The later signed-in demo uses raw 250,000-tick waits. It toggles P1.10 and P2.06 high/low with two `Timer::after_millis(250)` waits.
The Embassy thread executor sleeps using WFE at PC 0x6e2; its wake path issues SEV.
The GRTC compare interrupt is the time source. This original manifest incorrectly selects a 32.768 kHz time driver: the requested
250 ms waits actually arm 8,192 us GRTC deadlines. Preserve it as a regression
case, not as the 500 ms timing oracle.

`nrf54l15-embassy-blinky.elf` preserves the original loadable segments; only DWARF
was removed with `arm-none-eabi-objcopy --strip-debug`. Source and manifest used
for the original build are included below for reproducibility.

```rust
#![no_std]
#![no_main]

// Board demo for the nRF54L15 import path. Toggles the DK's own LED (P1.10,
// active high) and P2.06 (active high) on alternating half-seconds, so BOTH the
// bare board's starter and an imported design wired to P2.06 — the
// RoyalBlue54L-Feather's D1 — show digital activity on the canvas. No sensor
// driver, no vendor header: two GPIOs and a timer.
use embassy_executor::Spawner;
use embassy_nrf::gpio::{Level, Output, OutputDrive};
use embassy_time::Timer;
use panic_halt as _;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_nrf::init(Default::default());
    // Both LEDs are active high — the DK's LED1 on P1.10 and the imported LED
    // on P2.06 — so Level::Low is the off state for each.
    let mut dk_led = Output::new(p.P1_10, Level::Low, OutputDrive::Standard);
    let mut imported_led = Output::new(p.P2_06, Level::Low, OutputDrive::Standard);

    loop {
        dk_led.set_high();
        imported_led.set_high();
        Timer::after_millis(250).await;
        dk_led.set_low();
        imported_led.set_low();
        Timer::after_millis(250).await;
    }
}

```

```toml
[package]
name = "labwired-fw"
version = "0.1.0"
edition = "2021"
publish = false

[dependencies]
cortex-m = { version = "0.7.7", features = ["critical-section-single-core"] }
cortex-m-rt = "0.7.5"
panic-halt = "0.2.0"
embassy-executor = { version = "0.9.1", features = [
    "arch-cortex-m",
    "executor-thread",
] }
embassy-time = { version = "0.5.1", features = ["tick-hz-32_768"] }
embassy-nrf = { version = "0.11.0", features = [
    "nrf54l15-app-ns",
    "time-driver-rtc1",
    "gpiote",
    "rt",
] }

[profile.release]
debug = 2
lto = false
opt-level = "s"

```

The regression `nrf54l15_embassy_realtime` checks accelerated versus per-cycle
execution at 65 million cycles: all captured P2.06 edge timestamps and the CPU
snapshot must match, and acceleration must coalesce over 60 million sleep cycles.

`nrf54l15-embassy-grtc.elf` rebuilds the same source using
`embassy-time/tick-hz-1_000_000` and `embassy-nrf/time-driver-grtc`, matching the
corrected hosted builder manifests. Each high/low interval is 250 ms, plus the
firmware instructions between waits. Both fixtures use the same scheduler and
CPU model; the emulator's 128 MHz CPU / 1 MHz GRTC clocks are unchanged.

The corrected fixture was built with rustc 1.95.0 for
`thumbv8m.main-none-eabihf`, release `opt-level=s`, no LTO, from the same source
and locked dependency graph as the original local build, then stripped of DWARF.

SHA-256 (stripped ELF):

- Original: `f1e88e48fdea91e281d7026ff9d46529773574d12a88cf5ad0a3458cb0f0a5b2`
- Correct GRTC: `6c0c70d3b23f4c49a3579e531546844050ab5d2c26cfe267e5980c2827552cb1`
