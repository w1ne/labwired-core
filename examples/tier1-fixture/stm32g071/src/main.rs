// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32G071RB Tier-1 fixture firmware (Cortex-M0+, thumbv6m-none-eabi).
//!
//! Validates the simulator's chip model peripheral-by-peripheral with RAW
//! REGISTER accesses against the peripherals wired by
//! `configs/chips/stm32g071.yaml`, reporting one line per peripheral class
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
//! The console bring-up is itself a check: USART2 is clock-gated on
//! RCC_APBENR1.USART2EN (bit 17) and must read dead until that bit is set.
//!
//! Every gated peripheral is FIRST poked while its RCC clock-enable bit is
//! off and required to read dead/0 — proving the G0 clock gate is modelled —
//! then its bit is enabled before the behavioural round-trip. iwdg is ungated
//! on silicon (LSI clock, no RCC enable bit), so it is a pure behavioural
//! check.
//!
//! Every poll is bounded by a fixed iteration count (the simulator is
//! deterministic — no wall-clock timeouts). Register offsets follow RM0444
//! (STM32G0x1) and the simulator's models: rcc.rs (`stm32g0`), gpio.rs
//! (`stm32v2`), uart.rs (`stm32v2`), timer.rs, i2c.rs (`stm32l4`),
//! spi.rs (classic `stm32`), adc.rs (`stm32g0`), dma.rs (Dma1), rtc.rs,
//! iwdg.rs and nvic.rs.
//!
//! Cortex-M0+ note: reading the full 32-bit status word and bit-testing it
//! avoids byte loads with sign tests, which compile to `LDRSB` reg-offset —
//! the simulator's 16-bit Thumb decoder does not implement that form.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::{entry, exception};
use panic_halt as _;
use tier1_fixture_common::{rd32, spin, wr32, Console};

// ── Wired peripherals (configs/chips/stm32g071.yaml) ──────────────────────
const RCC_BASE: u32 = 0x4002_1000; // type rcc, profile stm32g0
const GPIOA_BASE: u32 = 0x5000_0000; // type gpio, profile stm32v2
const GPIOC_BASE: u32 = 0x5000_0800; // type gpio, profile stm32v2
const USART2_BASE: u32 = 0x4000_4400; // type uart, profile stm32v2 (console)
const TIM2_BASE: u32 = 0x4000_0000; // type timer, width 32
const TIM1_BASE: u32 = 0x4001_2C00; // type timer, advanced (id tim1_pwm)
const I2C1_BASE: u32 = 0x4000_5400; // type i2c, profile stm32l4
const SPI1_BASE: u32 = 0x4001_3000; // type spi, profile stm32 (FIFO-less)
const ADC1_BASE: u32 = 0x4001_2400; // type adc, profile stm32g0
const DMA1_BASE: u32 = 0x4002_0000; // type dma (Dma1, 7ch)
const IWDG_BASE: u32 = 0x4000_3000; // type iwdg
const RTC_BASE: u32 = 0x4000_2800; // type rtc (stm32l4 layout)

// G0 RCC map — RM0444 §5.4 / CMSIS stm32g071xx.h. NOT the L0 or G4 map:
// CR @0x00 (HSION bit8 → HSIRDY bit10, HSEON bit16 → HSERDY bit17),
// CFGR @0x08 (3-bit SW[2:0]/SWS[5:3]), IOPENR@0x34, AHBENR@0x38,
// APBENR1@0x3C, APBENR2@0x40.
const RCC_CFGR: u32 = RCC_BASE + 0x08;
const RCC_IOPENR: u32 = RCC_BASE + 0x34;
const RCC_AHBENR: u32 = RCC_BASE + 0x38;
const RCC_APBENR1: u32 = RCC_BASE + 0x3C;
const RCC_APBENR2: u32 = RCC_BASE + 0x40;

// NVIC (installed for every Cortex-M chip; declared in the yaml as `nvic`).
const NVIC_ISER0: u32 = 0xE000_E100;
const NVIC_ICER0: u32 = 0xE000_E180;
const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ICPR0: u32 = 0xE000_E280;
/// Software-pended test IRQ. Must be < 32 (cortex-m-rt's ARMv6-M vector table
/// carries 32 external slots) and unused by any wired peripheral. G071 uses
/// 0, 2, 12, 13, 15..29 in that range — 30 is free (stm32g071xx.h IRQn_Type).
const TEST_IRQ: i16 = 30;

// USART2, stm32v2 layout: ISR @ 0x1C (TXE = bit 7), TDR @ 0x28.
const CONSOLE: Console = Console::new(USART2_BASE + 0x1C, USART2_BASE + 0x28, 1 << 7);

// ── Console bring-up (itself the implicit uart proof) ──────────────────────

/// Did the USART2 clock gate hold up: dead while RCC_APBENR1.USART2EN (bit 17)
/// is off, TXE present once it is on? Recorded by [`console_bringup`] and
/// checked first by `check_clock`, so a model that ignores the declared gate
/// is REPORTED (`TIER1 clock FAIL code=clock-usart2-gate`) instead of
/// silently darkening the console.
static mut CONSOLE_GATE_OK: bool = false;

/// Bring up USART2 as a polled console. The gate verdict lands in
/// [`CONSOLE_GATE_OK`]; the console is configured either way so the protocol
/// lines still have somewhere to go.
fn console_bringup() {
    // Dead while gated: the register file reads 0 and TXE cannot be observed.
    let dead_before = rd32(USART2_BASE + 0x1C) == 0;
    // HSION is on/ready out of reset (RM0444 §5.4.1); re-assert.
    wr32(RCC_BASE, rd32(RCC_BASE) | (1 << 8));
    wr32(RCC_APBENR1, rd32(RCC_APBENR1) | (1 << 17)); // USART2EN
    let txe_after = rd32(USART2_BASE + 0x1C) & (1 << 7) != 0;
    unsafe {
        CONSOLE_GATE_OK = dead_before && txe_after;
    }
    // 115200 8N1 against the 16 MHz HSISYS kernel clock (RM0444 §5.2.4).
    wr32(USART2_BASE + 0x0C, 16_000_000 / 115_200); // BRR
    wr32(USART2_BASE + 0x00, (1 << 0) | (1 << 3)); // UE | TE
}

// ── Checks ──────────────────────────────────────────────────────────────────

/// clock: RCC `stm32g0` layout (RM0444 §5.4). CR ready bits, the 3-bit
/// CFGR.SW/SWS switch (reserved encodings must not move SWS), and the
/// IOPENR/AHBENR/APBENR1/APBENR2 enable registers. The gate proof uses GPIOC
/// (IOPENR bit2): dead until enabled, then a real ODR round-trip. The ENR
/// probes set AND clear their bit so the later class checks still see their
/// own peripheral gated.
fn check_clock() -> Result<(), &'static [u8]> {
    // The console's own gate proof (APBENR1.USART2EN bit17) must have held:
    // dead before the enable, TXE after it.
    if !unsafe { read_volatile(core::ptr::addr_of!(CONSOLE_GATE_OK)) } {
        return Err(b"clock-usart2-gate");
    }
    let cr = rd32(RCC_BASE);
    if cr & (1 << 8) == 0 {
        return Err(b"clock-hsion");
    }
    if cr & (1 << 10) == 0 {
        return Err(b"clock-hsirdy");
    }
    // HSEON bit16 → HSERDY bit17 (the G0 CR ready rule).
    wr32(RCC_BASE, cr | (1 << 16));
    if rd32(RCC_BASE) & (1 << 17) == 0 {
        return Err(b"clock-hserdy");
    }
    // SW is a 3-bit field: 001 = HSE. SWS[5:3] follows only once ready.
    wr32(RCC_CFGR, 0x1);
    if (rd32(RCC_CFGR) >> 3) & 0x7 != 0x1 {
        return Err(b"clock-sws");
    }
    // Reserved SW encodings (101) must not move SWS.
    wr32(RCC_CFGR, 0x5);
    if (rd32(RCC_CFGR) >> 3) & 0x7 != 0x1 {
        return Err(b"clock-sws-reserved");
    }
    wr32(RCC_CFGR, 0x0); // back to HSISYS
    if (rd32(RCC_CFGR) >> 3) & 0x7 != 0x0 {
        return Err(b"clock-sws-hsi");
    }
    wr32(RCC_BASE, cr & !(1 << 16)); // drop HSE
    if rd32(RCC_BASE) & (1 << 17) != 0 {
        return Err(b"clock-hserdy-stuck");
    }
    // IOPENR bit2 (GPIOC) gates a real port: dead until enabled.
    wr32(GPIOC_BASE + 0x14, 0x0020); // ODR write while gated is dropped
    if rd32(GPIOC_BASE + 0x14) != 0 {
        return Err(b"clock-gate-open");
    }
    wr32(RCC_IOPENR, rd32(RCC_IOPENR) | (1 << 2));
    if rd32(RCC_IOPENR) & (1 << 2) == 0 {
        return Err(b"clock-iopenr");
    }
    wr32(GPIOC_BASE + 0x14, 0x0020);
    if rd32(GPIOC_BASE + 0x14) != 0x0020 {
        return Err(b"clock-gate-open-en");
    }
    // AHBENR / APBENR1 / APBENR2 round-trip, set then clear (bit 0 of APBENR1
    // is TIM2EN, APBENR2 bit11 is TIM1EN; both are probed gated by their class
    // checks later, so leave them off).
    let ahbenr = rd32(RCC_AHBENR);
    wr32(RCC_AHBENR, ahbenr | 1);
    if rd32(RCC_AHBENR) & 1 == 0 {
        return Err(b"clock-ahbenr");
    }
    wr32(RCC_AHBENR, ahbenr);
    if rd32(RCC_AHBENR) & 1 != 0 {
        return Err(b"clock-ahbenr-clear");
    }
    let apbenr1 = rd32(RCC_APBENR1);
    wr32(RCC_APBENR1, apbenr1 | 1);
    if rd32(RCC_APBENR1) & 1 == 0 {
        return Err(b"clock-apbenr1");
    }
    wr32(RCC_APBENR1, apbenr1);
    if rd32(RCC_APBENR1) & 1 != 0 {
        return Err(b"clock-apbenr1-clear");
    }
    let apbenr2 = rd32(RCC_APBENR2);
    wr32(RCC_APBENR2, apbenr2 | (1 << 11));
    if rd32(RCC_APBENR2) & (1 << 11) == 0 {
        return Err(b"clock-apbenr2");
    }
    wr32(RCC_APBENR2, apbenr2);
    if rd32(RCC_APBENR2) & (1 << 11) != 0 {
        return Err(b"clock-apbenr2-clear");
    }
    Ok(())
}

/// gpio: stm32v2 port on the IOPORT bus. GPIOA is gated on IOPENR bit0;
/// MODER/OTYPER round-trip, BSRR set (ODR + IDR follow for a push-pull
/// output), BRR and BSRR-high reset clear, ODR direct write.
fn check_gpio() -> Result<(), &'static [u8]> {
    if rd32(GPIOA_BASE) != 0 {
        return Err(b"gpio-gated");
    }
    wr32(GPIOA_BASE, 0x0000_0400); // PA5 output — dropped while gated
    if rd32(GPIOA_BASE) != 0 {
        return Err(b"gpio-gated-write");
    }
    wr32(RCC_IOPENR, rd32(RCC_IOPENR) | (1 << 0)); // GPIOAEN
    let moder = rd32(GPIOA_BASE);
    wr32(GPIOA_BASE, (moder & !(0x3 << 10)) | (0x1 << 10)); // PA5 = output
    if rd32(GPIOA_BASE) & (0x3 << 10) != (0x1 << 10) {
        return Err(b"gpio-moder");
    }
    wr32(GPIOA_BASE + 0x04, 1 << 6); // OTYPER: PA6 open-drain
    if rd32(GPIOA_BASE + 0x04) & (1 << 6) == 0 {
        return Err(b"gpio-otyper");
    }
    if rd32(GPIOA_BASE + 0x04) & (1 << 5) != 0 {
        return Err(b"gpio-otyper-5");
    }
    // BSRR sets PA5; a push-pull output drives the pin, so IDR follows ODR.
    wr32(GPIOA_BASE + 0x18, 1 << 5);
    if rd32(GPIOA_BASE + 0x14) & (1 << 5) == 0 {
        return Err(b"gpio-bsrr");
    }
    if rd32(GPIOA_BASE + 0x10) & (1 << 5) == 0 {
        return Err(b"gpio-idr");
    }
    wr32(GPIOA_BASE + 0x28, 1 << 5); // BRR clears
    if rd32(GPIOA_BASE + 0x14) & (1 << 5) != 0 {
        return Err(b"gpio-brr");
    }
    // ODR direct write on PA6 (now an output), then BSRR high half resets it.
    wr32(GPIOA_BASE, (rd32(GPIOA_BASE) & !(0x3 << 12)) | (0x1 << 12)); // PA6 = output
    wr32(GPIOA_BASE + 0x14, 1 << 6);
    if rd32(GPIOA_BASE + 0x14) & (1 << 6) == 0 {
        return Err(b"gpio-odr");
    }
    wr32(GPIOA_BASE + 0x18, 1 << (6 + 16));
    if rd32(GPIOA_BASE + 0x14) & (1 << 6) != 0 {
        return Err(b"gpio-bsrr-reset");
    }
    Ok(())
}

/// timer: TIM2 (32-bit GP timer, APBENR1 bit0). Gated it reads dead; enabled,
/// ARR round-trips 32-bit, EGR.UG latches UIF, SR is rc_w0, CEN makes the
/// counter advance.
fn check_timer() -> Result<(), &'static [u8]> {
    wr32(TIM2_BASE + 0x2C, 0x1234); // ARR while gated
    if rd32(TIM2_BASE + 0x2C) != 0 {
        return Err(b"tim-gated");
    }
    wr32(RCC_APBENR1, rd32(RCC_APBENR1) | (1 << 0)); // TIM2EN
    wr32(TIM2_BASE + 0x28, 0); // PSC = 0
    wr32(TIM2_BASE + 0x2C, 0xFFFF_FFFF); // ARR = max (32-bit TIM2)
    if rd32(TIM2_BASE + 0x2C) != 0xFFFF_FFFF {
        return Err(b"tim-arr32");
    }
    wr32(TIM2_BASE + 0x14, 1); // EGR.UG
    if rd32(TIM2_BASE + 0x10) & 1 == 0 {
        return Err(b"tim-uif");
    }
    wr32(TIM2_BASE + 0x10, 0); // SR: rc_w0 clear
    if rd32(TIM2_BASE + 0x10) & 1 != 0 {
        return Err(b"tim-uif-clear");
    }
    wr32(TIM2_BASE, 1); // CR1.CEN
    let c1 = rd32(TIM2_BASE + 0x24);
    spin(2_000);
    let c2 = rd32(TIM2_BASE + 0x24);
    wr32(TIM2_BASE, 0); // stop
    if c2 == c1 {
        return Err(b"tim-cnt-stuck");
    }
    Ok(())
}

/// pwm: TIM1 advanced 16-bit (APBENR2 bit11). UG latches the compare-match
/// flags for every channel whose CCR equals the reloaded CNT (CCR1=50 does
/// not match CNT=0); the running counter raises CC1IF when it crosses 50.
fn check_pwm() -> Result<(), &'static [u8]> {
    wr32(TIM1_BASE + 0x2C, 0x100);
    if rd32(TIM1_BASE + 0x2C) != 0 {
        return Err(b"pwm-gated");
    }
    wr32(RCC_APBENR2, rd32(RCC_APBENR2) | (1 << 11)); // TIM1EN
    wr32(TIM1_BASE + 0x18, 0x0068); // CCMR1: OC1M=PWM1, OC1PE
    wr32(TIM1_BASE + 0x20, 0x0001); // CCER: CC1E
    wr32(TIM1_BASE + 0x28, 0); // PSC
    wr32(TIM1_BASE + 0x2C, 100); // ARR
    wr32(TIM1_BASE + 0x34, 50); // CCR1
    wr32(TIM1_BASE + 0x44, 0x8000); // BDTR.MOE
    wr32(TIM1_BASE + 0x14, 0x1); // EGR.UG
    let sr = rd32(TIM1_BASE + 0x10);
    // UIF + CC2..4IF (CCR2..4=0 match the reloaded CNT=0) + CC5IF/CC6IF.
    if sr & 0x0003_001D != 0x0003_001D {
        return Err(b"pwm-ug-latch");
    }
    if sr & 0x2 != 0 {
        return Err(b"pwm-cc1-early"); // CCR1=50 != 0: must NOT match at UG
    }
    wr32(TIM1_BASE + 0x10, 0); // clear SR
    wr32(TIM1_BASE, 0x1); // CEN
    let mut hit = false;
    for _ in 0..20_000 {
        if rd32(TIM1_BASE + 0x10) & 0x2 != 0 {
            hit = true;
            break;
        }
    }
    wr32(TIM1_BASE, 0); // CEN off
    if !hit {
        return Err(b"pwm-cc1if");
    }
    Ok(())
}

/// dma: DMA1 channel 1 mem-to-mem copy (CCR.MEM2MEM, CMAR → CPAR), byte
/// elements with MINC+PINC. Gated on AHBENR bit0, so the channel registers
/// must read dead before the enable and TCIF1 must latch with matching data.
fn check_dma() -> Result<(), &'static [u8]> {
    const N: usize = 8;
    let src: [u8; N] = [0xA5, 0x5A, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
    let mut dst: [u8; N] = [0; N];

    wr32(DMA1_BASE + 0x0C, N as u32); // CNDTR1 while gated
    if rd32(DMA1_BASE + 0x0C) != 0 {
        return Err(b"dma-gated");
    }
    wr32(RCC_AHBENR, rd32(RCC_AHBENR) | (1 << 0)); // DMA1EN

    wr32(DMA1_BASE + 0x04, 0xF); // IFCR: clear stale CH1 flags
    wr32(DMA1_BASE + 0x10, dst.as_mut_ptr() as u32); // CPAR1 = destination
    wr32(DMA1_BASE + 0x14, src.as_ptr() as u32); // CMAR1 = source
    wr32(DMA1_BASE + 0x0C, N as u32); // CNDTR1
                                      // Configure first WITHOUT EN, then flip EN alone.
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

/// Hit counter for the software-pended test IRQ.
static mut IRQ_HITS: u32 = 0;

/// irq: NVIC delivery round-trip. Enable TEST_IRQ in ISER0, software-pend it
/// via ISPR0, and require the vector to actually run.
fn check_irq() -> Result<(), &'static [u8]> {
    wr32(NVIC_ISER0, 1 << TEST_IRQ as u32);
    wr32(NVIC_ISPR0, 1 << TEST_IRQ as u32);
    for _ in 0..10_000 {
        if unsafe { read_volatile(core::ptr::addr_of!(IRQ_HITS)) } != 0 {
            return Ok(());
        }
    }
    wr32(NVIC_ICER0, 1 << TEST_IRQ as u32);
    wr32(NVIC_ICPR0, 1 << TEST_IRQ as u32);
    Err(b"irq-not-delivered")
}

/// i2c: I2C1 (v2 TIMINGR controller, APBENR1 bit21). Gated ISR reads 0;
/// enabled, ISR resets with TXE; PE round-trips; CR2.START latches
/// ISR.BUSY; CR2.STOP clears it; a 1-byte master write to an ABSENT slave
/// NACKs and AUTOEND releases the bus.
fn check_i2c() -> Result<(), &'static [u8]> {
    if rd32(I2C1_BASE + 0x18) != 0 {
        return Err(b"i2c-gated");
    }
    wr32(RCC_APBENR1, rd32(RCC_APBENR1) | (1 << 21)); // I2C1EN
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
    // Transaction engine: a 1-byte master write to an ABSENT slave (addr 0x52)
    // must drive the address+data phase and NACK. CR2 = SADD(0x52<<1) |
    // NBYTES(1)<<16 | AUTOEND<<25 | START<<13; the TXDR byte arms the phase.
    let cr2 = (0x52u32 << 1) | (1 << 16) | (1 << 25) | (1 << 13);
    wr32(I2C1_BASE + 0x04, cr2);
    unsafe { write_volatile((I2C1_BASE + 0x28) as *mut u8, 0xAB) }; // TXDR
    let mut nacked = false;
    for _ in 0..20_000 {
        if rd32(I2C1_BASE + 0x18) & (1 << 4) != 0 {
            nacked = true; // ISR.NACKF
            break;
        }
    }
    if !nacked {
        return Err(b"i2c-no-nack");
    }
    if rd32(I2C1_BASE + 0x18) & (1 << 15) != 0 {
        return Err(b"i2c-autoend-busy"); // AUTOEND must release the bus
    }
    wr32(I2C1_BASE + 0x1C, (1 << 4) | (1 << 5)); // ICR: NACKCF | STOPCF
    if rd32(I2C1_BASE + 0x18) & (1 << 4) != 0 {
        return Err(b"i2c-nack-stuck");
    }
    wr32(I2C1_BASE + 0x00, 0);
    Ok(())
}

/// spi: SPI1 (classic FIFO-less `stm32` profile, APBENR2 bit12). Gated SR
/// reads 0; enabled, SR.TXE (bit1) asserts and a byte DR write sets SR.BSY
/// (bit7); completion clears BSY, re-asserts TXE and raises RXNE (bit0).
fn check_spi() -> Result<(), &'static [u8]> {
    if rd32(SPI1_BASE + 0x08) != 0 {
        return Err(b"spi-gated");
    }
    wr32(RCC_APBENR2, rd32(RCC_APBENR2) | (1 << 12)); // SPI1EN
    if rd32(SPI1_BASE + 0x08) & (1 << 1) == 0 {
        return Err(b"spi-txe-reset");
    }
    // CR1: SPE(6) | MSTR(2) | SSM(9) | SSI(8) — master, software NSS high.
    wr32(SPI1_BASE + 0x00, (1 << 6) | (1 << 2) | (1 << 9) | (1 << 8));
    unsafe { write_volatile((SPI1_BASE + 0x0C) as *mut u8, 0xA5) }; // byte DR write
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
    // Classic SPI completes the receive on every frame: RXNE latches once the
    // byte has shifted out (no slave attached — the captured level is the
    // idle line; the flag, not the value, is the proof).
    if rd32(SPI1_BASE + 0x08) & 0x1 == 0 {
        return Err(b"spi-rxne");
    }
    wr32(SPI1_BASE + 0x00, 0); // disable
    Ok(())
}

/// Run one ADC1 single conversion at CFGR.RES = `res`, returning DR. The model
/// converts a fixed internal source (V(IN)=3.0 V, V(REF+)=3.3 V): the 12-bit
/// code is (3.0/3.3)*4096=3723, narrower resolutions drop LSBs.
fn adc_convert(res: u32) -> Result<u32, &'static [u8]> {
    let cfgr = rd32(ADC1_BASE + 0x0C) & !(0x3 << 3);
    wr32(ADC1_BASE + 0x0C, cfgr | (res << 3));
    // G0 CHSELR (bitmap mode): a conversion needs a selected channel. IN0 has
    // no held input in the Tier-1 run, so it converts the fixed source.
    wr32(ADC1_BASE + 0x28, 1 << 0);
    wr32(ADC1_BASE + 0x00, 1 << 2); // ISR rc_w1: clear any stale EOC
    wr32(ADC1_BASE + 0x08, rd32(ADC1_BASE + 0x08) | (1 << 2)); // CR.ADSTART
    let mut eoc = false;
    for _ in 0..20_000 {
        if rd32(ADC1_BASE + 0x00) & (1 << 2) != 0 {
            eoc = true;
            break;
        }
    }
    if !eoc {
        return Err(b"adc-eoc");
    }
    Ok(rd32(ADC1_BASE + 0x40) & 0xFFFF)
}

/// adc: ADC1 (stm32g0 layout, APBENR2 bit20). Gated the CR reads 0. After
/// ungating + power-up (clear DEEPPWD, ADVREGEN, ADEN → ISR.ADRDY; ADRDY
/// must NOT assert before ADEN), prove a REAL conversion BY VALUE: ADSTART
/// converts the fixed internal source and the code must scale when CFGR.RES
/// narrows. Fails if the model returned a constant.
fn check_adc() -> Result<(), &'static [u8]> {
    if rd32(ADC1_BASE + 0x08) != 0 {
        return Err(b"adc-gated");
    }
    wr32(RCC_APBENR2, rd32(RCC_APBENR2) | (1 << 20)); // ADC1EN at APBENR2 bit20
    wr32(ADC1_BASE + 0x08, 0); // CR: clear DEEPPWD
    wr32(ADC1_BASE + 0x08, 1 << 28); // CR: ADVREGEN
    if rd32(ADC1_BASE + 0x00) & 0x1 != 0 {
        return Err(b"adc-adrdy-early");
    }
    wr32(ADC1_BASE + 0x08, (1 << 28) | 1); // CR: ADVREGEN | ADEN
    if rd32(ADC1_BASE + 0x00) & 0x1 == 0 {
        return Err(b"adc-adrdy");
    }
    let code12 = adc_convert(0)?;
    if code12 != 3723 {
        return Err(b"adc-code12");
    }
    let code10 = adc_convert(1)?;
    if code10 != 930 {
        return Err(b"adc-code10");
    }
    if code10 >= code12 {
        return Err(b"adc-scale");
    }
    Ok(())
}

/// wdt: IWDG. Ungated (clocked by the LSI on silicon — no RCC enable bit).
/// PR/RLR are write-protected until KR (0x00) gets the 0x5555 unlock and
/// re-protect on any other code; reset PR=0, RLR=0x0FFF.
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

/// rtc: RTC (stm32l4 layout), register interface clock-gated on
/// RCC_APBENR1.RTCAPBEN (bit 10). Gated DR reads 0; enabled, DR resets to
/// 0x2101; WPR half-unlocks on 0xCA then unlocks on 0x53, and TR round-trips.
fn check_rtc() -> Result<(), &'static [u8]> {
    if rd32(RTC_BASE + 0x04) != 0 {
        return Err(b"rtc-gated");
    }
    wr32(RCC_APBENR1, rd32(RCC_APBENR1) | (1 << 10)); // RTCAPBEN
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
        // Disarm first so a re-pend can't wedge the main thread.
        wr32(NVIC_ICER0, 1 << TEST_IRQ as u32);
        wr32(NVIC_ICPR0, 1 << TEST_IRQ as u32);
        let hits = read_volatile(core::ptr::addr_of!(IRQ_HITS));
        write_volatile(core::ptr::addr_of_mut!(IRQ_HITS), hits + 1);
    }
}

#[entry]
fn main() -> ! {
    // The console must be alive before any TIER1 line can be reported; its
    // bring-up (APBENR1.USART2EN gate) is the implicit uart proof and its
    // gate verdict is the first thing `check_clock` asserts.
    console_bringup();
    CONSOLE.report(b"clock", check_clock());
    CONSOLE.report(b"gpio", check_gpio());
    CONSOLE.report(b"timer", check_timer());
    CONSOLE.report(b"pwm", check_pwm());
    CONSOLE.report(b"dma", check_dma());
    CONSOLE.report(b"irq", check_irq());
    CONSOLE.report(b"i2c", check_i2c());
    CONSOLE.report(b"spi", check_spi());
    CONSOLE.report(b"adc", check_adc());
    CONSOLE.report(b"wdt", check_wdt());
    CONSOLE.report(b"rtc", check_rtc());
    CONSOLE.puts(b"TIER1 done\n");

    loop {
        spin(1_000_000);
    }
}
