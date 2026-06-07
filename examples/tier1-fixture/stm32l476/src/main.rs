// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32L476 Tier-1 fixture firmware.
//!
//! Validates the simulator's chip model peripheral-by-peripheral with RAW
//! REGISTER accesses against the peripherals wired by
//! `configs/chips/stm32l476.yaml`, reporting one line per peripheral class
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
//! Every poll is bounded by a fixed iteration count (the simulator is
//! deterministic — no wall-clock timeouts). Register offsets follow the
//! simulator's models: rcc.rs (`stm32l4` profile), gpio.rs (`stm32v2`),
//! uart.rs (`stm32v2`), timer.rs, dma.rs (Dma1) and nvic.rs.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::{entry, exception};
use panic_halt as _;

// ── Wired peripherals (configs/chips/stm32l476.yaml) ──────────────────────
const RCC_BASE: u32 = 0x4002_1000; // type rcc, profile stm32l4
const GPIOA_BASE: u32 = 0x4800_0000; // type gpio, profile stm32v2
const USART2_BASE: u32 = 0x4000_4400; // type uart, profile stm32v2
const TIM2_BASE: u32 = 0x4000_0000; // type timer, width 32
const DMA1_BASE: u32 = 0x4002_0000; // type dma (Dma1, 7ch)

// NVIC (installed for every Cortex-M chip; declared in the yaml as `nvic`).
const NVIC_ISER0: u32 = 0xE000_E100;
const NVIC_ICER0: u32 = 0xE000_E180;
const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ICPR0: u32 = 0xE000_E280;
/// Software-pended test IRQ. Must be < 32 (cortex-m-rt default vector
/// table) and unused by any yaml peripheral (L476 yaml uses 0, 11, 15-19,
/// 24-31 in that range — 20 is free).
const TEST_IRQ: i16 = 20;

// USART2, stm32v2 layout: ISR @ 0x1C (TXE = bit 7), TDR @ 0x28.
// Read the full ISR word and bit-test TXE: a sign-bit test on a byte
// load compiles to LDRSB reg-offset, which the simulator's 16-bit
// Thumb decoder does not implement (decoder/arm.rs only matches
// even-op 0101-family encodings).
const UART_STATUS: *const u32 = (USART2_BASE + 0x1C) as *const u32;
const UART_TX: *mut u8 = (USART2_BASE + 0x28) as *mut u8;
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

/// clock: L4 RCC. MSI is on+ready out of reset (CR=0x63); HSE ready follows
/// HSEON only with HSEBYP; SW→SWS in CFGR @ 0x08 follows when the source is
/// ready; AHB2ENR @ 0x4C round-trips GPIO port enables.
fn check_clock() -> Result<(), &'static [u8]> {
    if rd32(RCC_BASE) & (1 << 1) == 0 {
        return Err(b"clock-msirdy");
    }
    // HSE with bypass: HSEON(16)+HSEBYP(18) → HSERDY(17).
    let cr = rd32(RCC_BASE);
    wr32(RCC_BASE, cr | (1 << 16) | (1 << 18));
    if rd32(RCC_BASE) & (1 << 17) == 0 {
        return Err(b"clock-hserdy");
    }
    wr32(RCC_BASE, cr); // restore (drops HSE, HSERDY must clear)
    if rd32(RCC_BASE) & (1 << 17) != 0 {
        return Err(b"clock-hserdy-stuck");
    }
    // Switch SYSCLK to HSI16 (SW=01); SWS must follow (HSI ready @ bit 1).
    wr32(RCC_BASE + 0x08, 0x1);
    if (rd32(RCC_BASE + 0x08) >> 2) & 0x3 != 0x1 {
        return Err(b"clock-sws");
    }
    // AHB2ENR round-trip: enable GPIOA/B/C clocks.
    wr32(RCC_BASE + 0x4C, 0x7);
    if rd32(RCC_BASE + 0x4C) != 0x7 {
        return Err(b"clock-enr");
    }
    Ok(())
}

/// gpio: stm32v2 port. PA5 to output via MODER, set via BSRR, observe ODR,
/// clear via BRR.
fn check_gpio() -> Result<(), &'static [u8]> {
    let moder = rd32(GPIOA_BASE);
    wr32(GPIOA_BASE, (moder & !(0x3 << 10)) | (0x1 << 10)); // PA5 output
    wr32(GPIOA_BASE + 0x18, 1 << 5); // BSRR set
    if rd32(GPIOA_BASE + 0x14) & (1 << 5) == 0 {
        return Err(b"gpio-set");
    }
    wr32(GPIOA_BASE + 0x28, 1 << 5); // BRR clear
    if rd32(GPIOA_BASE + 0x14) & (1 << 5) != 0 {
        return Err(b"gpio-clear");
    }
    Ok(())
}

/// timer: TIM2 (32-bit). EGR.UG latches UIF and zeroes CNT; SR write-0
/// clears; with CEN set the counter advances between two bounded reads.
fn check_timer() -> Result<(), &'static [u8]> {
    wr32(TIM2_BASE + 0x28, 0); // PSC = 0
    wr32(TIM2_BASE + 0x2C, 0xFFFF_FFFF); // ARR = max (32-bit TIM2)
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

/// Hit counter for the software-pended test IRQ. Plain volatile (not
/// atomic RMW) — single-core, handler never preempts the check loop's read
/// in a way that loses the increment.
static mut IRQ_HITS: u32 = 0;

/// irq: NVIC delivery round-trip. Enable TEST_IRQ in ISER0, software-pend
/// it via ISPR0, and require the vector to actually run (DefaultHandler
/// counts it and disarms itself).
fn check_irq() -> Result<(), &'static [u8]> {
    wr32(NVIC_ISER0, 1 << TEST_IRQ as u32);
    wr32(NVIC_ISPR0, 1 << TEST_IRQ as u32);
    for _ in 0..10_000 {
        if unsafe { read_volatile(core::ptr::addr_of!(IRQ_HITS)) } != 0 {
            return Ok(());
        }
    }
    // Disarm in case delivery never happened.
    wr32(NVIC_ICER0, 1 << TEST_IRQ as u32);
    wr32(NVIC_ICPR0, 1 << TEST_IRQ as u32);
    Err(b"irq-not-delivered")
}

#[exception]
unsafe fn DefaultHandler(irqn: i16) {
    if irqn == TEST_IRQ {
        // Disarm first so a level re-pend can't wedge the main thread.
        wr32(NVIC_ICER0, 1 << TEST_IRQ as u32);
        wr32(NVIC_ICPR0, 1 << TEST_IRQ as u32);
        let hits = read_volatile(core::ptr::addr_of!(IRQ_HITS));
        write_volatile(core::ptr::addr_of_mut!(IRQ_HITS), hits + 1);
    }
}

#[entry]
fn main() -> ! {
    report(b"clock", check_clock());
    report(b"gpio", check_gpio());
    report(b"timer", check_timer());
    report(b"dma", check_dma());
    report(b"irq", check_irq());
    puts(b"TIER1 done\n");

    loop {
        spin(1_000_000);
    }
}
