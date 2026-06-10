// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32H563 Tier-1 fixture firmware (Cortex-M33; built thumbv7m-none-eabi,
//! matching the in-repo H563 firmware convention).
//!
//! Validates the simulator's chip model peripheral-by-peripheral with RAW
//! REGISTER accesses against the peripherals wired by
//! `configs/chips/stm32h563.yaml`, reporting one line per peripheral class
//! over USART3 using the TIER1 protocol:
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
//! `irq` is NOT reported: the H563 yaml declares no NVIC-class peripheral
//! id (systick does not count), so the matrix renders that cell `na`.
//!
//! Every poll is bounded by a fixed iteration count (the simulator is
//! deterministic — no wall-clock timeouts). Register offsets follow the
//! simulator's models: rcc.rs (`stm32v2` profile), gpio.rs (`stm32v2`),
//! uart.rs (`stm32v2`) and dma.rs (Dma1 — the yaml wires the generic
//! 7-channel STM32 DMA model, not the H5 GPDMA).

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ── Wired peripherals (configs/chips/stm32h563.yaml) ──────────────────────
const RCC_BASE: u32 = 0x4402_0C00; // type rcc, profile stm32v2
const GPIOA_BASE: u32 = 0x4202_0000; // type gpio, profile stm32v2
const USART3_BASE: u32 = 0x4000_4800; // type uart, profile stm32v2
const DMA1_BASE: u32 = 0x4002_0000; // type dma (Dma1, 7ch)

// USART3, stm32v2 layout: ISR @ 0x1C (TXE = bit 7), TDR @ 0x28.
// Read the full ISR word and bit-test TXE: a sign-bit test on a byte
// load compiles to LDRSB reg-offset, which the simulator's 16-bit
// Thumb decoder does not implement (decoder/arm.rs only matches
// even-op 0101-family encodings).
const UART_STATUS: *const u32 = (USART3_BASE + 0x1C) as *const u32;
const UART_TX: *mut u8 = (USART3_BASE + 0x28) as *mut u8;
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

/// clock: H5 RCC (RM0481 layout, offsets verified on NUCLEO-H563ZI silicon).
/// HSI is on+ready out of reset; HSEON (bit 16) must latch HSERDY (bit 17);
/// SW[2:0]→SWS[5:3] mirrors in CFGR1 @ 0x1C; AHB2ENR @ 0x8C round-trips
/// GPIO port enables.
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
    // SW→SWS is gated on the source's ready bit (silicon-verified): SW=001
    // (CSI) with CSI off must NOT switch; after CSION+CSIRDY it must.
    wr32(RCC_BASE + 0x1C, 0x1);
    if (rd32(RCC_BASE + 0x1C) >> 3) & 0x7 != 0x0 {
        return Err(b"clock-sws-gate");
    }
    wr32(RCC_BASE, cr | (1 << 8)); // CSION
    for _ in 0..10_000 {
        if rd32(RCC_BASE) & (1 << 9) != 0 {
            break;
        }
    }
    if rd32(RCC_BASE) & (1 << 9) == 0 {
        return Err(b"clock-csirdy");
    }
    wr32(RCC_BASE + 0x1C, 0x1);
    if (rd32(RCC_BASE + 0x1C) >> 3) & 0x7 != 0x1 {
        return Err(b"clock-sws");
    }
    // Back to HSI, CSI off.
    wr32(RCC_BASE + 0x1C, 0x0);
    if (rd32(RCC_BASE + 0x1C) >> 3) & 0x7 != 0x0 {
        return Err(b"clock-sws-back");
    }
    wr32(RCC_BASE, cr);
    // AHB2ENR round-trip: GPIOA..GPIOG enables on top of the reset value
    // (SRAM2EN|SRAM3EN stay up), restored afterwards.
    let enr = rd32(RCC_BASE + 0x8C);
    wr32(RCC_BASE + 0x8C, enr | 0x7F);
    if rd32(RCC_BASE + 0x8C) & 0x7F != 0x7F {
        return Err(b"clock-enr");
    }
    wr32(RCC_BASE + 0x8C, enr);
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

/// timer: TIM2 (32-bit). PSC/ARR write+readback, EGR.UG latches SR.UIF,
/// write-0 clears it, CEN makes CNT advance, CEN off freezes it. State
/// restored (ARR back to full-scale, CNT 0).
fn check_timer() -> Result<(), &'static [u8]> {
    const TIM2: u32 = 0x4000_0000;
    wr32(TIM2 + 0x28, 7); // PSC
    if rd32(TIM2 + 0x28) != 7 {
        return Err(b"tim-psc");
    }
    wr32(TIM2 + 0x2C, 0x0001_0000); // ARR beyond 16 bits — counter is 32-bit
    if rd32(TIM2 + 0x2C) != 0x0001_0000 {
        return Err(b"tim-arr32");
    }
    wr32(TIM2 + 0x14, 0x1); // EGR.UG
    if rd32(TIM2 + 0x10) & 0x1 == 0 {
        return Err(b"tim-uif");
    }
    wr32(TIM2 + 0x10, 0); // clear UIF
    if rd32(TIM2 + 0x10) & 0x1 != 0 {
        return Err(b"tim-uif-clear");
    }
    wr32(TIM2 + 0x00, 0x1); // CR1.CEN
    spin(2_000);
    let cnt = rd32(TIM2 + 0x24);
    if cnt == 0 {
        return Err(b"tim-cnt-stuck");
    }
    wr32(TIM2 + 0x00, 0x0); // CEN off
    let frozen = rd32(TIM2 + 0x24);
    spin(2_000);
    if rd32(TIM2 + 0x24) != frozen {
        return Err(b"tim-cnt-runs");
    }
    wr32(TIM2 + 0x24, 0);
    wr32(TIM2 + 0x28, 0);
    wr32(TIM2 + 0x2C, 0xFFFF_FFFF);
    Ok(())
}

/// i2c: I2C1 (v2 IP). ISR resets to TXE; OAR1/TIMINGR round-trip; CR1.PE
/// set + clear. State restored.
fn check_i2c() -> Result<(), &'static [u8]> {
    const I2C1: u32 = 0x4000_5400;
    if rd32(I2C1 + 0x18) & 0x1 == 0 {
        return Err(b"i2c-txe");
    }
    wr32(I2C1 + 0x08, 0x8000_0052); // OAR1: OA1EN | addr 0x29<<1
    if rd32(I2C1 + 0x08) & 0xFFFF != 0x0052 {
        return Err(b"i2c-oar1");
    }
    wr32(I2C1 + 0x10, 0x0070_3031); // TIMINGR
    if rd32(I2C1 + 0x10) != 0x0070_3031 {
        return Err(b"i2c-timingr");
    }
    wr32(I2C1 + 0x00, 0x1); // CR1.PE
    if rd32(I2C1 + 0x00) & 0x1 == 0 {
        return Err(b"i2c-pe");
    }
    wr32(I2C1 + 0x00, 0x0);
    wr32(I2C1 + 0x08, 0x0);
    wr32(I2C1 + 0x10, 0x0);
    Ok(())
}

/// wdt: IWDG register-access protocol — PR/RLR writes only take effect after
/// KR=0x5555 AND only once the LSI-domain sync completes (bench-probed: with
/// LSI off, SR.PVU stays 1 and the writes never commit). The check brings LSI
/// up via RCC_BDCR first and polls SR between steps, so it is faithful to
/// real silicon. The watchdog is never started (KR=0xCCCC is not written).
fn check_wdt() -> Result<(), &'static [u8]> {
    const IWDG: u32 = 0x4000_3000;
    const RCC_BDCR: u32 = RCC_BASE + 0xF0;
    if rd32(IWDG + 0x08) != 0xFFF {
        return Err(b"wdt-rlr-reset");
    }
    // LSI on (BDCR bit 26 → LSIRDY bit 27).
    let bdcr = rd32(RCC_BDCR);
    wr32(RCC_BDCR, bdcr | (1 << 26));
    for _ in 0..10_000 {
        if rd32(RCC_BDCR) & (1 << 27) != 0 {
            break;
        }
    }
    if rd32(RCC_BDCR) & (1 << 27) == 0 {
        return Err(b"wdt-lsirdy");
    }
    wr32(IWDG + 0x00, 0x5555); // enable register access
    wr32(IWDG + 0x04, 0x2); // PR
    for _ in 0..100_000 {
        if rd32(IWDG + 0x0C) & 0x1 == 0 {
            break; // PVU cleared — prescaler committed
        }
    }
    if rd32(IWDG + 0x04) != 0x2 {
        return Err(b"wdt-pr");
    }
    wr32(IWDG + 0x08, 0xABC);
    for _ in 0..100_000 {
        if rd32(IWDG + 0x0C) & 0x2 == 0 {
            break; // RVU cleared — reload committed
        }
    }
    if rd32(IWDG + 0x08) != 0xABC {
        return Err(b"wdt-rlr");
    }
    wr32(IWDG + 0x08, 0xFFF);
    wr32(IWDG + 0x04, 0x0);
    wr32(IWDG + 0x00, 0x0);
    wr32(RCC_BDCR, bdcr);
    Ok(())
}

#[entry]
fn main() -> ! {
    report(b"clock", check_clock());
    report(b"gpio", check_gpio());
    report(b"dma", check_dma());
    report(b"timer", check_timer());
    report(b"i2c", check_i2c());
    report(b"wdt", check_wdt());
    puts(b"TIER1 done\n");

    loop {
        spin(1_000_000);
    }
}
