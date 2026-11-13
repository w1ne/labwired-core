// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! SAM4S family smoke shared by the `sam4s`, `sam4s16c`, `sam4s8b` and
//! `sam4s_xplained` rows (the others include this file by `#[path]`).
//!
//! The run spans three boots, told apart by RSTC_SR.RSTTYP:
//!
//! 1. General reset (0): EEFC FMR wait states, GETD descriptor, lock bits,
//!    GPNVM bits, a bad-key FCMDE; RSTC_SR and the armed-at-reset WDT_MR
//!    (0x3FFF_2FFF). Then RSTC_CR PROCRST|PERRST with the key.
//! 2. Software reset (3): WDT_MR = WDRSTEN with WDV = 2 and waits for the
//!    watchdog to reboot the core.
//! 3. Watchdog reset (2): WDT_MR is back at its reset value (the processor
//!    reset unlocked the write-once register); a restart outside the WDD
//!    window raises WDERR and pends NVIC line 4 (WDFIEN, no reset), the SR
//!    read releases it, and a second WDT_MR write is ignored. Then PMC
//!    peripheral clocks, the PIO set/clear latch on every port the package
//!    bonds out, and TC0's RC compare on NVIC line 23. Selected parts also
//!    check USART0/USART1 TXRDY interrupts on NVIC lines 14/15.
//!
//! Check bits persist across the reboots in a word below the summary (not
//! in .bss, which cortex-m-rt zeroes on every boot). Summary =
//! 0x5A40_0000 | bits; 0x5A40_0FFF means all twelve base checks passed.
//! Parts with `check_usart0_irq` set bit 12; those with `check_usart1_irq`
//! set bit 13.

use core::ptr::{read_volatile, write_volatile};

pub struct Part {
    pub name: &'static str,
    pub flash_bytes: u32,
    pub has_pioc: bool,
    /// Check USART0 TXRDY IRQ 14 for parts whose imported profile wires it.
    pub check_usart0_irq: bool,
    /// Check USART1 TXRDY IRQ 15 for parts whose imported profile wires it.
    pub check_usart1_irq: bool,
    /// RAM end - 0x100.
    pub summary: u32,
    /// Board-level checks, run after the family summary on the watchdog
    /// boot (the SAM4S Xplained row). Never returns.
    pub board: Option<fn() -> !>,
}

const PMC_PCER0: u32 = 0x400E_0410;
const PMC_PCSR0: u32 = 0x400E_0418;
const PMC_SR: u32 = 0x400E_0468;

const UART0_CR: u32 = 0x400E_0600;
const UART0_MR: u32 = 0x400E_0604;
const UART0_SR: u32 = 0x400E_0614;
const UART0_THR: u32 = 0x400E_061C;
const UART0_BRGR: u32 = 0x400E_0620;

const EEFC_FMR: u32 = 0x400E_0A00;
const EEFC_FCR: u32 = 0x400E_0A04;
const EEFC_FSR: u32 = 0x400E_0A08;
const EEFC_FRR: u32 = 0x400E_0A0C;
const FKEY: u32 = 0x5A << 24;

const RSTC_CR: u32 = 0x400E_1400;
const RSTC_SR: u32 = 0x400E_1404;

const WDT_CR: u32 = 0x400E_1450;
const WDT_MR: u32 = 0x400E_1454;
const WDT_SR: u32 = 0x400E_1458;
const WDT_MR_RESET: u32 = 0x3FFF_2FFF;
const WDFIEN: u32 = 1 << 12;
const WDRSTEN: u32 = 1 << 13;
const WDDIS: u32 = 1 << 15;
const WDT_RESTART: u32 = 0xA500_0001;

const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ICPR0: u32 = 0xE000_E280;
const IRQ_WDT: u32 = 4;
const IRQ_USART0: u32 = 14;
const IRQ_USART1: u32 = 15;
const USART0: u32 = 0x4002_4000;
const USART1: u32 = 0x4002_8000;

const ID_UART0: u32 = 8;
const ID_UART1: u32 = 9;
const ID_PIOA: u32 = 11;
const ID_USART0: u32 = 14;
const ID_TC0: u32 = 23;

const PIO_BASES: [u32; 3] = [0x400E_0E00, 0x400E_1000, 0x400E_1200];
const PIO_PER: u32 = 0x00;
const PIO_OER: u32 = 0x10;
const PIO_SODR: u32 = 0x30;
const PIO_CODR: u32 = 0x34;
const PIO_ODSR: u32 = 0x38;

const TC0: u32 = 0x4001_0000;

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn putc(byte: u8) {
    while rd(UART0_SR) & 2 == 0 {}
    wr(UART0_THR, byte as u32);
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

fn finish(label: &str, ok: bool, bit: u32, state: u32) {
    puts(label);
    puts(if ok { " ok\n" } else { " FAIL\n" });
    if ok {
        wr(state, rd(state) | 1 << bit);
    }
}

#[inline(never)]
fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}

fn eefc(cmd: u32, arg: u32) {
    wr(EEFC_FCR, FKEY | (arg << 8) | cmd);
    while rd(EEFC_FSR) & 1 == 0 {}
}

fn boot_general(part: &Part, state: u32) {
    wr(state, 0);

    wr(EEFC_FMR, 5 << 8);
    let fmr = rd(EEFC_FMR);
    puts("eefc fmr=");
    put_hex(fmr, 8);
    finish("", fmr == 5 << 8, 0, state);

    eefc(0x00, 0); // GETD
    let mut d = [0u32; 8];
    for word in d.iter_mut() {
        *word = rd(EEFC_FRR);
    }
    let regions = part.flash_bytes / 8192;
    puts("eefc getd size=");
    put_hex(d[1], 8);
    puts(" page=");
    put_hex(d[2], 4);
    puts(" locks=");
    put_hex(d[5], 4);
    let getd_ok = d[1] == part.flash_bytes
        && d[2] == 512
        && d[3] == 1
        && d[4] == part.flash_bytes
        && d[5] == regions
        && d[6] == 8192
        && d[7] == 8192;
    finish("", getd_ok, 1, state);

    eefc(0x0D, 0); // GGPB
    let boot = rd(EEFC_FRR);
    eefc(0x0B, 0); // SGPB 0
    eefc(0x0D, 0);
    let set = rd(EEFC_FRR);
    eefc(0x0C, 0); // CGPB 0
    eefc(0x08, 20); // SLB on page 20: lock region 1
    eefc(0x0A, 0); // GLB
    let locks = rd(EEFC_FRR);
    eefc(0x09, 20); // CLB
    eefc(0x0A, 0);
    let unlocked = rd(EEFC_FRR);
    puts("eefc gpnvm=");
    put_hex(boot, 1);
    puts(" set=");
    put_hex(set, 1);
    puts(" glb=");
    put_hex(locks, 1);
    puts(" clb=");
    put_hex(unlocked, 1);
    finish(
        "",
        boot == 0b10 && set == 0b11 && locks == 0b10 && unlocked == 0,
        2,
        state,
    );

    wr(EEFC_FCR, 0x00); // no FKEY
    let bad = rd(EEFC_FSR);
    let after = rd(EEFC_FSR);
    puts("eefc fsr badkey=");
    put_hex(bad, 1);
    puts(" after=");
    put_hex(after, 1);
    finish("", bad == 0b11 && after == 0b01, 3, state);

    let sr = rd(RSTC_SR);
    let mr = rd(WDT_MR);
    puts("rstc sr=");
    put_hex(sr, 8);
    puts(" wdt mr=");
    put_hex(mr, 8);
    finish(
        "",
        (sr >> 8) & 7 == 0 && sr & (1 << 16) != 0 && mr == WDT_MR_RESET,
        4,
        state,
    );

    puts("rstc software reset\n");
    wr(RSTC_CR, 0xA500_0005); // PROCRST | PERRST
    spin(10_000);
    puts("rstc reset did not happen\n");
}

fn boot_software(state: u32) {
    finish("rstc rsttyp=3", true, 5, state);
    puts("wdt arm wdv=2\n");
    wr(WDT_MR, WDRSTEN | (0xFFF << 16) | 2);
    // Two counts at SLCK/128 from the 4 MHz reset clock: ~31k cycles.
    spin(200_000);
    puts("wdt reset did not happen\n");
}

fn boot_watchdog(state: u32) {
    let mr = rd(WDT_MR);
    puts("wdt rsttyp=2 mr=");
    put_hex(mr, 8);
    finish("", mr == WDT_MR_RESET, 6, state);

    // WDV 0x100, WDD 0x80, WDFIEN without WDRSTEN: restarting at once (counter
    // 0x100 > WDD) is a window error.
    wr(WDT_MR, (0x80 << 16) | WDFIEN | 0x100);
    wr(WDT_CR, WDT_RESTART);
    let pended = rd(NVIC_ISPR0) & (1 << IRQ_WDT) != 0;
    let sr = rd(WDT_SR);
    wr(NVIC_ICPR0, 1 << IRQ_WDT);
    let released = rd(NVIC_ISPR0) & (1 << IRQ_WDT) == 0 && rd(WDT_SR) == 0;
    puts("wdt window sr=");
    put_hex(sr, 1);
    puts(if pended { " pended" } else { " idle" });
    puts(if released { " released" } else { " stuck" });
    finish("", sr == 2 && pended && released, 7, state);

    wr(WDT_MR, WDDIS);
    let locked = rd(WDT_MR);
    puts("wdt mr once=");
    put_hex(locked, 8);
    finish("", locked == (0x80 << 16) | WDFIEN | 0x100, 8, state);
}

fn peripherals(part: &Part, state: u32) {
    let ports = if part.has_pioc { 3 } else { 2 };
    let mut want = (1 << ID_UART0) | (1 << ID_UART1) | (3 << ID_USART0) | (0x3F << ID_TC0);
    want |= ((1 << ports) - 1) << ID_PIOA;
    wr(PMC_PCER0, want);
    let pcsr = rd(PMC_PCSR0);
    puts("pmc pcsr0=");
    put_hex(pcsr, 8);
    finish("", pcsr & want == want, 9, state);

    let mut pio_ok = true;
    for &base in PIO_BASES.iter().take(ports) {
        let pin = 1 << 3;
        wr(base + PIO_PER, pin);
        wr(base + PIO_OER, pin);
        wr(base + PIO_SODR, pin);
        let high = rd(base + PIO_ODSR) & pin != 0;
        wr(base + PIO_CODR, pin);
        let low = rd(base + PIO_ODSR) & pin == 0;
        pio_ok &= high && low;
    }
    puts("pio ports=");
    put_hex(ports as u32, 1);
    finish("", pio_ok, 10, state);

    // TC0 UP_RC at MCK/8; the RC compare pends NVIC 23.
    wr(TC0 + 0x04, (1 << 15) | (2 << 13) | 1);
    wr(TC0 + 0x1C, 500);
    wr(TC0 + 0x24, 1 << 4);
    wr(TC0, 0b101);
    let mut pended = false;
    for _ in 0..100_000 {
        if rd(NVIC_ISPR0) & (1 << ID_TC0) != 0 {
            pended = true;
            break;
        }
    }
    let sr = rd(TC0 + 0x20);
    wr(TC0, 0b10);
    puts("tc0 rc pend=");
    put_hex(pended as u32, 1);
    finish("", pended && sr & (1 << 4) != 0, 11, state);

    if part.check_usart0_irq {
        check_usart_txrdy_irq(USART0, IRQ_USART0, "usart0", 12, state);
    }
    if part.check_usart1_irq {
        check_usart_txrdy_irq(USART1, IRQ_USART1, "usart1", 13, state);
    }
}

fn check_usart_txrdy_irq(base: u32, irq: u32, label: &str, bit: u32, state: u32) {
    // TXRDY is already set after TXEN; IER exposes it as a level IRQ and IDR
    // removes the source so the NVIC pending level must fall.
    wr(base, (1 << 2) | (1 << 3)); // RSTRX | RSTTX
    wr(base + 0x04, 0x8C0); // 8N1
    wr(base + 0x20, 2); // 4 MHz reset clock / (16 * 115200), rounded
    wr(base, (1 << 4) | (1 << 6)); // RXEN | TXEN
    wr(NVIC_ICPR0, 1 << irq);
    wr(base + 0x08, 1 << 1); // IER TXRDY
    let imr = rd(base + 0x10);
    let irq_pended = rd(NVIC_ISPR0) & (1 << irq) != 0;
    wr(base + 0x0C, 1 << 1); // IDR TXRDY
    let irq_released = rd(base + 0x10) == 0 && rd(NVIC_ISPR0) & (1 << irq) == 0;
    puts(label);
    puts(" txrdy imr=");
    put_hex(imr, 1);
    puts(if irq_pended { " pend=1" } else { " pend=0" });
    puts(if irq_released {
        " released=1"
    } else {
        " released=0"
    });
    finish(
        "",
        imr == (1 << 1) && irq_pended && irq_released,
        bit,
        state,
    );
}

pub fn run(part: &Part) -> ! {
    let state = part.summary - 0x100;
    wr(PMC_PCER0, 1 << ID_UART0);
    while rd(PMC_SR) & (1 << 3) == 0 {}
    wr(UART0_MR, 0x8C0);
    wr(UART0_BRGR, 65);
    wr(UART0_CR, (1 << 4) | (1 << 6));
    puts(part.name);
    puts(" RENODE SMOKE\n");

    match (rd(RSTC_SR) >> 8) & 7 {
        0 => boot_general(part, state),
        3 => boot_software(state),
        2 => {
            boot_watchdog(state);
            peripherals(part, state);
        }
        other => {
            puts("unexpected rsttyp=");
            put_hex(other, 1);
            puts("\n");
        }
    }

    let bits = rd(state);
    let expected_bits = 0x0FFF
        | (u32::from(part.check_usart0_irq) << 12)
        | (u32::from(part.check_usart1_irq) << 13);
    wr(part.summary, 0x5A40_0000 | bits);
    puts(part.name);
    puts(if bits == expected_bits {
        " PASS\n"
    } else {
        " FAIL\n"
    });
    if let (Some(board), 2) = (part.board, (rd(RSTC_SR) >> 8) & 7) {
        board();
    }
    loop {
        spin(20_000);
    }
}
