// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32G474RE Tier-1 fixture firmware (Cortex-M4F, thumbv7em-none-eabi).
//!
//! Validates the simulator's chip model peripheral-by-peripheral with RAW
//! REGISTER accesses against the peripherals wired by
//! `configs/chips/stm32g474re.yaml`, reporting one line per peripheral class
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
//! Each gated peripheral (timer/pwm/i2c/spi/adc/rtc) is FIRST poked while its
//! RCC clock-enable bit is off and required to read dead/0 — proving the
//! stm32v2 clock-gate is modelled — then its bit is enabled before the
//! behavioural round-trip. dma/wdt are ungated (AHB1ENR is not surfaced by the
//! V2 RCC model / IWDG has no enable gate on silicon), so those are pure
//! behavioural checks.
//!
//! Every poll is bounded by a fixed iteration count (the simulator is
//! deterministic — no wall-clock timeouts). Register offsets follow RM0440 and
//! the simulator's models: rcc.rs (`stm32v2`), gpio.rs (`stm32v2`), uart.rs
//! (`stm32v2`), timer.rs, i2c.rs (`stm32l4`), spi.rs (`stm32_fifo`), adc.rs
//! (`stm32l4`), dma.rs (Dma1), rtc.rs, iwdg.rs and nvic.rs.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::{entry, exception};
use panic_halt as _;

// ── Wired peripherals (configs/chips/stm32g474re.yaml) ────────────────────
const RCC_BASE: u32 = 0x4002_1000; // type rcc, profile stm32v2
const GPIOA_BASE: u32 = 0x4800_0000; // type gpio, profile stm32v2
const USART2_BASE: u32 = 0x4000_4400; // type uart, profile stm32v2 (console)
const TIM2_BASE: u32 = 0x4000_0000; // type timer, width 32
const TIM1_BASE: u32 = 0x4001_2C00; // type timer, advanced (id tim1_pwm)
const I2C1_BASE: u32 = 0x4000_5400; // type i2c, profile stm32l4
const SPI1_BASE: u32 = 0x4001_3000; // type spi, profile stm32_fifo
const ADC1_BASE: u32 = 0x5000_0000; // type adc, profile stm32l4
const DMA1_BASE: u32 = 0x4002_0000; // type dma (Dma1, 7ch)
const IWDG_BASE: u32 = 0x4000_3000; // type iwdg
const RTC_BASE: u32 = 0x4000_2800; // type rtc (stm32l4 layout)

// V2 RCC clock-enable registers (offsets the stm32v2 RCC model exposes).
const RCC_AHB2ENR: u32 = RCC_BASE + 0x8C;
const RCC_APB1ENR: u32 = RCC_BASE + 0x9C; // APB1LENR
const RCC_APB2ENR: u32 = RCC_BASE + 0xA4;

// NVIC (installed for every Cortex-M chip; declared in the yaml as `nvic`).
const NVIC_ISER0: u32 = 0xE000_E100;
const NVIC_ICER0: u32 = 0xE000_E180;
const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ICPR0: u32 = 0xE000_E280;
/// Software-pended test IRQ. Must be < 32 (cortex-m-rt default vector table)
/// and unused by any wired peripheral (G474 uses 2, 11, 18, 24, 28, 31 in
/// that range — 20 is free).
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

/// clock: V2 (H5-style) RCC. HSI is on+ready out of reset; HSEON (bit 16)
/// must latch HSERDY (bit 17); SW→SWS mirrors in CFGR @ 0x08; AHB2ENR @ 0x8C
/// round-trips GPIO port enables.
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
    // CFGR (0x08) SW=01 → SWS must mirror.
    wr32(RCC_BASE + 0x08, 0x1);
    if (rd32(RCC_BASE + 0x08) >> 2) & 0x3 != 0x1 {
        return Err(b"clock-sws");
    }
    // AHB2ENR round-trip: GPIOA/B/C/D enables.
    wr32(RCC_AHB2ENR, 0xF);
    if rd32(RCC_AHB2ENR) != 0xF {
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

/// timer: TIM2 (32-bit), clock-gated on RCC_APB1ENR1.TIM2EN (bit 0). Gated it
/// reads dead; enabled, EGR.UG latches UIF, SR write-0 clears, and CEN makes
/// the 32-bit counter advance.
fn check_timer() -> Result<(), &'static [u8]> {
    // Dead while gated: writes are dropped and reads return 0.
    wr32(TIM2_BASE + 0x2C, 0x1234);
    if rd32(TIM2_BASE + 0x2C) != 0 {
        return Err(b"tim-gated");
    }
    wr32(RCC_APB1ENR, rd32(RCC_APB1ENR) | (1 << 0)); // TIM2EN
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

/// pwm: TIM1 (advanced), clock-gated on RCC_APB2ENR.TIM1EN (bit 11). A bare UG
/// latches the compare-match flags for every channel whose CCR equals the
/// reloaded CNT — including the internal channels 5/6 at SR bits 16/17. With
/// CCR1=50 and ARR=100 the running counter must raise CC1IF when it crosses 50.
fn check_pwm() -> Result<(), &'static [u8]> {
    wr32(TIM1_BASE + 0x2C, 0x100);
    if rd32(TIM1_BASE + 0x2C) != 0 {
        return Err(b"pwm-gated");
    }
    wr32(RCC_APB2ENR, rd32(RCC_APB2ENR) | (1 << 11)); // TIM1EN
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
/// elements with MINC+PINC. TCIF1 must latch and the destination must match.
/// Ungated: RM0440 puts DMA1EN on AHB1ENR, which the V2 RCC model does not
/// surface, so the channel is left ungated and the round-trip carries the proof.
fn check_dma() -> Result<(), &'static [u8]> {
    const N: usize = 8;
    let src: [u8; N] = [0xA5, 0x5A, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
    let mut dst: [u8; N] = [0; N];

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

/// i2c: I2C1 (modern v2), clock-gated on RCC_APB1ENR1.I2C1EN (bit 21). Gated
/// ISR reads 0; enabled, ISR resets with TXE; PE round-trips; CR2.START latches
/// ISR.BUSY; CR2.STOP clears it.
fn check_i2c() -> Result<(), &'static [u8]> {
    if rd32(I2C1_BASE + 0x18) != 0 {
        return Err(b"i2c-gated");
    }
    wr32(RCC_APB1ENR, rd32(RCC_APB1ENR) | (1 << 21)); // I2C1EN
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

/// spi: SPI1 (stm32_fifo), clock-gated on RCC_APB2ENR.SPI1EN (bit 12). Gated
/// SR reads 0; enabled, SR.TXE (bit1) asserts; a byte DR write sets SR.BSY
/// (bit7), then the cycle-counted engine clears BSY and re-asserts TXE.
fn check_spi() -> Result<(), &'static [u8]> {
    if rd32(SPI1_BASE + 0x08) != 0 {
        return Err(b"spi-gated");
    }
    wr32(RCC_APB2ENR, rd32(RCC_APB2ENR) | (1 << 12)); // SPI1EN
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
    wr32(SPI1_BASE + 0x00, 0); // disable
    Ok(())
}

/// Run one ADC1 single conversion at CFGR.RES = `res`, returning DR. The model
/// converts a fixed internal source (V(IN)=3.0 V, V(REF+)=3.3 V): the 12-bit
/// code is (3.0/3.3)*4096=3723, narrower resolutions drop LSBs.
fn adc_convert(res: u32) -> Result<u32, &'static [u8]> {
    let cfgr = rd32(ADC1_BASE + 0x0C) & !(0x3 << 3);
    wr32(ADC1_BASE + 0x0C, cfgr | (res << 3));
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

/// adc: ADC1 (stm32l4), clock-gated on RCC_AHB2ENR.ADC12EN (bit 13). Gated the
/// CR reads 0. After ungating + power-up (clear DEEPPWD, ADVREGEN, ADEN ->
/// ISR.ADRDY; ADRDY must NOT assert before ADEN), prove a REAL conversion BY
/// VALUE: ADSTART converts the fixed internal source and the code must scale
/// when CFGR.RES narrows. Fails if the model returned a constant.
fn check_adc() -> Result<(), &'static [u8]> {
    if rd32(ADC1_BASE + 0x08) != 0 {
        return Err(b"adc-gated");
    }
    wr32(RCC_AHB2ENR, rd32(RCC_AHB2ENR) | (1 << 13)); // ADC12EN
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
/// RCC_APB1ENR1.RTCAPBEN (bit 10). Gated DR reads 0; enabled, DR resets to
/// 0x2101; WPR half-unlocks on 0xCA then unlocks on 0x53, and TR round-trips.
fn check_rtc() -> Result<(), &'static [u8]> {
    if rd32(RTC_BASE + 0x04) != 0 {
        return Err(b"rtc-gated");
    }
    wr32(RCC_APB1ENR, rd32(RCC_APB1ENR) | (1 << 10)); // RTCAPBEN
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
    report(b"pwm", check_pwm());
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
