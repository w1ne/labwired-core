// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32F401 Tier-1 fixture firmware (Cortex-M4F, thumbv7em-none-eabi).
//! Also covers the STM32F401CDU6 board variant (same silicon row).
//!
//! Validates the simulator's chip model peripheral-by-peripheral with RAW
//! REGISTER accesses against the peripherals wired by
//! `configs/chips/stm32f401.yaml`, reporting one line per peripheral class
//! over USART2 using the TIER1 protocol:
//!
//! ```text
//! TIER1 <class> PASS
//! TIER1 <class> FAIL code=<reason>
//! TIER1 done
//! ```
//!
//! The `uart` class is implicit: receiving `TIER1 done` over the UART is
//! itself the proof of a working UART path, so no `uart` line is printed.
//!
//! `timer`, `dma` and `irq` are NOT reported: the F401 yaml declares no
//! TIM/DMA/NVIC-class peripheral ids (systick does not count as a `timer`
//! class marker), so the matrix renders those cells `na`.
//!
//! Every poll is bounded by a fixed iteration count (the simulator is
//! deterministic — no wall-clock timeouts). Register offsets follow the
//! simulator's models: rcc.rs (`stm32f4` profile), gpio.rs (the yaml wires
//! the `stm32v2` MODER/ODR/BSRR layout per RM0368) and
//! uart.rs (`stm32f1` layout: SR @ 0x00, DR @ 0x04 — matches F4 silicon).

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ── Wired peripherals (configs/chips/stm32f401.yaml) ──────────────────────
const RCC_BASE: u32 = 0x4002_3800; // type rcc, profile stm32f4
const GPIOA_BASE: u32 = 0x4002_0000; // type gpio, stm32f1 layout (default)
const USART2_BASE: u32 = 0x4000_4400; // type uart, stm32f1 layout (default)

// USART2, stm32f1 layout: SR @ 0x00 (TXE = bit 7), DR @ 0x04.
// Read the full SR word and bit-test TXE: a sign-bit test on a byte
// load compiles to LDRSB reg-offset, which the simulator's 16-bit
// Thumb decoder does not implement (decoder/arm.rs only matches
// even-op 0101-family encodings).
const UART_STATUS: *const u32 = USART2_BASE as *const u32;
const UART_TX: *mut u8 = (USART2_BASE + 0x04) as *mut u8;
const TXE_BIT: u32 = 1 << 7;

// ── Helpers ────────────────────────────────────────────────────────────────

#[inline(always)]
fn rd32(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

#[inline(always)]
fn wr32(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

/// Fixed-iteration busy spin — deterministic in the simulator.
fn spin(iters: u32) {
    for i in 0..iters {
        core::hint::black_box(i);
    }
}

fn putc(byte: u8) {
    for _ in 0..10_000 {
        if unsafe { read_volatile(UART_STATUS) } & TXE_BIT != 0 {
            break;
        }
    }
    unsafe { write_volatile(UART_TX, byte) };
}

fn puts(s: &[u8]) {
    for &b in s {
        putc(b);
    }
}

fn report(class: &[u8], result: Result<(), &'static [u8]>) {
    puts(b"TIER1 ");
    puts(class);
    match result {
        Ok(()) => puts(b" PASS\n"),
        Err(code) => {
            puts(b" FAIL code=");
            puts(code);
            puts(b"\n");
        }
    }
}

// ── Checks ──────────────────────────────────────────────────────────────────

/// clock: F4 RCC. HSI is on+ready out of reset; HSEON (bit 16) must latch
/// HSERDY (bit 17); SW→SWS mirrors in CFGR @ 0x08 (RM0090 §6.3.3);
/// AHB1ENR @ 0x30 round-trips GPIO port enables.
fn check_clock() -> Result<(), &'static [u8]> {
    if rd32(RCC_BASE) & (1 << 1) == 0 {
        return Err(b"clock-hsirdy");
    }
    let cr = rd32(RCC_BASE);
    wr32(RCC_BASE, cr | (1 << 16)); // HSEON
    if rd32(RCC_BASE) & (1 << 17) == 0 {
        return Err(b"clock-hserdy");
    }
    wr32(RCC_BASE, cr); // drop HSE; HSERDY must clear
    if rd32(RCC_BASE) & (1 << 17) != 0 {
        return Err(b"clock-hserdy-stuck");
    }
    // CFGR SW=01 → SWS must mirror.
    wr32(RCC_BASE + 0x08, 0x1);
    if (rd32(RCC_BASE + 0x08) >> 2) & 0x3 != 0x1 {
        return Err(b"clock-sws");
    }
    // AHB1ENR round-trip: GPIOA/B/C enables.
    wr32(RCC_BASE + 0x30, 0x7);
    if rd32(RCC_BASE + 0x30) != 0x7 {
        return Err(b"clock-enr");
    }
    Ok(())
}

/// gpio: the yaml wires the default stm32f1 register layout. PA5 to
/// output mode via MODER, set via BSRR, observe ODR, clear via BSRR-reset.
fn check_gpio() -> Result<(), &'static [u8]> {
    // STM32v2 layout (real F401 silicon: MODER/ODR/BSRR — RM0368 §8.4).
    let moder = rd32(GPIOA_BASE); // MODER @ 0x00
    wr32(GPIOA_BASE, (moder & !(0x3 << 10)) | (0x1 << 10)); // PA5 output
    wr32(GPIOA_BASE + 0x18, 1 << 5); // BSRR @ 0x18: set PA5
    if rd32(GPIOA_BASE + 0x14) & (1 << 5) == 0 {
        // ODR @ 0x14
        return Err(b"gpio-set");
    }
    wr32(GPIOA_BASE + 0x18, 1 << (5 + 16)); // BSRR reset half: clear PA5
    if rd32(GPIOA_BASE + 0x14) & (1 << 5) != 0 {
        return Err(b"gpio-clear");
    }
    Ok(())
}

#[entry]
fn main() -> ! {
    report(b"clock", check_clock());
    report(b"gpio", check_gpio());
    puts(b"TIER1 done\n");

    loop {
        spin(1_000_000);
    }
}
