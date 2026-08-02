// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! STM32H735 Tier-1 fixture firmware (Arm Cortex-M7; built thumbv7em-none-eabi,
//! soft-float — the integer self-tests need no FPU bring-up).
//!
//! Validates the simulator's chip model peripheral-by-peripheral with RAW
//! REGISTER accesses against the peripherals wired by
//! `configs/chips/stm32h735.yaml`, reporting one line per peripheral class over
//! USART3 using the TIER1 protocol:
//!
//! ```text
//! TIER1 <class> PASS
//! TIER1 <class> FAIL code=<reason>
//! TIER1 done
//! ```
//!
//! The `uart` class is implicit: receiving `TIER1 done` over the UART is itself
//! the proof of a working UART path, so no `uart` line is printed.
//!
//! This is the FIRST Cortex-M7 fixture in LabWired and it is SIM-DERIVED — there
//! is no H735 bench part, so the register offsets/reset values below follow
//! RM0468 (STM32H723/733/725/735/730) and the simulator's H7 models rather than
//! a silicon capture. H7-family specifics vs the H5 fixture: RCC @ 0x5802_4400
//! with HSIRDY at bit 2 (not 1), the enable block at APB1LENR@0xE8 /
//! APB2ENR@0xF0, and LSI in RCC_CSR@0x74; GPIO @ 0x5802_0000.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::{entry, exception};
use panic_halt as _;

// ── Wired peripherals (configs/chips/stm32h735.yaml) ──────────────────────
const RCC_BASE: u32 = 0x5802_4400; // type rcc, profile h7 (RM0468 §8.7)
const GPIOA_BASE: u32 = 0x5802_0000; // type gpio, profile stm32v2
const USART3_BASE: u32 = 0x4000_4800; // type uart, profile stm32v2 (console)
const TIM2_BASE: u32 = 0x4000_0000; // type timer, 32-bit (APB1)
const TIM1_BASE: u32 = 0x4001_0000; // type timer, advanced (id tim1_pwm, APB2)
const I2C1_BASE: u32 = 0x4000_5400; // type i2c, profile h5 (APB1)
const SPI1_BASE: u32 = 0x4001_3000; // type spi, profile stm32h5 (APB2)
const IWDG_BASE: u32 = 0x5800_4800; // type iwdg (LSI-clocked)

// RCC enable/clock registers (H7 offsets).
const RCC_CR: u32 = RCC_BASE; // 0x00
const RCC_CFGR: u32 = RCC_BASE + 0x10; // SW[2:0] → SWS[5:3]
const RCC_CSR: u32 = RCC_BASE + 0x74; // LSION bit0 → LSIRDY bit1
const RCC_APB1LENR: u32 = RCC_BASE + 0xE8;
const RCC_APB2ENR: u32 = RCC_BASE + 0xF0;

// NVIC (installed for every Cortex-M chip; declared in the yaml as `nvic`).
const NVIC_ISER0: u32 = 0xE000_E100;
const NVIC_ICER0: u32 = 0xE000_E180;
const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ICPR0: u32 = 0xE000_E280;

/// IRQ used for the NVIC delivery round-trip: 28 = TIM2 — wired in the yaml but
/// never configured to raise its own interrupt (DIER stays 0), so a software
/// pend is unambiguous.
const TEST_IRQ: i16 = 28;

// USART3, stm32v2 layout: ISR @ 0x1C (TXE = bit 7), TDR @ 0x28.
// Read the full ISR word and bit-test TXE (a sign-bit test on a byte load
// compiles to an LDRSB reg-offset the 16-bit Thumb decoder does not implement).
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

/// clock: H7 RCC (RM0468 §8.7). HSI is on+ready out of reset (HSIRDY = bit 2);
/// HSEON (bit 16) must latch HSERDY (bit 17); the SYSCLK switch (SW→SWS in
/// CFGR) is gated on the requested source's ready bit; APB1LENR @ 0xE8
/// round-trips the peripheral enables.
fn check_clock() -> Result<(), &'static [u8]> {
    if rd32(RCC_CR) & (1 << 2) == 0 {
        return Err(b"clock-hsirdy");
    }
    let cr = rd32(RCC_CR);
    wr32(RCC_CR, cr | (1 << 16)); // HSEON
    if rd32(RCC_CR) & (1 << 17) == 0 {
        return Err(b"clock-hserdy");
    }
    wr32(RCC_CR, cr); // drop HSE; HSERDY must clear
    if rd32(RCC_CR) & (1 << 17) != 0 {
        return Err(b"clock-hserdy-stuck");
    }
    // SW→SWS is gated on the source's ready bit: SW=001 (CSI) with CSI off must
    // NOT switch; after CSION+CSIRDY it must.
    wr32(RCC_CFGR, 0x1);
    if (rd32(RCC_CFGR) >> 3) & 0x7 != 0x0 {
        return Err(b"clock-sws-gate");
    }
    wr32(RCC_CR, cr | (1 << 7)); // CSION
    for _ in 0..10_000 {
        if rd32(RCC_CR) & (1 << 8) != 0 {
            break;
        }
    }
    if rd32(RCC_CR) & (1 << 8) == 0 {
        return Err(b"clock-csirdy");
    }
    wr32(RCC_CFGR, 0x1);
    if (rd32(RCC_CFGR) >> 3) & 0x7 != 0x1 {
        return Err(b"clock-sws");
    }
    // Back to HSI, CSI off.
    wr32(RCC_CFGR, 0x0);
    if (rd32(RCC_CFGR) >> 3) & 0x7 != 0x0 {
        return Err(b"clock-sws-back");
    }
    wr32(RCC_CR, cr);
    // APB1LENR round-trip (TIM2EN etc.), restored afterwards.
    let enr = rd32(RCC_APB1LENR);
    wr32(RCC_APB1LENR, enr | 0x1);
    if rd32(RCC_APB1LENR) & 0x1 != 0x1 {
        return Err(b"clock-enr");
    }
    wr32(RCC_APB1LENR, enr);
    Ok(())
}

/// gpio: stm32v2 port A. PA5 to output via MODER, set via BSRR, observe ODR,
/// clear via BRR. (The 0x5802_xxxx window is un-shadowed for this core —
/// bit-band alias translation is gated to M3/M4.)
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

/// timer: TIM2 (32-bit). Clock-gated on RCC_APB1LENR.TIM2EN (bit 0). PSC/ARR
/// write+readback, EGR.UG latches SR.UIF, write-0 clears it, CEN makes CNT
/// advance, CEN off freezes it. State restored.
fn check_timer() -> Result<(), &'static [u8]> {
    // TIM2 unclocked out of reset — ARR reads 0, not its 0xFFFFFFFF reset.
    if rd32(TIM2_BASE + 0x2C) != 0 {
        return Err(b"tim-gated");
    }
    wr32(RCC_APB1LENR, rd32(RCC_APB1LENR) | 0x1); // TIM2EN
    wr32(TIM2_BASE + 0x28, 7); // PSC
    if rd32(TIM2_BASE + 0x28) != 7 {
        return Err(b"tim-psc");
    }
    wr32(TIM2_BASE + 0x2C, 0x0001_0000); // ARR beyond 16 bits — counter is 32-bit
    if rd32(TIM2_BASE + 0x2C) != 0x0001_0000 {
        return Err(b"tim-arr32");
    }
    wr32(TIM2_BASE + 0x14, 0x1); // EGR.UG
    if rd32(TIM2_BASE + 0x10) & 0x1 == 0 {
        return Err(b"tim-uif");
    }
    wr32(TIM2_BASE + 0x10, 0); // clear UIF
    if rd32(TIM2_BASE + 0x10) & 0x1 != 0 {
        return Err(b"tim-uif-clear");
    }
    wr32(TIM2_BASE + 0x00, 0x1); // CR1.CEN
    spin(2_000);
    if rd32(TIM2_BASE + 0x24) == 0 {
        return Err(b"tim-cnt-stuck");
    }
    wr32(TIM2_BASE + 0x00, 0x0); // CEN off
    let frozen = rd32(TIM2_BASE + 0x24);
    spin(2_000);
    if rd32(TIM2_BASE + 0x24) != frozen {
        return Err(b"tim-cnt-runs");
    }
    wr32(TIM2_BASE + 0x24, 0);
    wr32(TIM2_BASE + 0x28, 0);
    wr32(TIM2_BASE + 0x2C, 0xFFFF_FFFF);
    Ok(())
}

/// pwm: TIM1 (advanced) PWM mode 1 on CH1. Clock-gated on RCC_APB2ENR.TIM1EN
/// (bit 0). A bare UG latches the compare-match flags for every channel whose
/// CCR equals the reloaded CNT; with CCR1=50, ARR=100 the running counter must
/// raise CC1IF only when it crosses the compare value.
fn check_pwm() -> Result<(), &'static [u8]> {
    if rd32(TIM1_BASE + 0x2C) != 0 {
        return Err(b"pwm-gated");
    }
    let apb2 = rd32(RCC_APB2ENR);
    wr32(RCC_APB2ENR, apb2 | (1 << 0)); // TIM1EN

    wr32(TIM1_BASE + 0x18, 0x0068); // CCMR1: OC1M=PWM1, OC1PE
    wr32(TIM1_BASE + 0x20, 0x0001); // CCER: CC1E
    wr32(TIM1_BASE + 0x28, 0); // PSC
    wr32(TIM1_BASE + 0x2C, 100); // ARR
    wr32(TIM1_BASE + 0x34, 50); // CCR1
    wr32(TIM1_BASE + 0x44, 0x8000); // BDTR.MOE
    wr32(TIM1_BASE + 0x14, 0x1); // EGR.UG
    if rd32(TIM1_BASE + 0x10) & 0x1 == 0 {
        return Err(b"pwm-ug-uif");
    }
    if rd32(TIM1_BASE + 0x10) & 0x2 != 0 {
        return Err(b"pwm-cc1-early"); // CCR1=50 != reloaded CNT=0: must NOT match
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
    wr32(RCC_APB2ENR, apb2);
    Ok(())
}

/// i2c: I2C1 (v2 IP). Clock-gated on RCC_APB1LENR.I2C1EN (bit 21). ISR resets
/// to TXE; OAR1/TIMINGR round-trip; CR1.PE set; a 1-byte AUTOEND master write
/// to an absent slave drives the address phase and NACKs (ISR.NACKF), then the
/// bus releases. State restored.
fn check_i2c() -> Result<(), &'static [u8]> {
    if rd32(I2C1_BASE + 0x18) != 0 {
        return Err(b"i2c-gated");
    }
    wr32(RCC_APB1LENR, rd32(RCC_APB1LENR) | (1 << 21)); // I2C1EN
    if rd32(I2C1_BASE + 0x18) & 0x1 == 0 {
        return Err(b"i2c-txe");
    }
    wr32(I2C1_BASE + 0x08, 0x8000_0052); // OAR1: OA1EN | addr 0x29<<1
    if rd32(I2C1_BASE + 0x08) & 0xFFFF != 0x0052 {
        return Err(b"i2c-oar1");
    }
    wr32(I2C1_BASE + 0x10, 0x0070_3031); // TIMINGR
    if rd32(I2C1_BASE + 0x10) != 0x0070_3031 {
        return Err(b"i2c-timingr");
    }
    wr32(I2C1_BASE + 0x00, 0x1); // CR1.PE
    if rd32(I2C1_BASE + 0x00) & 0x1 == 0 {
        return Err(b"i2c-pe");
    }
    // CR2 = SADD(0x52<<1) | NBYTES(1)<<16 | AUTOEND<<25 | START<<13.
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
    wr32(I2C1_BASE + 0x00, 0x0);
    wr32(I2C1_BASE + 0x08, 0x0);
    wr32(I2C1_BASE + 0x10, 0x0);
    Ok(())
}

/// spi: SPI1 (stm32h5 "SPI v2" IP). Clock-gated on RCC_APB2ENR.SPI1EN (bit 12).
/// Reset pins (CFG1=0x00070007, SR=TXP|TXC), SSI-before-MASTER bring-up, the
/// SPE config lock, CTSIZE mirroring TSIZE in SR, and a 2-frame TX run to EOT.
fn check_spi() -> Result<(), &'static [u8]> {
    if rd32(SPI1_BASE + 0x08) != 0 {
        return Err(b"spi-gated");
    }
    let apb2 = rd32(RCC_APB2ENR);
    wr32(RCC_APB2ENR, apb2 | (1 << 12)); // SPI1EN

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
    // Config registers lock while SPE=1.
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
    wr32(RCC_APB2ENR, apb2);
    Ok(())
}

/// wdt: IWDG. No RCC peripheral-enable gate — it runs off the dedicated LSI
/// oscillator (RCC_CSR.LSION bit0 → LSIRDY bit1 on H7). RLR resets to 0xFFF;
/// KR=0x5555 unlocks PR/RLR writes. The watchdog is never started (KR=0xCCCC
/// is not written).
fn check_wdt() -> Result<(), &'static [u8]> {
    if rd32(IWDG_BASE + 0x08) != 0xFFF {
        return Err(b"wdt-rlr-reset");
    }
    // LSI on (RCC_CSR bit0 → LSIRDY bit1).
    let csr = rd32(RCC_CSR);
    wr32(RCC_CSR, csr | 0x1);
    for _ in 0..10_000 {
        if rd32(RCC_CSR) & (1 << 1) != 0 {
            break;
        }
    }
    if rd32(RCC_CSR) & (1 << 1) == 0 {
        return Err(b"wdt-lsirdy");
    }
    wr32(IWDG_BASE + 0x00, 0x5555); // enable register access
    wr32(IWDG_BASE + 0x04, 0x2); // PR
    for _ in 0..100_000 {
        if rd32(IWDG_BASE + 0x0C) & 0x1 == 0 {
            break; // PVU cleared — prescaler committed
        }
    }
    if rd32(IWDG_BASE + 0x04) != 0x2 {
        return Err(b"wdt-pr");
    }
    wr32(IWDG_BASE + 0x08, 0xABC); // RLR
    for _ in 0..100_000 {
        if rd32(IWDG_BASE + 0x0C) & 0x2 == 0 {
            break; // RVU cleared — reload committed
        }
    }
    if rd32(IWDG_BASE + 0x08) != 0xABC {
        return Err(b"wdt-rlr");
    }
    wr32(IWDG_BASE + 0x08, 0xFFF);
    wr32(IWDG_BASE + 0x04, 0x0);
    wr32(IWDG_BASE + 0x00, 0x0);
    wr32(RCC_CSR, csr);
    Ok(())
}

static mut IRQ_HITS: u32 = 0;

/// irq: NVIC delivery round-trip. Enable TEST_IRQ in ISER0, software-pend it via
/// ISPR0, and require the vector to actually run (DefaultHandler counts it and
/// disarms itself).
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
    report(b"timer", check_timer());
    report(b"pwm", check_pwm());
    report(b"i2c", check_i2c());
    report(b"spi", check_spi());
    report(b"wdt", check_wdt());
    report(b"irq", check_irq());
    puts(b"TIER1 done\n");

    loop {
        spin(1_000_000);
    }
}
