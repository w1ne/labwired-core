// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32F103 Tier-1 fixture firmware (Cortex-M3, thumbv7m-none-eabi).
//!
//! Validates the simulator's chip model peripheral-by-peripheral with RAW
//! REGISTER accesses against the peripherals wired by
//! `configs/chips/stm32f103.yaml`, reporting one line per peripheral class
//! over USART1 using the TIER1 protocol:
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
//! The `irq` class is NOT reported: the F103 yaml declares no NVIC/EXTI-class
//! peripheral id, so the matrix renders that cell `na`.
//!
//! Every poll is bounded by a fixed iteration count (the simulator is
//! deterministic — no wall-clock timeouts). Register offsets follow the
//! simulator's models: rcc.rs (`stm32f1` profile), gpio.rs (`stm32f1`),
//! uart.rs (`stm32f1`), timer.rs and dma.rs (Dma1).

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ── Wired peripherals (configs/chips/stm32f103.yaml) ──────────────────────
const RCC_BASE: u32 = 0x4002_1000; // type rcc, profile stm32f1 (default)
const GPIOA_BASE: u32 = 0x4001_0800; // type gpio, stm32f1 layout (default)
const USART1_BASE: u32 = 0x4001_3800; // type uart, stm32f1 layout (default)
const TIM2_BASE: u32 = 0x4000_0000; // type timer, 16-bit
const DMA1_BASE: u32 = 0x4002_0000; // type dma (Dma1, 7ch)

// USART1, stm32f1 layout: SR @ 0x00 (TXE = bit 7), DR @ 0x04.
// Read the full SR word and bit-test TXE: a sign-bit test on a byte
// load compiles to LDRSB reg-offset, which the simulator's 16-bit
// Thumb decoder does not implement (decoder/arm.rs only matches
// even-op 0101-family encodings).
const UART_STATUS: *const u32 = USART1_BASE as *const u32;
const UART_TX: *mut u8 = (USART1_BASE + 0x04) as *mut u8;
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

/// clock: F1 RCC. HSI is on+ready out of reset; HSEON (bit 16) must latch
/// HSERDY (bit 17); SW→SWS mirrors in CFGR @ 0x04; APB2ENR @ 0x18
/// round-trips peripheral enables.
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
    // APB2ENR round-trip: IOPA/IOPB/IOPC + USART1 enables.
    let enr = (1 << 2) | (1 << 3) | (1 << 4) | (1 << 14);
    wr32(RCC_BASE + 0x18, enr);
    if rd32(RCC_BASE + 0x18) != enr {
        return Err(b"clock-enr");
    }
    Ok(())
}

/// gpio: stm32f1 port. PA5 to push-pull output via CRL, set via BSRR,
/// observe ODR, clear via BRR.
fn check_gpio() -> Result<(), &'static [u8]> {
    let crl = rd32(GPIOA_BASE);
    wr32(GPIOA_BASE, (crl & !(0xF << 20)) | (0x3 << 20)); // PA5 output 50MHz
    wr32(GPIOA_BASE + 0x10, 1 << 5); // BSRR set
    if rd32(GPIOA_BASE + 0x0C) & (1 << 5) == 0 {
        return Err(b"gpio-set");
    }
    wr32(GPIOA_BASE + 0x14, 1 << 5); // BRR clear
    if rd32(GPIOA_BASE + 0x0C) & (1 << 5) != 0 {
        return Err(b"gpio-clear");
    }
    Ok(())
}

/// timer: TIM2 (16-bit). EGR.UG latches UIF and zeroes CNT; SR write-0
/// clears; with CEN set the counter advances between two bounded reads.
fn check_timer() -> Result<(), &'static [u8]> {
    wr32(TIM2_BASE + 0x28, 0); // PSC = 0
    wr32(TIM2_BASE + 0x2C, 0xFFFF); // ARR = max (16-bit)
    wr32(TIM2_BASE + 0x14, 1); // EGR.UG
    if rd32(TIM2_BASE + 0x10) & 1 == 0 {
        return Err(b"timer-uif");
    }
    wr32(TIM2_BASE + 0x10, 0); // SR: rc_w0 clear
    if rd32(TIM2_BASE + 0x10) & 1 != 0 {
        return Err(b"timer-uif-clear");
    }
    wr32(TIM2_BASE, 1); // CR1.CEN
    let c1 = rd32(TIM2_BASE + 0x24);
    spin(2_000);
    let c2 = rd32(TIM2_BASE + 0x24);
    wr32(TIM2_BASE, 0); // stop
    if c2 == c1 {
        return Err(b"timer-cnt-stuck");
    }
    Ok(())
}

/// dma: DMA1 channel 1 mem-to-mem copy (CCR.MEM2MEM, CMAR → CPAR), byte
/// elements with MINC+PINC. TCIF1 must latch and the destination must match.
fn check_dma() -> Result<(), &'static [u8]> {
    const N: usize = 8;
    let src: [u8; N] = [0xA5, 0x5A, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
    let mut dst: [u8; N] = [0; N];

    wr32(DMA1_BASE + 0x04, 0xF); // IFCR: clear stale CH1 flags
    wr32(DMA1_BASE + 0x10, dst.as_mut_ptr() as u32); // CPAR1 = destination
    wr32(DMA1_BASE + 0x14, src.as_ptr() as u32); // CMAR1 = source
    wr32(DMA1_BASE + 0x0C, N as u32); // CNDTR1
                                      // Configure first WITHOUT EN (the bus issues byte writes low-to-high, so
                                      // setting EN in the same word write would start the channel before the
                                      // MEM2MEM bit lands), then flip EN alone.
    let cfg: u32 = (1 << 14) | (1 << 7) | (1 << 6) | (1 << 4); // MEM2MEM|MINC|PINC|DIR
    wr32(DMA1_BASE + 0x08, cfg);
    unsafe { write_volatile((DMA1_BASE + 0x08) as *mut u8, (cfg | 1) as u8) }; // EN

    let mut done = false;
    for _ in 0..20_000 {
        if rd32(DMA1_BASE) & (1 << 1) != 0 {
            // TCIF1
            done = true;
            break;
        }
    }
    wr32(DMA1_BASE + 0x08, 0); // disable channel
    wr32(DMA1_BASE + 0x04, 0xF); // clear CH1 flags
    if !done {
        return Err(b"dma-tcif-timeout");
    }
    for i in 0..N {
        if unsafe { read_volatile(dst.as_ptr().add(i)) } != src[i] {
            return Err(b"dma-data-mismatch");
        }
    }
    Ok(())
}

#[entry]
fn main() -> ! {
    report(b"clock", check_clock());
    report(b"gpio", check_gpio());
    report(b"timer", check_timer());
    report(b"dma", check_dma());
    puts(b"TIER1 done\n");

    loop {
        spin(1_000_000);
    }
}
