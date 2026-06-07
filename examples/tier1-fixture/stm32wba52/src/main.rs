// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32WBA52 Tier-1 fixture firmware (Cortex-M33; built thumbv7m-none-eabi,
//! matching the in-repo H563/M33 firmware convention).
//!
//! Validates the simulator's chip model peripheral-by-peripheral with RAW
//! REGISTER accesses against the peripherals wired by
//! `configs/chips/stm32wba52.yaml`, reporting one line per peripheral class
//! over LPUART1 using the TIER1 protocol:
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
//! `timer`, `dma` and `irq` are NOT reported: the WBA52 yaml declares no
//! TIM/DMA/NVIC-class peripheral ids (systick does not count as a `timer`
//! class marker), so the matrix renders those cells `na`.
//!
//! Every poll is bounded by a fixed iteration count (the simulator is
//! deterministic — no wall-clock timeouts). Register offsets follow the
//! simulator's models: rcc.rs (`stm32v2` profile), gpio.rs (`stm32v2`) and
//! uart.rs (`stm32v2`).

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ── Wired peripherals (configs/chips/stm32wba52.yaml) ─────────────────────
const RCC_BASE: u32 = 0x4602_0C00; // type rcc, profile stm32v2
const GPIOA_BASE: u32 = 0x4202_0000; // type gpio, profile stm32v2
const LPUART1_BASE: u32 = 0x4600_2400; // type uart, profile stm32v2

// LPUART1, stm32v2 layout: ISR @ 0x1C (TXE = bit 7), TDR @ 0x28.
// Read the full ISR word and bit-test TXE: a sign-bit test on a byte
// load compiles to LDRSB reg-offset, which the simulator's 16-bit
// Thumb decoder does not implement (decoder/arm.rs only matches
// even-op 0101-family encodings).
const UART_STATUS: *const u32 = (LPUART1_BASE + 0x1C) as *const u32;
const UART_TX: *mut u8 = (LPUART1_BASE + 0x28) as *mut u8;
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

/// clock: V2 (H5-style) RCC. HSI is on+ready out of reset; HSEON (bit 16)
/// must latch HSERDY (bit 17); SW→SWS mirrors in CFGR @ 0x04; AHB2ENR @
/// 0x8C round-trips GPIO port enables.
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
    wr32(RCC_BASE + 0x04, 0x1);
    if (rd32(RCC_BASE + 0x04) >> 2) & 0x3 != 0x1 {
        return Err(b"clock-sws");
    }
    // AHB2ENR round-trip: GPIOA/B/C/H enables.
    wr32(RCC_BASE + 0x8C, 0x87);
    if rd32(RCC_BASE + 0x8C) != 0x87 {
        return Err(b"clock-enr");
    }
    Ok(())
}

/// gpio: stm32v2 port. PA5 to output via MODER, set via BSRR, observe ODR,
/// clear via BRR.
///
/// KNOWN MODEL GAP: this port's MMIO window (0x4202_xxxx) lies inside the
/// Cortex-M peripheral bit-band ALIAS range (0x4200_0000-0x43FF_FFFF), and
/// the simulator bus applies bit-band translation to every 32-bit access on
/// every ARM chip (bus/mod.rs `bit_band_translate`), even though this core
/// has no bit-banding and the chip yaml wires real peripherals here. Word
/// accesses therefore never reach the GPIO model and the check fails with
/// `gpio-bitband-shadow` (same root cause as the nucleo-h563zi io-smoke
/// assertion failure). The failure code names the root cause rather than
/// the first failing sub-step.
fn check_gpio() -> Result<(), &'static [u8]> {
    let moder = rd32(GPIOA_BASE);
    wr32(GPIOA_BASE, (moder & !(0x3 << 10)) | (0x1 << 10)); // PA5 output
    wr32(GPIOA_BASE + 0x18, 1 << 5); // BSRR set
    if rd32(GPIOA_BASE + 0x14) & (1 << 5) == 0 {
        return Err(b"gpio-bitband-shadow");
    }
    wr32(GPIOA_BASE + 0x28, 1 << 5); // BRR clear
    if rd32(GPIOA_BASE + 0x14) & (1 << 5) != 0 {
        return Err(b"gpio-bitband-shadow");
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
