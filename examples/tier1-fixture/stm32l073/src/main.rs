// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32L073 Tier-1 fixture firmware (Cortex-M0+, thumbv6m-none-eabi).
//!
//! Validates the simulator's chip model peripheral-by-peripheral with RAW
//! REGISTER accesses against the peripherals wired by
//! `configs/chips/stm32l073.yaml`, reporting one line per peripheral class
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
//! simulator's models: rcc.rs (`stm32l0` profile), gpio.rs (`stm32v2`),
//! uart.rs (`stm32v2`), timer.rs, dma.rs (Dma1) and nvic.rs.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::{entry, exception};
use panic_halt as _;

// ── Wired peripherals (configs/chips/stm32l073.yaml) ──────────────────────
const RCC_BASE: u32 = 0x4002_1000; // type rcc, profile stm32l0
const GPIOA_BASE: u32 = 0x5000_0000; // type gpio, profile stm32v2 (IOPORT bus)
const USART2_BASE: u32 = 0x4000_4400; // type uart, profile stm32v2
const TIM2_BASE: u32 = 0x4000_0000; // type timer, width 32
const DMA1_BASE: u32 = 0x4002_0000; // type dma (Dma1, 7ch)
const I2C1_BASE: u32 = 0x4000_5400; // type i2c, profile stm32l4
const SPI1_BASE: u32 = 0x4001_3000; // type spi, profile stm32 (classic)
const ADC1_BASE: u32 = 0x4001_2400; // type adc, profile stm32l4
const RTC_BASE: u32 = 0x4000_2800; // type rtc (stm32l4 layout)
const IWDG_BASE: u32 = 0x4000_3000; // type iwdg

// NVIC (declared in the yaml as `nvic`; shared state with the CPU core).
const NVIC_ISER0: u32 = 0xE000_E100;
const NVIC_ICER0: u32 = 0xE000_E180;
const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ICPR0: u32 = 0xE000_E280;
/// Software-pended test IRQ. Must be < 32 (cortex-m-rt default vector
/// table) and unused by any yaml peripheral — 5 is free on the L073 yaml.
const TEST_IRQ: i16 = 5;

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

/// clock: L0 RCC. MSI is on+ready out of reset (CR=0x300); HSI16ON (bit 0)
/// must latch HSI16RDY (bit 2); SW→SWS in CFGR @ 0x0C follows when the
/// source is ready; IOPENR @ 0x2C round-trips GPIO port enables.
fn check_clock() -> Result<(), &'static [u8]> {
    if rd32(RCC_BASE) & (1 << 9) == 0 {
        return Err(b"clock-msirdy");
    }
    let cr = rd32(RCC_BASE);
    wr32(RCC_BASE, cr | 1); // HSI16ON
    if rd32(RCC_BASE) & (1 << 2) == 0 {
        return Err(b"clock-hsi16rdy");
    }
    // Switch SYSCLK to HSI16 (SW=01); SWS must follow.
    wr32(RCC_BASE + 0x0C, 0x1);
    if (rd32(RCC_BASE + 0x0C) >> 2) & 0x3 != 0x1 {
        return Err(b"clock-sws");
    }
    // IOPENR round-trip: enable GPIOA/B/C port clocks.
    wr32(RCC_BASE + 0x2C, 0x7);
    if rd32(RCC_BASE + 0x2C) != 0x7 {
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
/// atomic RMW — thumbv6m has no LDREX/STREX); single-core, the handler
/// never races the check loop.
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

/// i2c: I2C1, modern (stm32l4) controller. ISR @ 0x18 resets with TXE (bit0).
/// PE (CR1.PE bit0) round-trips; CR2.START (bit13) latches ISR.BUSY (bit15);
/// CR2.STOP (bit14) clears it (i2c.rs L4I2c register-fidelity model).
fn check_i2c() -> Result<(), &'static [u8]> {
    if rd32(I2C1_BASE + 0x18) & 0x1 == 0 {
        return Err(b"i2c-txe-reset");
    }
    wr32(I2C1_BASE + 0x00, 1); // CR1.PE
    if rd32(I2C1_BASE + 0x00) & 0x1 == 0 {
        return Err(b"i2c-pe");
    }
    wr32(I2C1_BASE + 0x04, 1 << 13); // CR2.START → ISR.BUSY
    if rd32(I2C1_BASE + 0x18) & (1 << 15) == 0 {
        return Err(b"i2c-busy");
    }
    wr32(I2C1_BASE + 0x04, 1 << 14); // CR2.STOP → clear ISR.BUSY
    if rd32(I2C1_BASE + 0x18) & (1 << 15) != 0 {
        return Err(b"i2c-busy-stuck");
    }
    Ok(())
}

/// spi: SPI1 (classic stm32, no clock-gate field on L073). SR.TXE (bit1) is
/// set out of reset; enabling the master and writing one byte to DR (0x0C)
/// starts a frame — SR.BSY (bit7) sets, then the cycle-counted engine clears
/// BSY and re-asserts TXE on completion (spi.rs Stm32 transfer engine).
fn check_spi() -> Result<(), &'static [u8]> {
    if rd32(SPI1_BASE + 0x08) & (1 << 1) == 0 {
        return Err(b"spi-txe-reset");
    }
    // CR1: SPE(6) | MSTR(2) | SSM(9) | SSI(8) — master, software NSS high.
    wr32(SPI1_BASE + 0x00, (1 << 6) | (1 << 2) | (1 << 9) | (1 << 8));
    // Byte DR write kicks off one frame; word write would restart it four
    // times, so use an 8-bit store like the UART TX path.
    unsafe { write_volatile((SPI1_BASE + 0x0C) as *mut u8, 0xA5) };
    if rd32(SPI1_BASE + 0x08) & (1 << 7) == 0 {
        return Err(b"spi-bsy");
    }
    let mut done = false;
    for _ in 0..20_000 {
        if rd32(SPI1_BASE + 0x08) & (1 << 7) == 0 {
            done = true;
            break;
        }
    }
    if !done {
        return Err(b"spi-bsy-stuck");
    }
    if rd32(SPI1_BASE + 0x08) & (1 << 1) == 0 {
        return Err(b"spi-txe");
    }
    wr32(SPI1_BASE + 0x00, 0); // disable
    Ok(())
}

/// adc: ADC1 (stm32l4 model). The L4 ADC model has NO conversion engine
/// (EOC/DR conversion is F1-only), so this exercises the genuinely-modelled
/// power-up: CR resets with DEEPPWD (bit29); clearing it, setting ADVREGEN
/// (bit28) then ADEN (bit0) raises ISR.ADRDY (bit0), which must NOT assert
/// before ADEN.
fn check_adc() -> Result<(), &'static [u8]> {
    wr32(ADC1_BASE + 0x08, 0); // CR: clear DEEPPWD
    wr32(ADC1_BASE + 0x08, 1 << 28); // CR: ADVREGEN
    if rd32(ADC1_BASE + 0x00) & 0x1 != 0 {
        return Err(b"adc-adrdy-early");
    }
    wr32(ADC1_BASE + 0x08, (1 << 28) | 1); // CR: ADVREGEN | ADEN
    if rd32(ADC1_BASE + 0x00) & 0x1 == 0 {
        return Err(b"adc-adrdy");
    }
    Ok(())
}

/// wdt: IWDG. PR/RLR are write-protected until KR (0x00) gets the 0x5555
/// unlock and re-protect on any other code; reset PR=0, RLR=0x0FFF
/// (iwdg.rs write-access gate).
fn check_wdt() -> Result<(), &'static [u8]> {
    if rd32(IWDG_BASE + 0x04) != 0 || rd32(IWDG_BASE + 0x08) != 0x0FFF {
        return Err(b"wdt-reset");
    }
    // Without the 0x5555 unlock, PR/RLR writes are dropped.
    wr32(IWDG_BASE + 0x04, 0x5);
    wr32(IWDG_BASE + 0x08, 0x123);
    if rd32(IWDG_BASE + 0x04) != 0 || rd32(IWDG_BASE + 0x08) != 0x0FFF {
        return Err(b"wdt-unprotected");
    }
    // Unlock → PR/RLR latch.
    wr32(IWDG_BASE + 0x00, 0x5555);
    wr32(IWDG_BASE + 0x04, 0x5);
    wr32(IWDG_BASE + 0x08, 0x123);
    if rd32(IWDG_BASE + 0x04) != 0x5 || rd32(IWDG_BASE + 0x08) != 0x123 {
        return Err(b"wdt-latch");
    }
    // Any other KR code (0xAAAA reload) re-protects.
    wr32(IWDG_BASE + 0x00, 0xAAAA);
    wr32(IWDG_BASE + 0x04, 0x2);
    if rd32(IWDG_BASE + 0x04) != 0x5 {
        return Err(b"wdt-reprotect");
    }
    Ok(())
}

/// rtc: RTC (stm32l4 layout). DR resets to 0x2101; the write-protect state
/// machine half-unlocks on WPR=0xCA (readable back) then unlocks on 0x53, and
/// TR round-trips under its 0x007F7F7F writable mask (rtc.rs).
fn check_rtc() -> Result<(), &'static [u8]> {
    if rd32(RTC_BASE + 0x04) != 0x0000_2101 {
        return Err(b"rtc-dr-reset");
    }
    // WPR is byte-accessed: 0xCA half-unlocks (latches, readable), 0x53 unlocks.
    unsafe { write_volatile((RTC_BASE + 0x24) as *mut u8, 0xCA) };
    if rd32(RTC_BASE + 0x24) & 0xFF != 0xCA {
        return Err(b"rtc-wpr");
    }
    unsafe { write_volatile((RTC_BASE + 0x24) as *mut u8, 0x53) };
    wr32(RTC_BASE + 0x00, 0x0012_3456); // TR
    if rd32(RTC_BASE + 0x00) != 0x0012_3456 {
        return Err(b"rtc-tr");
    }
    Ok(())
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
    report(b"i2c", check_i2c());
    report(b"spi", check_spi());
    report(b"adc", check_adc());
    report(b"wdt", check_wdt());
    report(b"rtc", check_rtc());
    puts(b"TIER1 done\n");

    loop {
        spin(1_000_000);
    }
}
