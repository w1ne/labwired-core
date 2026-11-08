// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SAM4S Xplained smoke for the imported Renode platform (`sam4s_xplained`).
//!
//! The demo bin (`src/main.rs`) prints a banner and the LED/button state; this
//! one checks what Renode's `sam4s.repl` models and the chip descriptor
//! claims: PMC peripheral clocks (PCER0/PCDR0/PCSR0), the PC10 LED toggling
//! through PIO_SODR/CODR, and both `SAM_TC` blocks:
//!
//! * TC0 — UP_RC waveform, MCK/8, RC compare pends NVIC line 23 (IMR.CPCS);
//!   reading TC_SR clears CPCS and lets the pend be cleared.
//! * TC1 — UP waveform at MCK/2 runs past 0xFFFF and latches COVFS.
//! * TC2 — CPCSTOP: the counter stops at RC, the clock stays enabled.
//! * TC3 — CPCDIS: the RC compare clears SR.CLKSTA.
//! * TC4/TC5 — CLKEN alone does not count; BCR.SYNC on the second block
//!   starts both. TC5 runs in capture mode with CPCTRG, where RA ignores
//!   stores, and stops counting once PMC_PCDR0 gates its clock off.
//!
//! Every check sets a bit in a summary word at 0x2001FF00; 0x5A45007F means
//! all seven passed.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const PMC_PCER0: *mut u32 = 0x400E_0410 as *mut u32;
const PMC_PCDR0: *mut u32 = 0x400E_0414 as *mut u32;
const PMC_PCSR0: *const u32 = 0x400E_0418 as *const u32;
const PMC_SR: *const u32 = 0x400E_0468 as *const u32;
const PMC_SR_MCKRDY: u32 = 1 << 3;

const PIOC_PER: *mut u32 = 0x400E_1200 as *mut u32;
const PIOC_OER: *mut u32 = 0x400E_1210 as *mut u32;
const PIOC_SODR: *mut u32 = 0x400E_1230 as *mut u32;
const PIOC_CODR: *mut u32 = 0x400E_1234 as *mut u32;
const PIOC_ODSR: *const u32 = 0x400E_1238 as *const u32;
const LED_PIN: u32 = 1 << 10;

const UART0_CR: *mut u32 = 0x400E_0600 as *mut u32;
const UART0_MR: *mut u32 = 0x400E_0604 as *mut u32;
const UART0_SR: *const u32 = 0x400E_0614 as *const u32;
const UART0_THR: *mut u32 = 0x400E_061C as *mut u32;
const UART0_BRGR: *mut u32 = 0x400E_0620 as *mut u32;

const NVIC_ISPR0: *const u32 = 0xE000_E200 as *const u32;
const NVIC_ICPR0: *mut u32 = 0xE000_E280 as *mut u32;

/// Peripheral ids (= NVIC lines = PMC_PCSR0 bits).
const ID_UART0: u32 = 8;
const ID_PIOC: u32 = 13;
const ID_TC0: u32 = 23;

const SUMMARY: *mut u32 = 0x2001_FF00 as *mut u32;

// TC channel n of the device: block n / 3 at 0x40010000 + 0x4000 * block,
// channel registers every 0x40.
const TC_CCR: u32 = 0x00;
const TC_CMR: u32 = 0x04;
const TC_CV: u32 = 0x10;
const TC_RA: u32 = 0x14;
const TC_RC: u32 = 0x1C;
const TC_SR: u32 = 0x20;
const TC_IER: u32 = 0x24;
const TC_IDR: u32 = 0x28;
const TC_IMR: u32 = 0x2C;
const TC1_BCR: *mut u32 = 0x4001_40C0 as *mut u32;

const CCR_CLKEN: u32 = 1;
const CCR_CLKDIS: u32 = 1 << 1;
const CCR_SWTRG: u32 = 1 << 2;
const SR_COVFS: u32 = 1;
const SR_CPCS: u32 = 1 << 4;
const SR_CLKSTA: u32 = 1 << 16;
const CMR_WAVE: u32 = 1 << 15;
const CMR_UP_RC: u32 = 2 << 13;
const CMR_CPCSTOP: u32 = 1 << 6;
const CMR_CPCDIS: u32 = 1 << 7;
const CMR_CPCTRG: u32 = 1 << 14;
const MCK_2: u32 = 0;
const MCK_8: u32 = 1;
const MCK_128: u32 = 3;

fn tc(n: u32, reg: u32) -> *mut u32 {
    (0x4001_0000 + 0x4000 * (n / 3) + 0x40 * (n % 3) + reg) as *mut u32
}

fn rd(p: *mut u32) -> u32 {
    unsafe { read_volatile(p) }
}

fn wr(p: *mut u32, v: u32) {
    unsafe { write_volatile(p, v) }
}

fn putc(byte: u8) {
    unsafe {
        while read_volatile(UART0_SR) & 2 == 0 {}
        write_volatile(UART0_THR, byte as u32);
    }
}

fn puts(text: &str) {
    text.bytes().for_each(putc);
}

fn put_hex(value: u32, digits: u32) {
    for shift in (0..digits).rev() {
        let nibble = ((value >> (shift * 4)) & 0xF) as u8;
        putc(if nibble < 10 {
            b'0' + nibble
        } else {
            b'a' + nibble - 10
        });
    }
}

fn put_flag(label: &str, value: bool) {
    puts(label);
    putc(if value { b'1' } else { b'0' });
}

#[inline(never)]
fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}

/// Poll TC_SR until any of `mask` shows up (bounded). TC_SR clears its flags
/// on every read, so the returned word is the only record of them.
fn wait_sr(n: u32, mask: u32) -> u32 {
    for _ in 0..200_000 {
        let sr = rd(tc(n, TC_SR));
        if sr & mask != 0 {
            return sr;
        }
    }
    0
}

fn finish(label: &str, ok: bool, bit: u32, summary: &mut u32) {
    puts(label);
    puts(if ok { " ok\n" } else { " FAIL\n" });
    if ok {
        *summary |= 1 << bit;
    }
}

#[entry]
fn main() -> ! {
    let mut summary = 0u32;
    let tc_ids = 0x3F << ID_TC0;
    unsafe {
        write_volatile(PMC_PCER0, (1 << ID_UART0) | (1 << ID_PIOC) | tc_ids);
        while read_volatile(PMC_SR) & PMC_SR_MCKRDY == 0 {}
        write_volatile(UART0_MR, 0x8C0);
        write_volatile(UART0_BRGR, 65);
        write_volatile(UART0_CR, (1 << 4) | (1 << 6));
        write_volatile(PIOC_PER, LED_PIN);
        write_volatile(PIOC_OER, LED_PIN);
    }
    puts("SAM4S XPLAINED RENODE SMOKE\n");

    // 1. PMC: the enabled ids read back in PCSR0.
    let pcsr = unsafe { read_volatile(PMC_PCSR0) };
    puts("pmc pcsr0=");
    put_hex(pcsr, 8);
    let want = (1 << ID_UART0) | (1 << ID_PIOC) | tc_ids;
    finish("", pcsr & want == want, 0, &mut summary);

    // 2. TC0: UP_RC at MCK/8, RC compare pends NVIC 23; SR read releases it.
    wr(tc(0, TC_CMR), CMR_WAVE | CMR_UP_RC | MCK_8);
    wr(tc(0, TC_RC), 1000);
    wr(tc(0, TC_IER), SR_CPCS);
    let imr = rd(tc(0, TC_IMR));
    wr(tc(0, TC_CCR), CCR_CLKEN | CCR_SWTRG);
    let mut pends = 0;
    let mut released = 0;
    for _ in 0..3 {
        let mut pended = false;
        for _ in 0..100_000 {
            if unsafe { read_volatile(NVIC_ISPR0) } & (1 << ID_TC0) != 0 {
                pended = true;
                break;
            }
        }
        let sr = rd(tc(0, TC_SR));
        unsafe { write_volatile(NVIC_ICPR0, 1 << ID_TC0) };
        let clear = unsafe { read_volatile(NVIC_ISPR0) } & (1 << ID_TC0) == 0;
        if pended && sr & SR_CPCS != 0 {
            pends += 1;
        }
        if clear {
            released += 1;
        }
    }
    let cv = rd(tc(0, TC_CV));
    wr(tc(0, TC_IDR), SR_CPCS);
    wr(tc(0, TC_CCR), CCR_CLKDIS);
    puts("tc0 up_rc imr=");
    put_hex(imr, 2);
    puts(" pends=");
    put_hex(pends, 1);
    puts(" released=");
    put_hex(released, 1);
    put_flag(" cv<=rc=", cv <= 1000);
    finish(
        "",
        imr == SR_CPCS && pends == 3 && released == 3 && cv <= 1000,
        1,
        &mut summary,
    );

    // 3. TC1: free-running UP at MCK/2 advances and latches COVFS.
    wr(tc(1, TC_CMR), CMR_WAVE | MCK_2);
    wr(tc(1, TC_CCR), CCR_CLKEN | CCR_SWTRG);
    let a = rd(tc(1, TC_CV));
    spin(200);
    let b = rd(tc(1, TC_CV));
    let sr = wait_sr(1, SR_COVFS);
    let again = rd(tc(1, TC_SR));
    wr(tc(1, TC_CCR), CCR_CLKDIS);
    put_flag("tc1 up adv=", b != a);
    put_flag(" covfs=", sr & SR_COVFS != 0);
    put_flag(" cleared=", again & SR_COVFS == 0);
    finish(
        "",
        b != a && sr & SR_COVFS != 0 && again & SR_COVFS == 0,
        2,
        &mut summary,
    );

    // 4. TC2: CPCSTOP stops the counter at RC with the clock still enabled.
    wr(tc(2, TC_CMR), CMR_WAVE | CMR_CPCSTOP | MCK_128);
    wr(tc(2, TC_RC), 50);
    wr(tc(2, TC_CCR), CCR_CLKEN | CCR_SWTRG);
    let sr = wait_sr(2, SR_CPCS);
    let held = rd(tc(2, TC_CV));
    spin(2_000);
    let later = rd(tc(2, TC_CV));
    put_flag("tc2 cpcstop cpcs=", sr & SR_CPCS != 0);
    put_flag(" held=", held == later);
    put_flag(" clksta=", sr & SR_CLKSTA != 0);
    finish(
        "",
        sr & SR_CPCS != 0 && held == later && sr & SR_CLKSTA != 0,
        3,
        &mut summary,
    );

    // 5. TC3 (second block): CPCDIS clears CLKSTA on the RC compare.
    wr(tc(3, TC_CMR), CMR_WAVE | CMR_CPCDIS | MCK_8);
    wr(tc(3, TC_RC), 300);
    wr(tc(3, TC_CCR), CCR_CLKEN | CCR_SWTRG);
    let running = rd(tc(3, TC_SR)) & SR_CLKSTA != 0;
    let sr = wait_sr(3, SR_CPCS);
    put_flag("tc3 cpcdis running=", running);
    put_flag(" cpcs=", sr & SR_CPCS != 0);
    put_flag(" clksta=", sr & SR_CLKSTA != 0);
    finish(
        "",
        running && sr & SR_CPCS != 0 && sr & SR_CLKSTA == 0,
        4,
        &mut summary,
    );

    // 6. TC4/TC5: CLKEN alone holds CV at 0; BCR.SYNC starts both.
    wr(tc(4, TC_CMR), CMR_WAVE | MCK_8);
    wr(tc(5, TC_CMR), CMR_CPCTRG | MCK_2);
    wr(tc(5, TC_RC), 0xFFFF);
    wr(tc(5, TC_RA), 0x1234);
    let ra = rd(tc(5, TC_RA));
    wr(tc(4, TC_CCR), CCR_CLKEN);
    wr(tc(5, TC_CCR), CCR_CLKEN);
    spin(500);
    let idle = rd(tc(4, TC_CV)) | rd(tc(5, TC_CV));
    wr(TC1_BCR, 1);
    spin(500);
    let (c4, c5) = (rd(tc(4, TC_CV)), rd(tc(5, TC_CV)));
    put_flag("tc4 tc5 idle=", idle == 0);
    put_flag(" sync=", c4 != 0 && c5 != 0);
    puts(" ra=");
    put_hex(ra, 4);
    finish(
        "",
        idle == 0 && c4 != 0 && c5 != 0 && ra == 0,
        5,
        &mut summary,
    );

    // 7. PMC_PCDR0 gates TC5 off: its counter stops; TC4 keeps counting.
    unsafe { write_volatile(PMC_PCDR0, 1 << (ID_TC0 + 5)) };
    spin(50);
    let g0 = rd(tc(5, TC_CV));
    let t0 = rd(tc(4, TC_CV));
    spin(500);
    let g1 = rd(tc(5, TC_CV));
    let t1 = rd(tc(4, TC_CV));
    let gated = unsafe { read_volatile(PMC_PCSR0) } & (1 << (ID_TC0 + 5)) == 0;
    put_flag("tc5 pmc gated=", gated);
    put_flag(" held=", g0 == g1);
    put_flag(" tc4_runs=", t1 != t0);
    finish("", gated && g0 == g1 && t1 != t0, 6, &mut summary);

    wr(tc(4, TC_CCR), CCR_CLKDIS);
    wr(tc(5, TC_CCR), CCR_CLKDIS);
    unsafe { write_volatile(SUMMARY, 0x5A45_0000 | summary) };
    puts(if summary == 0x7F {
        "SAM4S PASS\n"
    } else {
        "SAM4S FAIL\n"
    });

    loop {
        unsafe { write_volatile(PIOC_SODR, LED_PIN) };
        spin(20_000);
        unsafe { write_volatile(PIOC_CODR, LED_PIN) };
        let _ = unsafe { read_volatile(PIOC_ODSR) };
        spin(20_000);
    }
}
