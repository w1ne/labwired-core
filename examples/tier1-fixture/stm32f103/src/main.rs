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
const I2C1_BASE: u32 = 0x4000_5400; // type i2c, stm32f1 layout (default)
const SPI1_BASE: u32 = 0x4001_3000; // type spi, stm32 classic layout (default)
const ADC1_BASE: u32 = 0x4001_2400; // type adc, stm32f1 layout (default)
const IWDG_BASE: u32 = 0x4000_3000; // type iwdg
const RTC_BASE: u32 = 0x4000_2800; // type rtc_f1

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

/// i2c: F1 legacy I2C1. Enable (CR1.PE) then request a START (CR1.START,
/// bit 8); the transaction state machine must latch SR1.SB (bit 0) after a
/// bounded number of ticks, then a STOP (CR1.STOP, bit 9) releases the bus.
fn check_i2c() -> Result<(), &'static [u8]> {
    wr32(I2C1_BASE, 1); // CR1.PE @ 0x00
    wr32(I2C1_BASE, (1 << 8) | 1); // CR1: START + PE
    let mut sb = false;
    for _ in 0..20_000 {
        if rd32(I2C1_BASE + 0x14) & 0x1 != 0 {
            // SR1.SB @ 0x14
            sb = true;
            break;
        }
    }
    if !sb {
        return Err(b"i2c-sb");
    }
    wr32(I2C1_BASE, (1 << 9) | 1); // CR1: STOP + PE
    Ok(())
}

/// spi: classic SPI1. TXE (SR bit 1) is set out of reset. With SPE (CR1
/// bit 6) + MSTR (bit 2) + software-NSS (SSM bit 9, SSI bit 8), a DR write
/// kicks off a shift-register transfer: BSY (SR bit 7) latches immediately
/// and the cycle-counted engine clears it / re-asserts TXE on completion.
fn check_spi() -> Result<(), &'static [u8]> {
    if rd32(SPI1_BASE + 0x08) & (1 << 1) == 0 {
        return Err(b"spi-txe-reset"); // SR.TXE @ 0x08
    }
    wr32(SPI1_BASE, (1 << 6) | (1 << 2) | (1 << 9) | (1 << 8)); // CR1: SPE|MSTR|SSM|SSI
    unsafe { write_volatile((SPI1_BASE + 0x0C) as *mut u8, 0xAB) }; // DR @ 0x0C → start transfer
    if rd32(SPI1_BASE + 0x08) & (1 << 7) == 0 {
        return Err(b"spi-bsy-set"); // BSY must be high while the frame shifts
    }
    let mut done = false;
    for _ in 0..20_000 {
        let sr = rd32(SPI1_BASE + 0x08);
        if sr & (1 << 7) == 0 && sr & (1 << 1) != 0 {
            // BSY clear + TXE set
            done = true;
            break;
        }
    }
    if !done {
        return Err(b"spi-bsy-stuck");
    }
    Ok(())
}

/// adc: F1 ADC1. ADON (CR2 bit 0) powers the converter; a rising SWSTART
/// (CR2 bit 30) launches a regular conversion. The engine latches EOC
/// (SR bit 1) after its fixed conversion time and writes the result to DR.
fn check_adc() -> Result<(), &'static [u8]> {
    wr32(ADC1_BASE + 0x08, 1); // CR2.ADON @ 0x08
    spin(100); // converter wake-up
    wr32(ADC1_BASE + 0x08, 1 | (1 << 30)); // CR2: ADON + SWSTART (rising edge)
    let mut eoc = false;
    for _ in 0..20_000 {
        if rd32(ADC1_BASE) & (1 << 1) != 0 {
            // SR.EOC @ 0x00
            eoc = true;
            break;
        }
    }
    if !eoc {
        return Err(b"adc-eoc");
    }
    if rd32(ADC1_BASE + 0x4C) & 0xFFF == 0 {
        return Err(b"adc-dr"); // DR @ 0x4C must hold the converted count
    }
    Ok(())
}

/// wdt: IWDG. PR/RLR are write-protected until KR (@ 0x00) receives the
/// 0x5555 unlock code (RM0008 §19.4): a pre-unlock RLR write is dropped
/// (RLR keeps its 0x0FFF reset), and after unlock PR/RLR round-trip.
fn check_wdt() -> Result<(), &'static [u8]> {
    wr32(IWDG_BASE + 0x08, 0x123); // RLR @ 0x08 without key → dropped
    if rd32(IWDG_BASE + 0x08) != 0xFFF {
        return Err(b"wdt-unprotected");
    }
    wr32(IWDG_BASE, 0x5555); // KR unlock
    wr32(IWDG_BASE + 0x04, 0x5); // PR @ 0x04
    wr32(IWDG_BASE + 0x08, 0x123); // RLR @ 0x08
    if rd32(IWDG_BASE + 0x04) != 0x5 {
        return Err(b"wdt-pr");
    }
    if rd32(IWDG_BASE + 0x08) != 0x123 {
        return Err(b"wdt-rlr");
    }
    Ok(())
}

/// rtc: F1 RTC. CRL.RTOFF (bit 5) stays asserted while idle, and the 32-bit
/// counter round-trips through its CNTH/CNTL half-registers (RM0008 §18).
fn check_rtc() -> Result<(), &'static [u8]> {
    if rd32(RTC_BASE + 0x04) & 0x20 == 0 {
        return Err(b"rtc-rtoff"); // CRL @ 0x04
    }
    wr32(RTC_BASE + 0x18, 0xBEEF); // CNTH @ 0x18
    wr32(RTC_BASE + 0x1C, 0x1234); // CNTL @ 0x1C
    if rd32(RTC_BASE + 0x18) & 0xFFFF != 0xBEEF {
        return Err(b"rtc-cnth");
    }
    if rd32(RTC_BASE + 0x1C) & 0xFFFF != 0x1234 {
        return Err(b"rtc-cntl");
    }
    Ok(())
}

#[entry]
fn main() -> ! {
    report(b"clock", check_clock());
    report(b"gpio", check_gpio());
    report(b"timer", check_timer());
    report(b"i2c", check_i2c());
    report(b"spi", check_spi());
    report(b"adc", check_adc());
    report(b"dma", check_dma());
    report(b"wdt", check_wdt());
    report(b"rtc", check_rtc());
    puts(b"TIER1 done\n");

    loop {
        spin(1_000_000);
    }
}
