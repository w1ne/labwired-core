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

    loop {}
}
