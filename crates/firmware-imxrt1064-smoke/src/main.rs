// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// i.MX RT1064 Renode-onboarding smoke. Keeps the RT106x demo's LPUART6 banner
// and GPIO2 pin 3 toggle, then exercises GPT1 (0x401E_C000, NVIC line 100 —
// both from Renode's imxrt1064.repl and IMXRT1064RM §52):
//
// 1. CR.SWR resets the block and self-clears.
// 2. ipg_clk / (PR+1) = 1 MHz, restart mode, OCR1 = 49: CNT advances and stays
//    below OCR1 + 1, and the OF1 interrupt reaches the core through the NVIC
//    three times. The handler clears SR.OF1 by writing one.
// 3. Free-run mode with OF2IE and the NVIC line masked: the OF2 level reaches
//    ISPR, and clearing SR.OF2 drops the level so ICPR stays clear.
//
// Then LPUART6 CTRL.TCIE with the NVIC line masked must pend line 25 only
// (LPUART6_IRQn in MIMXRT1064.h and Renode's imxrt1064.repl) inside the
// LPUART1..8 window 20..27, and clearing TCIE releases it.
//
// Bare-register — no HAL. Soft-float thumbv7em-none-eabi.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, Ordering};
use cortex_m_rt::{entry, exception};
use panic_halt as _;

// CCM @ 0x400FC000 — CCGR0 (GPIO2 CG15), CCGR1 (GPT1 bus CG10, serial CG11),
// CCGR3 (LPUART6 CG3).
const CCM_CCGR0: u32 = 0x400F_C068;
const CCM_CCGR1: u32 = 0x400F_C06C;
const CCM_CCGR3: u32 = 0x400F_C074;

// LPUART6 @ 0x40198000 — STAT@+0x04, CTRL@+0x08, DATA@+0x0C.
const LPUART6_STAT: u32 = 0x4019_8004;
const LPUART6_CTRL: u32 = 0x4019_8008;
const LPUART6_DATA: u32 = 0x4019_800C;
const STAT_TDRE: u32 = 1 << 23;
const STAT_TC: u32 = 1 << 22;
const CTRL_TCIE: u32 = 1 << 22;
const CTRL_TE: u32 = 1 << 19;
const CTRL_RE: u32 = 1 << 18;

// GPIO2 @ 0x401BC000 — GDIR@+0x04, DR_TOGGLE@+0x8C.
const GPIO2_GDIR: u32 = 0x401B_C004;
const GPIO2_DR_TOGGLE: u32 = 0x401B_C08C;
const LED: u32 = 1 << 3;

// GPT1 @ 0x401EC000 (IMXRT1064RM §52.7).
const GPT1: u32 = 0x401E_C000;
const GPT_CR: u32 = GPT1;
const GPT_PR: u32 = GPT1 + 0x04;
const GPT_SR: u32 = GPT1 + 0x08;
const GPT_IR: u32 = GPT1 + 0x0C;
const GPT_OCR1: u32 = GPT1 + 0x10;
const GPT_OCR2: u32 = GPT1 + 0x14;
const GPT_CNT: u32 = GPT1 + 0x24;
const CR_EN: u32 = 1 << 0;
const CR_ENMOD: u32 = 1 << 1;
const CR_CLKSRC_IPG: u32 = 1 << 6;
const CR_FRR: u32 = 1 << 9;
const CR_SWR: u32 = 1 << 15;
const SR_OF1: u32 = 1 << 0;
const SR_OF2: u32 = 1 << 1;

// GPT1_IRQn = 100 → NVIC word 3, bit 4.
const GPT1_IRQN: i16 = 100;
const NVIC_ISER3: u32 = 0xE000_E10C;
const NVIC_ICER3: u32 = 0xE000_E18C;
const NVIC_ISPR3: u32 = 0xE000_E20C;
const NVIC_ICPR3: u32 = 0xE000_E28C;
const GPT1_NVIC_BIT: u32 = 1 << (GPT1_IRQN as u32 - 96);

// LPUART6_IRQn = 25 → NVIC word 0. LPUART1..8 occupy lines 20..27.
const NVIC_ICER0: u32 = 0xE000_E180;
const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ICPR0: u32 = 0xE000_E280;
const LPUART_LINES: u32 = 0xFF << 20;
const LPUART6_NVIC_BIT: u32 = 1 << 25;

#[cfg(feature = "onboarding-adc")]
const ADC1: u32 = 0x400C_4000;
#[cfg(feature = "onboarding-adc")]
const ADC1_HC0: u32 = ADC1;
#[cfg(feature = "onboarding-adc")]
const ADC1_HS: u32 = ADC1 + 0x20;
#[cfg(feature = "onboarding-adc")]
const ADC1_R0: u32 = ADC1 + 0x24;
#[cfg(feature = "onboarding-adc")]
const ADC1_CFG: u32 = ADC1 + 0x44;
#[cfg(feature = "onboarding-adc")]
const ADC1_GC: u32 = ADC1 + 0x48;
#[cfg(feature = "onboarding-adc")]
const ADC1_GS: u32 = ADC1 + 0x4C;
#[cfg(feature = "onboarding-adc")]
const ADC1_GC_CAL: u32 = 1 << 7;
#[cfg(feature = "onboarding-adc")]
const ADC1_GS_CALF: u32 = 1 << 1;
#[cfg(feature = "onboarding-adc")]
const ADC1_HS_COCO0: u32 = 1;

const OCR1_RESTART: u32 = 49;
const WANT_IRQS: u32 = 3;

static GPT1_IRQS: AtomicU32 = AtomicU32::new(0);
static OTHER_IRQS: AtomicU32 = AtomicU32::new(0);
static OF1_SEEN: AtomicU32 = AtomicU32::new(0);
static SR_AFTER_CLEAR: AtomicU32 = AtomicU32::new(u32::MAX);

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn putc(byte: u8) {
    while rd(LPUART6_STAT) & STAT_TDRE == 0 {}
    wr(LPUART6_DATA, byte as u32);
}

fn send(text: &[u8]) {
    for b in text {
        putc(*b);
    }
}

fn send_u32(mut value: u32) {
    let mut digits = [0u8; 10];
    let mut n = 0;
    loop {
        digits[n] = b'0' + (value % 10) as u8;
        n += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    while n > 0 {
        n -= 1;
        putc(digits[n]);
    }
}

#[cfg(feature = "onboarding-adc")]
fn send_hex(value: u32) {
    for shift in [12, 8, 4, 0] {
        let digit = ((value >> shift) & 0xF) as u8;
        putc(if digit < 10 {
            b'0' + digit
        } else {
            b'A' + digit - 10
        });
    }
}

fn flag(name: &[u8], ok: bool) {
    send(name);
    putc(b'=');
    putc(if ok { b'1' } else { b'0' });
    putc(b' ');
}

/// Poll `cond` up to `limit` times; true once it holds.
fn wait(limit: u32, mut cond: impl FnMut() -> bool) -> bool {
    for _ in 0..limit {
        if cond() {
            return true;
        }
    }
    cond()
}

#[exception]
unsafe fn DefaultHandler(irqn: i16) {
    if irqn == GPT1_IRQN {
        if rd(GPT_SR) & SR_OF1 != 0 {
            OF1_SEEN.fetch_add(1, Ordering::Relaxed);
        }
        wr(GPT_SR, SR_OF1);
        SR_AFTER_CLEAR.store(rd(GPT_SR) & SR_OF1, Ordering::Relaxed);
        GPT1_IRQS.fetch_add(1, Ordering::Relaxed);
    } else {
        OTHER_IRQS.fetch_add(1, Ordering::Relaxed);
    }
}

fn smoke_gpt1() -> bool {
    wr(CCM_CCGR1, rd(CCM_CCGR1) | (0b1111 << 20));

    // 1. Software reset self-clears and leaves the registers at reset values.
    wr(GPT_CR, CR_SWR);
    let swr = wait(1_000, || rd(GPT_CR) & CR_SWR == 0)
        && rd(GPT_IR) == 0
        && rd(GPT_SR) == 0
        && rd(GPT_OCR1) == u32::MAX;
    flag(b"swr", swr);

    // 2. Restart mode at 1 MHz (ipg_clk 150 MHz / 150) with the OF1 IRQ.
    wr(GPT_PR, 149);
    wr(GPT_OCR1, OCR1_RESTART);
    wr(GPT_CR, CR_CLKSRC_IPG | CR_ENMOD);
    wr(GPT_IR, SR_OF1);
    wr(NVIC_ICPR3, GPT1_NVIC_BIT);
    wr(NVIC_ISER3, GPT1_NVIC_BIT);
    wr(GPT_CR, CR_CLKSRC_IPG | CR_ENMOD | CR_EN);

    let first = rd(GPT_CNT);
    let counting = wait(20_000, || rd(GPT_CNT) != first);
    let mut in_range = true;
    for _ in 0..64 {
        in_range &= rd(GPT_CNT) <= OCR1_RESTART;
    }
    flag(b"cnt", counting && in_range);

    let fired = wait(200_000, || GPT1_IRQS.load(Ordering::Relaxed) >= WANT_IRQS);
    wr(NVIC_ICER3, GPT1_NVIC_BIT);
    wr(GPT_IR, 0);
    let irqs = GPT1_IRQS.load(Ordering::Relaxed);
    let irq_ok = fired
        && OF1_SEEN.load(Ordering::Relaxed) == irqs
        && SR_AFTER_CLEAR.load(Ordering::Relaxed) == 0
        && OTHER_IRQS.load(Ordering::Relaxed) == 0;
    send(b"irq=");
    send_u32(irqs);
    putc(b' ');
    flag(b"of1", irq_ok);

    // 3. Free-run OF2 level with the NVIC line masked: pending, then released.
    wr(GPT_CR, CR_CLKSRC_IPG | CR_ENMOD | CR_FRR);
    wr(GPT_SR, 0x3F);
    wr(GPT_OCR2, 20);
    wr(GPT_IR, SR_OF2);
    wr(NVIC_ICPR3, GPT1_NVIC_BIT);
    wr(GPT_CR, CR_CLKSRC_IPG | CR_ENMOD | CR_FRR | CR_EN);
    let pended = wait(100_000, || rd(NVIC_ISPR3) & GPT1_NVIC_BIT != 0) && rd(GPT_SR) & SR_OF2 != 0;
    wr(GPT_SR, SR_OF2);
    wr(NVIC_ICPR3, GPT1_NVIC_BIT);
    let released = rd(GPT_SR) & SR_OF2 == 0 && rd(NVIC_ISPR3) & GPT1_NVIC_BIT == 0;
    let of2 = pended && released && GPT1_IRQS.load(Ordering::Relaxed) == irqs;
    flag(b"of2", of2);

    wr(GPT_IR, 0);
    wr(GPT_CR, 0);
    wr(NVIC_ICPR3, GPT1_NVIC_BIT);

    swr && counting && in_range && irq_ok && of2
}

fn send_hex_byte(value: u32) {
    for shift in [4, 0] {
        let digit = ((value >> shift) & 0xF) as u8;
        putc(if digit < 10 {
            b'0' + digit
        } else {
            b'a' + digit - 10
        });
    }
}

fn smoke_lpuart6_irq() -> bool {
    // Drain the transmitter so STAT.TC is set, then mask and clear 20..27.
    let idle = wait(20_000, || rd(LPUART6_STAT) & STAT_TC != 0);
    wr(NVIC_ICER0, LPUART_LINES);
    wr(NVIC_ICPR0, LPUART_LINES);
    let ctrl = rd(LPUART6_CTRL);
    wr(LPUART6_CTRL, ctrl | CTRL_TCIE);
    wait(1_000, || rd(NVIC_ISPR0) & LPUART_LINES != 0);
    let pended = rd(NVIC_ISPR0) & LPUART_LINES;
    wr(LPUART6_CTRL, ctrl);
    wr(NVIC_ICPR0, LPUART_LINES);
    let released = rd(NVIC_ISPR0) & LPUART_LINES;
    let ok = idle && pended == LPUART6_NVIC_BIT && released == 0;

    // ISPR bits 20..27 as one byte: 0x20 is line 25.
    send(b"lpuart6 tcie ispr=");
    send_hex_byte(pended >> 20);
    send(b" rel=");
    send_hex_byte(released >> 20);
    send(if ok { b" ok\n" } else { b" FAIL\n" });
    ok
}

#[cfg(feature = "onboarding-adc")]
fn smoke_adc1() -> bool {
    // Select 12-bit results while retaining the reset clock selection.
    let cfg = rd(ADC1_CFG);
    wr(ADC1_CFG, (cfg & !(0b11 << 2)) | (0b10 << 2));

    // Clear CALF, request calibration, and wait for the model's timed finish.
    wr(ADC1_GS, ADC1_GS_CALF);
    wr(ADC1_GC, rd(ADC1_GC) | ADC1_GC_CAL);
    let started = rd(ADC1_GC) & ADC1_GC_CAL != 0;
    let ready = wait(20_000, || rd(ADC1_GC) & ADC1_GC_CAL == 0);
    let calibrated = rd(ADC1_GS) & ADC1_GS_CALF == 0;

    // A 25% position on the 3.3 V test fixture supplies 0.825 V; at 12 bits
    // the modeled channel result is 1024. HC0 writes start software mode.
    wr(ADC1_HC0, 0);
    let complete = wait(20_000, || rd(ADC1_HS) & ADC1_HS_COCO0 != 0);
    let coco = rd(ADC1_HS) & ADC1_HS_COCO0 != 0;
    let result = rd(ADC1_R0);
    let cleared = rd(ADC1_HS) & ADC1_HS_COCO0 == 0;
    let ok = started && ready && calibrated && complete && coco && result == 0x400 && cleared;

    send(b"adc cal=");
    putc(if calibrated { b'1' } else { b'0' });
    send(b" ready=");
    putc(if started && ready { b'1' } else { b'0' });
    send(b" coco=");
    putc(if complete && coco && cleared {
        b'1'
    } else {
        b'0'
    });
    send(b" result=");
    send_hex(result);
    send(if ok { b" ok\n" } else { b" FAIL\n" });
    ok
}

#[entry]
fn main() -> ! {
    // Ungate GPIO2 (CCGR0 CG15) and LPUART6 (CCGR3 CG3).
    wr(CCM_CCGR0, rd(CCM_CCGR0) | (0b11 << 30));
    wr(CCM_CCGR3, rd(CCM_CCGR3) | (0b11 << 6));
    wr(LPUART6_CTRL, CTRL_TE | CTRL_RE);
    wr(GPIO2_GDIR, rd(GPIO2_GDIR) | LED);

    send(b"OK\n");
    wr(GPIO2_DR_TOGGLE, LED);

    send(b"gpt1 ");
    let ok = smoke_gpt1();
    send(if ok { b"ok\n" } else { b"FAIL\n" });

    smoke_lpuart6_irq();

    #[cfg(feature = "onboarding-adc")]
    {
        let adc_ok = smoke_adc1();
        if !adc_ok {
            send(b"ADC1 smoke failed\n");
        }
    }

    loop {}
}
