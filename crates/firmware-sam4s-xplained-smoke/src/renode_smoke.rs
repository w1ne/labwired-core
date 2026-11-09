// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SAM4S Xplained smoke for the imported Renode platform (`sam4s_xplained`).
//!
//! The demo bin (`src/main.rs`) prints a banner and the LED/button state; this
//! one first runs the SAM4S16C family smoke (`firmware-sam4s-smoke/src/
//! family.rs`: EEFC, RSTC software reset, WDT reset and window interrupt,
//! PMC, PIO latches, TC0 — three boots, summary 0x5A400FFF at 0x2001FF00),
//! then the board checks below on the watchdog boot:
//!
//! * TC0 — UP_RC waveform, MCK/8, RC compare pends NVIC line 23 (IMR.CPCS);
//!   reading TC_SR clears CPCS and lets the pend be cleared.
//! * TC1 — UP waveform at MCK/2 runs past 0xFFFF and latches COVFS.
//! * TC2 — CPCSTOP: the counter stops at RC, the clock stays enabled.
//! * TC3 — CPCDIS: the RC compare clears SR.CLKSTA.
//! * TC4/TC5 — CLKEN alone does not count; BCR.SYNC on the second block
//!   starts both. TC5 runs in capture mode with CPCTRG, where RA ignores
//!   stores, and stops counting once PMC_PCDR0 gates its clock off.
//! * LEDs D9 (PC10) and D10 (PC17) — configured as ASF does
//!   (PIO_OUTPUT_1, i.e. off on this active-low board).
//! * BP2 (PA5) — ASF's PIO_PULLUP | PIO_IT_RISE_EDGE: the press (falling)
//!   does not pend PIOA's line 11, the release (rising) does, and the
//!   PIO_ISR read releases it. The stimuli press and release the button.
//! * USART0 — US_IER enables TXRDY on NVIC line 14; US_IDR drops the mask and
//!   releases the level pending bit.
//!
//! Every board check sets a bit in a second summary word at 0x2001FF04;
//! 0x5A4503FF means all ten passed. PC10 then toggles in the idle loop.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

#[path = "../../firmware-sam4s-smoke/src/family.rs"]
mod family;

const PMC_PCDR0: *mut u32 = 0x400E_0414 as *mut u32;
const PMC_PCSR0: *mut u32 = 0x400E_0418 as *mut u32;

const PIOA: u32 = 0x400E_0E00;
const PIOC: u32 = 0x400E_1200;
const PIO_PER: u32 = 0x00;
const PIO_OER: u32 = 0x10;
const PIO_ODR: u32 = 0x14;
const PIO_SODR: u32 = 0x30;
const PIO_CODR: u32 = 0x34;
const PIO_ODSR: u32 = 0x38;
const PIO_PDSR: u32 = 0x3C;
const PIO_IER: u32 = 0x40;
const PIO_IDR: u32 = 0x44;
const PIO_IMR: u32 = 0x48;
const PIO_ISR: u32 = 0x4C;
const PIO_PUER: u32 = 0x64;
const PIO_AIMER: u32 = 0xB0;
const PIO_ESR: u32 = 0xC0;
const PIO_REHLSR: u32 = 0xD4;
const LED_D9: u32 = 1 << 10;
const LED_D10: u32 = 1 << 17;
const BP2: u32 = 1 << 5;

const UART0_SR: *const u32 = 0x400E_0614 as *const u32;
const UART0_THR: *mut u32 = 0x400E_061C as *mut u32;

const USART0: u32 = 0x4002_4000;
const US_CR: u32 = 0x00;
const US_MR: u32 = 0x04;
const US_IER: u32 = 0x08;
const US_IDR: u32 = 0x0C;
const US_IMR: u32 = 0x10;
const US_BRGR: u32 = 0x20;
const US_CR_RSTRX: u32 = 1 << 2;
const US_CR_RSTTX: u32 = 1 << 3;
const US_CR_RXEN: u32 = 1 << 4;
const US_CR_TXEN: u32 = 1 << 6;
const US_MR_8N1: u32 = 0x8C0;
const US_CSR_TXRDY: u32 = 1 << 1;

const NVIC_ISPR0: *const u32 = 0xE000_E200 as *const u32;
const NVIC_ICPR0: *mut u32 = 0xE000_E280 as *mut u32;

/// Peripheral ids (= NVIC lines = PMC_PCSR0 bits).
const ID_PIOA: u32 = 11;
const ID_TC0: u32 = 23;
const ID_USART0: u32 = 14;

const BOARD_SUMMARY: *mut u32 = 0x2001_FF04 as *mut u32;

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

fn pio(port: u32, reg: u32) -> *mut u32 {
    (port + reg) as *mut u32
}

fn tc(n: u32, reg: u32) -> *mut u32 {
    (0x4001_0000 + 0x4000 * (n / 3) + 0x40 * (n % 3) + reg) as *mut u32
}

fn usart0(reg: u32) -> *mut u32 {
    (USART0 + reg) as *mut u32
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
    family::run(&family::Part {
        name: "SAM4S XPLAINED",
        flash_bytes: 1024 * 1024,
        has_pioc: true,
        summary: 0x2001_FF00,
        board: Some(board),
    })
}

/// Bounded wait for `ready`; false on time-out.
fn wait(ready: impl Fn() -> bool) -> bool {
    for _ in 0..2_000_000 {
        if ready() {
            return true;
        }
    }
    false
}

fn pioa_pended() -> bool {
    (unsafe { read_volatile(NVIC_ISPR0) } & (1 << ID_PIOA)) != 0
}

fn board() -> ! {
    let mut summary = 0u32;
    puts("SAM4S XPLAINED BOARD\n");
    // The family smoke enabled UART0/1, USART0/1, PIOA-C and TC0-5 in
    // PMC_PCER0; the counters below run on MCK = the 4 MHz reset clock.

    // 1. TC0: UP_RC at MCK/8, RC compare pends NVIC 23; SR read releases it.
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
        0,
        &mut summary,
    );

    // 2. TC1: free-running UP at MCK/2 advances and latches COVFS.
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
        1,
        &mut summary,
    );

    // 3. TC2: CPCSTOP stops the counter at RC with the clock still enabled.
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
        2,
        &mut summary,
    );

    // 4. TC3 (second block): CPCDIS clears CLKSTA on the RC compare.
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
        3,
        &mut summary,
    );

    // 5. TC4/TC5: CLKEN alone holds CV at 0; BCR.SYNC starts both.
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
        4,
        &mut summary,
    );

    // 6. PMC_PCDR0 gates TC5 off: its counter stops; TC4 keeps counting.
    wr(PMC_PCDR0, 1 << (ID_TC0 + 5));
    spin(50);
    let g0 = rd(tc(5, TC_CV));
    let t0 = rd(tc(4, TC_CV));
    spin(500);
    let g1 = rd(tc(5, TC_CV));
    let t1 = rd(tc(4, TC_CV));
    let gated = rd(PMC_PCSR0) & (1 << (ID_TC0 + 5)) == 0;
    put_flag("tc5 pmc gated=", gated);
    put_flag(" held=", g0 == g1);
    put_flag(" tc4_runs=", t1 != t0);
    finish("", gated && g0 == g1 && t1 != t0, 5, &mut summary);

    wr(tc(4, TC_CCR), CCR_CLKDIS);
    wr(tc(5, TC_CCR), CCR_CLKDIS);

    // 7. LEDs as ASF configures them: PIO_OUTPUT_1 = off (active low).
    wr(pio(PIOC, PIO_SODR), LED_D9 | LED_D10);
    wr(pio(PIOC, PIO_PER), LED_D9 | LED_D10);
    wr(pio(PIOC, PIO_OER), LED_D9 | LED_D10);
    let off = rd(pio(PIOC, PIO_PDSR)) & (LED_D9 | LED_D10);
    wr(pio(PIOC, PIO_CODR), LED_D10);
    let lit = rd(pio(PIOC, PIO_ODSR)) & (LED_D9 | LED_D10);
    wr(pio(PIOC, PIO_SODR), LED_D10);
    puts("leds off=");
    put_hex(off >> 10, 2);
    puts(" d10 lit=");
    put_hex(lit >> 10, 2);
    finish(
        "",
        off == LED_D9 | LED_D10 && lit == LED_D9,
        6,
        &mut summary,
    );

    // 8. BP2: pull-up input, rising-edge interrupt (ASF PIN_PUSHBUTTON_1).
    wr(pio(PIOA, PIO_ODR), BP2);
    wr(pio(PIOA, PIO_PER), BP2);
    wr(pio(PIOA, PIO_PUER), BP2);
    wr(pio(PIOA, PIO_AIMER), BP2);
    wr(pio(PIOA, PIO_ESR), BP2);
    wr(pio(PIOA, PIO_REHLSR), BP2);
    let _ = rd(pio(PIOA, PIO_ISR));
    unsafe { write_volatile(NVIC_ICPR0, 1 << ID_PIOA) };
    wr(pio(PIOA, PIO_IER), BP2);
    let imr = rd(pio(PIOA, PIO_IMR));
    let idle = rd(pio(PIOA, PIO_PDSR)) & BP2 != 0 && !pioa_pended();
    puts("bp2 wait\n");
    let pressed = wait(|| rd(pio(PIOA, PIO_PDSR)) & BP2 == 0);
    let quiet_on_press = !pioa_pended();
    puts("bp2 pressed\n");
    let released = wait(pioa_pended);
    let isr = rd(pio(PIOA, PIO_ISR));
    unsafe { write_volatile(NVIC_ICPR0, 1 << ID_PIOA) };
    let cleared = !pioa_pended() && rd(pio(PIOA, PIO_ISR)) == 0;
    wr(pio(PIOA, PIO_IDR), BP2);
    puts("bp2 imr=");
    put_hex(imr, 2);
    put_flag(" idle=", idle);
    put_flag(" pressed=", pressed);
    put_flag(" quiet=", quiet_on_press);
    put_flag(" release_pend=", released);
    puts(" isr=");
    put_hex(isr, 2);
    put_flag(" cleared=", cleared);
    finish(
        "",
        imr == BP2 && idle && pressed && quiet_on_press && released && isr == BP2 && cleared,
        7,
        &mut summary,
    );

    // 9. The button reads back released.
    let level = rd(pio(PIOA, PIO_PDSR)) & BP2 != 0;
    put_flag("bp2 released level=", level);
    finish("", level, 8, &mut summary);

    // 10. USART0 TXRDY is a level source on NVIC 14. Enabling its mask
    // asserts the line; disabling TXRDY releases the pending NVIC level.
    wr(usart0(US_CR), US_CR_RSTRX | US_CR_RSTTX);
    wr(usart0(US_MR), US_MR_8N1);
    wr(usart0(US_BRGR), 2); // 4 MHz reset clock / (16 * 115200), rounded.
    wr(usart0(US_CR), US_CR_RXEN | US_CR_TXEN);
    unsafe { write_volatile(NVIC_ICPR0, 1 << ID_USART0) };
    wr(usart0(US_IER), US_CSR_TXRDY);
    spin(8);
    let imr = rd(usart0(US_IMR));
    let pended = unsafe { read_volatile(NVIC_ISPR0) } & (1 << ID_USART0) != 0;
    wr(usart0(US_IDR), US_CSR_TXRDY);
    let released =
        rd(usart0(US_IMR)) == 0 && unsafe { read_volatile(NVIC_ISPR0) } & (1 << ID_USART0) == 0;
    puts("usart0 txrdy imr=");
    put_hex(imr, 1);
    put_flag(" pend=", pended);
    put_flag(" released=", released);
    finish(
        "",
        imr == US_CSR_TXRDY && pended && released,
        9,
        &mut summary,
    );

    unsafe { write_volatile(BOARD_SUMMARY, 0x5A45_0000 | summary) };
    puts(if summary == 0x3FF {
        "SAM4S XPLAINED BOARD PASS\n"
    } else {
        "SAM4S XPLAINED BOARD FAIL\n"
    });

    loop {
        wr(pio(PIOC, PIO_CODR), LED_D9);
        spin(20_000);
        wr(pio(PIOC, PIO_SODR), LED_D9);
        spin(20_000);
    }
}
