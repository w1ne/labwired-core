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
//! Every poll is bounded by a fixed iteration count (the simulator is
//! deterministic — no wall-clock timeouts). Register offsets follow RM0481
//! and the simulator's models: rcc.rs (`h5` profile), gpio.rs (`stm32v2`),
//! uart.rs (`stm32v2`), gpdma.rs, spi.rs (`stm32h5`), adc.rs (`stm32l4`
//! layout), rtc_v3.rs and timer.rs. The dma/spi/adc/pwm/rtc/irq sequences
//! mirror the 2026-06-11 NUCLEO-H563ZI bench probes, so the same ELF is
//! silicon-replayable.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::{entry, exception};
use panic_halt as _;

// ── Wired peripherals (configs/chips/stm32h563.yaml) ──────────────────────
const RCC_BASE: u32 = 0x4402_0C00; // type rcc, profile h5
const GPIOA_BASE: u32 = 0x4202_0000; // type gpio, profile stm32v2
const USART3_BASE: u32 = 0x4000_4800; // type uart, profile stm32v2
const GPDMA1_BASE: u32 = 0x4002_0000; // type gpdma (8ch, RM0481)
const SPI1_BASE: u32 = 0x4001_3000; // type spi, profile stm32h5
const ADC1_BASE: u32 = 0x4202_8000; // type adc, stm32l4 layout
const RTC_BASE: u32 = 0x4400_7800; // type rtc_v3
const TIM1_BASE: u32 = 0x4001_2C00; // type timer, advanced (id tim1_pwm)
const FDCAN1_BASE: u32 = 0x4000_A400; // type fdcan (M_CAN, fixed RAM layout)
const SRAMCAN_BASE: u32 = 0x4000_AC00; // FDCAN message RAM (window +0x800)

// NVIC (installed for every Cortex-M chip; declared in the yaml as `nvic`).
const NVIC_ISER0: u32 = 0xE000_E100;
const NVIC_ICER0: u32 = 0xE000_E180;
const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ICPR0: u32 = 0xE000_E280;

/// IRQ used for the NVIC delivery round-trip: 27 = GPDMA1 channel 0 — wired
/// in the yaml but never configured to fire on its own here (the dma check
/// leaves IER-equivalents off), so a software pend is unambiguous.
const TEST_IRQ: i16 = 27;

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
/// clear via BRR. (The 0x4202_xxxx window is un-shadowed for this core —
/// bit-band alias translation is gated to M3/M4, silicon-verified by the
/// `h563_mmio_diff` oracle.)
fn check_gpio() -> Result<(), &'static [u8]> {
    let moder = rd32(GPIOA_BASE);
    wr32(GPIOA_BASE, (moder & !(0x3 << 10)) | (0x1 << 10)); // PA5 output
    wr32(GPIOA_BASE + 0x18, 1 << 5); // BSRR set
    if rd32(GPIOA_BASE + 0x14) & (1 << 5) == 0 {
        return Err(b"gpio-bsrr");
    }
    wr32(GPIOA_BASE + 0x28, 1 << 5); // BRR clear
    if rd32(GPIOA_BASE + 0x14) & (1 << 5) != 0 {
        return Err(b"gpio-brr");
    }
    wr32(GPIOA_BASE, moder);
    Ok(())
}

/// dma: GPDMA1 channel 0 software-request (mem-to-mem) block copy, byte
/// width, SINC+DINC. Mirrors the 2026-06-11 silicon probe: TCF (and HTF)
/// latch in C0SR, BNDT drains to 0, EN auto-clears, CFCR clears the flags,
/// and the destination must match byte-exact.
fn check_dma() -> Result<(), &'static [u8]> {
    const N: usize = 8;
    const C0FCR: u32 = GPDMA1_BASE + 0x5C;
    const C0SR: u32 = GPDMA1_BASE + 0x60;
    const C0CR: u32 = GPDMA1_BASE + 0x64;
    const C0TR1: u32 = GPDMA1_BASE + 0x90;
    const C0TR2: u32 = GPDMA1_BASE + 0x94;
    const C0BR1: u32 = GPDMA1_BASE + 0x98;
    const C0SAR: u32 = GPDMA1_BASE + 0x9C;
    const C0DAR: u32 = GPDMA1_BASE + 0xA0;

    let src: [u8; N] = [0xA5, 0x5A, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
    let mut dst: [u8; N] = [0; N];

    // GPDMA1 bus clock (AHB1ENR bit 0), restored at the end.
    let ahb1 = rd32(RCC_BASE + 0x88);
    wr32(RCC_BASE + 0x88, ahb1 | 0x1);

    if rd32(C0SR) & 0x1 == 0 {
        return Err(b"dma-idlef-reset");
    }
    wr32(C0FCR, 0xFFFF_FFFF); // clear any stale flags
    wr32(C0TR1, (1 << 3) | (1 << 19)); // SINC | DINC, byte width
    wr32(C0TR2, 1 << 9); // SWREQ: software request = mem-to-mem
    wr32(C0BR1, N as u32); // BNDT
    wr32(C0SAR, src.as_ptr() as u32);
    wr32(C0DAR, dst.as_mut_ptr() as u32);
    wr32(C0CR, 0x1); // EN

    let mut done = false;
    for _ in 0..20_000 {
        if rd32(C0SR) & (1 << 8) != 0 {
            // TCF
            done = true;
            break;
        }
    }
    if !done {
        wr32(C0CR, 0x2); // RESET the channel before bailing
        wr32(RCC_BASE + 0x88, ahb1);
        return Err(b"dma-tcf-timeout");
    }
    if rd32(C0BR1) & 0xFFFF != 0 {
        return Err(b"dma-bndt");
    }
    if rd32(C0CR) & 0x1 != 0 {
        return Err(b"dma-en-stuck"); // EN auto-clears at TC (silicon-pinned)
    }
    wr32(C0FCR, 0xFFFF_FFFF);
    if rd32(C0SR) != 0x1 {
        return Err(b"dma-fcr");
    }
    for i in 0..N {
        if unsafe { read_volatile(dst.as_ptr().add(i)) } != src[i] {
            return Err(b"dma-data-mismatch");
        }
    }
    wr32(RCC_BASE + 0x88, ahb1);
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

/// spi: SPI1 (stm32h5 IP). Reset pins (CFG1=0x00070007, SR=TXP|TXC), the
/// SSI-before-MASTER bring-up from the silicon probe, the SPE config lock,
/// CTSIZE mirroring TSIZE in SR, and a 2-frame TX run to EOT. On silicon
/// the same writes behave identically except frames only shift once
/// spi_ker_ck is configured (divergence documented in the chip yaml).
fn check_spi() -> Result<(), &'static [u8]> {
    let apb2 = rd32(RCC_BASE + 0xA4);
    wr32(RCC_BASE + 0xA4, apb2 | (1 << 12)); // SPI1EN

    if rd32(SPI1_BASE + 0x08) != 0x0007_0007 {
        return Err(b"spi-cfg1-reset");
    }
    if rd32(SPI1_BASE + 0x14) != 0x0000_1002 {
        return Err(b"spi-sr-reset");
    }
    wr32(SPI1_BASE, 1 << 12); // CR1.SSI first — internal SS high
    wr32(SPI1_BASE + 0x0C, (1 << 22) | (1 << 26)); // CFG2: MASTER|SSM
    if rd32(SPI1_BASE + 0x0C) != 0x0440_0000 {
        return Err(b"spi-master");
    }
    wr32(SPI1_BASE + 0x04, 2); // CR2.TSIZE = 2
    wr32(SPI1_BASE, (1 << 12) | 1); // SPE
    let sr = rd32(SPI1_BASE + 0x14);
    if (sr >> 16) & 0xFFFF != 2 || sr & (1 << 12) != 0 {
        return Err(b"spi-ctsize");
    }
    // Config registers lock while SPE=1 (silicon-pinned).
    wr32(SPI1_BASE + 0x0C, 0);
    if rd32(SPI1_BASE + 0x0C) != 0x0440_0000 {
        return Err(b"spi-spe-lock");
    }
    wr32(SPI1_BASE, (1 << 12) | (1 << 9) | 1); // CSTART
    wr32(SPI1_BASE + 0x20, 0xA5); // TXDR frame 1
    wr32(SPI1_BASE + 0x20, 0x5A); // TXDR frame 2
    let mut eot = false;
    for _ in 0..10_000 {
        if rd32(SPI1_BASE + 0x14) & (1 << 3) != 0 {
            eot = true;
            break;
        }
    }
    if !eot {
        return Err(b"spi-eot");
    }
    wr32(SPI1_BASE + 0x18, 0xFFFF_FFFF); // IFCR
    wr32(SPI1_BASE, 0); // SPE off
    wr32(SPI1_BASE + 0x0C, 0);
    wr32(SPI1_BASE + 0x04, 0);
    wr32(RCC_BASE + 0xA4, apb2);
    Ok(())
}

/// adc: ADC1 power-up handshake, silicon-pinned 2026-06-11: DEEPPWD out of
/// reset, DEEPPWD clear -> ADVREGEN -> ADEN raises ISR.ADRDY.
fn check_adc() -> Result<(), &'static [u8]> {
    let ahb2 = rd32(RCC_BASE + 0x8C);
    wr32(RCC_BASE + 0x8C, ahb2 | (1 << 10)); // ADCEN

    if rd32(ADC1_BASE + 0x08) != 0x2000_0000 {
        return Err(b"adc-deeppwd-reset");
    }
    wr32(ADC1_BASE + 0x08, 0); // exit deep power-down
    wr32(ADC1_BASE + 0x08, 1 << 28); // ADVREGEN
    wr32(ADC1_BASE + 0x08, (1 << 28) | 1); // ADEN
    let mut ready = false;
    for _ in 0..10_000 {
        if rd32(ADC1_BASE) & 0x1 != 0 {
            ready = true;
            break;
        }
    }
    if !ready {
        return Err(b"adc-adrdy");
    }
    wr32(ADC1_BASE, 0x1); // rc_w1: clear ADRDY
    wr32(ADC1_BASE + 0x08, 0x2000_0000); // back to deep power-down
    wr32(RCC_BASE + 0x8C, ahb2);
    Ok(())
}

/// pwm: TIM1 (advanced) PWM mode 1 on CH1. A bare UG latches the
/// compare-match flags for every output channel whose CCR equals the
/// reloaded CNT — including the internal channels 5/6 at SR bits 16/17
/// (silicon-pinned 2026-06-11). With CCR1=50 and ARR=100 the running
/// counter must raise CC1IF when it crosses the compare value.
fn check_pwm() -> Result<(), &'static [u8]> {
    let apb2 = rd32(RCC_BASE + 0xA4);
    wr32(RCC_BASE + 0xA4, apb2 | (1 << 11)); // TIM1EN

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
    wr32(TIM1_BASE + 0x10, 0);
    wr32(TIM1_BASE + 0x18, 0);
    wr32(TIM1_BASE + 0x20, 0);
    wr32(TIM1_BASE + 0x2C, 0xFFFF);
    wr32(TIM1_BASE + 0x34, 0);
    wr32(TIM1_BASE + 0x44, 0);
    wr32(TIM1_BASE + 0x24, 0);
    wr32(RCC_BASE + 0xA4, apb2);
    Ok(())
}

/// rtc: RTC v3 calendar bring-up, mirroring the 2026-06-11 silicon probe:
/// RTCAPB clock + LSI + BDCR.RTCEN, WPR unlock, BYPSHAD, init mode
/// (INIT->INITF), TR/DR/PRER writes, exit (INITS rises), and the WPR
/// relock dropping further calendar writes.
fn check_rtc() -> Result<(), &'static [u8]> {
    let apb3 = rd32(RCC_BASE + 0xA8);
    wr32(RCC_BASE + 0xA8, apb3 | (1 << 21)); // RTCAPBEN
    let bdcr = rd32(RCC_BASE + 0xF0);
    wr32(RCC_BASE + 0xF0, bdcr | (1 << 26)); // LSION
    for _ in 0..10_000 {
        if rd32(RCC_BASE + 0xF0) & (1 << 27) != 0 {
            break;
        }
    }
    if rd32(RCC_BASE + 0xF0) & (1 << 27) == 0 {
        return Err(b"rtc-lsirdy");
    }
    wr32(RCC_BASE + 0xF0, rd32(RCC_BASE + 0xF0) | (0x2 << 8) | (1 << 15)); // RTCSEL=LSI, RTCEN

    wr32(RTC_BASE + 0x24, 0xCA); // WPR key 1
    wr32(RTC_BASE + 0x24, 0x53); // WPR key 2
    wr32(RTC_BASE + 0x18, 1 << 5); // CR.BYPSHAD
    wr32(RTC_BASE + 0x0C, 1 << 7); // ICSR.INIT
    let mut initf = false;
    for _ in 0..10_000 {
        if rd32(RTC_BASE + 0x0C) & (1 << 6) != 0 {
            initf = true;
            break;
        }
    }
    if !initf {
        return Err(b"rtc-initf");
    }
    wr32(RTC_BASE + 0x10, 0x007F_00FF); // PRER
    if rd32(RTC_BASE + 0x10) != 0x007F_00FF {
        return Err(b"rtc-prer");
    }
    wr32(RTC_BASE, 0x0012_3456); // TR 12:34:56 BCD
    wr32(RTC_BASE + 0x04, 0x0026_0611); // DR 2026-06-11
    if rd32(RTC_BASE) != 0x0012_3456 {
        return Err(b"rtc-tr");
    }
    wr32(RTC_BASE + 0x0C, 0); // exit init
    for _ in 0..10_000 {
        if rd32(RTC_BASE + 0x0C) & (1 << 6) == 0 {
            break;
        }
    }
    if rd32(RTC_BASE + 0x0C) & (1 << 4) == 0 {
        return Err(b"rtc-inits"); // year != 0 => INITS
    }
    // Seconds field still in the written minute (calendar may have ticked).
    if rd32(RTC_BASE) & 0x00FF_FF00 != 0x0012_3400 {
        return Err(b"rtc-run");
    }
    wr32(RTC_BASE + 0x24, 0xFF); // relock
    wr32(RTC_BASE, 0); // locked write must be dropped (silicon-pinned)
    if rd32(RTC_BASE) & 0x00FF_FF00 != 0x0012_3400 {
        return Err(b"rtc-wpr");
    }
    Ok(())
}

static mut IRQ_HITS: u32 = 0;

/// can: FDCAN1 internal loopback (CCCR.TEST + MON, TEST.LBCK) — TX buffer 0
/// to RX FIFO0 through the fixed SRAMCAN layout. Replays silicon capture13
/// (2026-06-12, NUCLEO-H563ZI) register-for-register, so the same ELF is
/// bench-replayable; no transceiver needed.
fn check_can() -> Result<(), &'static [u8]> {
    // Bus clock, then kernel clock: FDCANSEL resets to 00 = HSE, and the
    // Nucleo's HSE is the ST-LINK 8 MHz MCO — digital bypass
    // (HSEON | HSEBYP | HSEEXT).
    wr32(RCC_BASE + 0xA0, rd32(RCC_BASE + 0xA0) | (1 << 9));
    wr32(RCC_BASE, rd32(RCC_BASE) | 0x0015_0000);
    for _ in 0..50_000 {
        if rd32(RCC_BASE) & (1 << 17) != 0 {
            break;
        }
    }
    if rd32(FDCAN1_BASE + 0x04) != 0x8765_4321 {
        return Err(b"can-endn");
    }
    // INIT | CCE unlocks config; TEST + MON arm internal loopback.
    wr32(FDCAN1_BASE + 0x18, 0x3);
    if rd32(FDCAN1_BASE + 0x18) & 0x3 != 0x3 {
        return Err(b"can-cce");
    }
    wr32(FDCAN1_BASE + 0x18, 0xA3);
    wr32(FDCAN1_BASE + 0x10, 1 << 4); // TEST.LBCK
    if rd32(FDCAN1_BASE + 0x10) & (1 << 4) == 0 {
        return Err(b"can-lbck");
    }
    // TX buffer 0 (SRAMCAN + 0x278): std ID 0x123, DLC 8.
    wr32(SRAMCAN_BASE + 0x278, 0x123 << 18);
    wr32(SRAMCAN_BASE + 0x27C, 8 << 16);
    wr32(SRAMCAN_BASE + 0x280, 0xDEAD_BEEF);
    wr32(SRAMCAN_BASE + 0x284, 0xCAFE_BABE);
    // Blank the RX element so stale RAM can't fake the compare below.
    wr32(SRAMCAN_BASE + 0xB0, 0);
    wr32(SRAMCAN_BASE + 0xB4, 0);
    wr32(SRAMCAN_BASE + 0xB8, 0);
    wr32(SRAMCAN_BASE + 0xBC, 0);
    // Leave INIT (CCE drops with it — silicon: write 0xA2, read 0xA0).
    wr32(FDCAN1_BASE + 0x18, 0xA2);
    for _ in 0..50_000 {
        if rd32(FDCAN1_BASE + 0x18) & 0x1 == 0 {
            break;
        }
    }
    if rd32(FDCAN1_BASE + 0x18) & 0x1 != 0 {
        return Err(b"can-init-stuck");
    }
    wr32(FDCAN1_BASE + 0xCC, 0x1); // TXBAR: fire buffer 0
    let mut received = false;
    for _ in 0..50_000 {
        if rd32(FDCAN1_BASE + 0x90) & 0x7F != 0 {
            received = true;
            break;
        }
    }
    if !received {
        return Err(b"can-rx-timeout");
    }
    if rd32(FDCAN1_BASE + 0xD4) & 0x1 == 0 {
        return Err(b"can-txbto");
    }
    if rd32(FDCAN1_BASE + 0x50) & 0x1 == 0 {
        return Err(b"can-ir-rf0n");
    }
    // RX element 0 at SRAMCAN + 0xB0. Silicon leaves R0[17:0] undefined
    // for standard IDs — compare the masked field only.
    if (rd32(SRAMCAN_BASE + 0xB0) >> 18) & 0x7FF != 0x123 {
        return Err(b"can-rx-id");
    }
    if (rd32(SRAMCAN_BASE + 0xB4) >> 16) & 0xF != 8 {
        return Err(b"can-rx-dlc");
    }
    if rd32(SRAMCAN_BASE + 0xB8) != 0xDEAD_BEEF || rd32(SRAMCAN_BASE + 0xBC) != 0xCAFE_BABE {
        return Err(b"can-rx-data");
    }
    // IR is rc_w1; acking RXF0 element 0 must drop the fill level.
    let ir = rd32(FDCAN1_BASE + 0x50);
    wr32(FDCAN1_BASE + 0x50, ir);
    if rd32(FDCAN1_BASE + 0x50) != 0 {
        return Err(b"can-ir-w1c");
    }
    wr32(FDCAN1_BASE + 0x94, 0);
    if rd32(FDCAN1_BASE + 0x90) & 0x7F != 0 {
        return Err(b"can-ack");
    }
    Ok(())
}

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
        unsafe {
            let p = core::ptr::addr_of_mut!(IRQ_HITS);
            write_volatile(p, read_volatile(p) + 1);
        }
    }
}

#[entry]
fn main() -> ! {
    report(b"clock", check_clock());
    report(b"gpio", check_gpio());
    report(b"dma", check_dma());
    report(b"timer", check_timer());
    report(b"i2c", check_i2c());
    report(b"wdt", check_wdt());
    report(b"spi", check_spi());
    report(b"adc", check_adc());
    report(b"pwm", check_pwm());
    report(b"rtc", check_rtc());
    report(b"can", check_can());
    report(b"irq", check_irq());
    puts(b"TIER1 done\n");

    loop {
        spin(1_000_000);
    }
}
