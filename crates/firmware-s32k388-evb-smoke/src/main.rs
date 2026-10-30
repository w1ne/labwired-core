// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! S32K388EVB-Q289 smoke firmware (CM7_0 only).
//!
//! The S32K388 boots CM7_0 through the boot header (IVT) at the start of
//! program flash: marker 0x5AA55AA5, BOOT_CONFIG bit 0 (CM7_0 enable) and the
//! CM7_0 start address at IVT + 0x0C (S32DS RTD `ivt_config.c`). This image's
//! IVT names 0x00401000, where cortex-m-rt puts the vector table.
//!
//! Bring-up order, as the RTD startup and Clock_Ip/Port drivers do it:
//! 1. SWT_0 is unlocked (SR = 0xC520, 0xD928) and CR.WEN cleared.
//! 2. MC_ME partition 1 gates SIUL2 (COFB1 REQ42) and LPUART2 (COFB2 REQ76):
//!    the CLKEN bits, PRTN1_PCONF.PCE, PRTN1_PUPD.PCUD, then the CTL_KEY
//!    0x5AF0 / 0xA50F sequence (Clock_Ip_Gate.c, Power_Ip_MC_ME.c).
//! 3. LPUART2 runs 8N1 at 115384 baud from the 24 MHz reset AIPS_SLOW_CLK
//!    (OSR 16, SBR 13).
//! 4. The six RGB LED pads (PTF17-19, PTF22-24) become GPIO outputs with the
//!    input buffer on (MSCR SSS = 0, OBE, IBE), then are walked through the
//!    GPDO bytes, PGPDO11 and MPGPDO11, reading the pads back through PGPDI11.
//! 5. PTA1 (SW10) is read with the internal pull-up, then pull-down.
//!
//! The firmware never starts the PLL, so the core runs at the 48 MHz FIRC
//! reset clock. It does not mux LPUART2 onto pads: which LPUART and pins the
//! EVB's USB-UART bridge uses is not confirmed (see VALIDATION.md).

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

/// Boot header (IVT), 256 bytes at 0x00400000 (`ivt_config.c` layout).
#[link_section = ".ivt.s32k3"]
#[used]
static IVT: [u32; 64] = {
    let mut ivt = [0xFFFF_FFFFu32; 64];
    ivt[0] = IVT_MARKER;
    ivt[1] = 0x0000_0001; // BOOT_CONFIG: CM7_0 enabled
    ivt[3] = APP_START; // CM7_0 application start address
    ivt[9] = 0x0000_0000; // life-cycle configuration
    ivt
};

const IVT_BASE: u32 = 0x0040_0000;
const IVT_MARKER: u32 = 0x5AA5_5AA5;
const APP_START: u32 = 0x0040_1000;
const SCB_VTOR: *const u32 = 0xE000_ED08 as *const u32;

// SWT_0 @ 0x40270000: CR @ 0x00 (WEN = bit 0), SR @ 0x10.
const SWT0_CR: *mut u32 = 0x4027_0000 as *mut u32;
const SWT0_SR: *mut u32 = 0x4027_0010 as *mut u32;

// MC_ME @ 0x402DC000.
const MC_ME: u32 = 0x402D_C000;
const MC_ME_CTL_KEY: u32 = 0x000;
const MC_ME_PRTN1_PCONF: u32 = 0x300;
const MC_ME_PRTN1_PUPD: u32 = 0x304;
const MC_ME_PRTN1_COFB1_CLKEN: u32 = 0x334;
const MC_ME_PRTN1_COFB2_CLKEN: u32 = 0x338;
const REQ42_SIUL2: u32 = 1 << (42 - 32);
const REQ76_LPUART2: u32 = 1 << (76 - 64);

// SIUL2 @ 0x40290000.
const SIUL2: u32 = 0x4029_0000;
const MSCR_PUS: u32 = 1 << 11;
const MSCR_PUE: u32 = 1 << 13;
const MSCR_IBE: u32 = 1 << 19;
const MSCR_OBE: u32 = 1 << 21;
/// PTF16-31 is parallel port 11; PGPDO/PGPDI halfwords are swapped in pairs.
const PGPDO11: u32 = 0x1700 + 2 * (11 ^ 1);
const PGPDI11: u32 = 0x1740 + 2 * (11 ^ 1);
const MPGPDO11: u32 = 0x1780 + 4 * 11;

// LPUART2 @ 0x40330000: BAUD @ 0x10, STAT @ 0x14, CTRL @ 0x18, DATA @ 0x1C.
const LPUART2: u32 = 0x4033_0000;
const BAUD: u32 = 0x10;
const STAT: u32 = 0x14;
const CTRL: u32 = 0x18;
const DATA: u32 = 0x1C;
const STAT_TDRE: u32 = 1 << 23;
const CTRL_TE: u32 = 1 << 19;
const CTRL_RE: u32 = 1 << 18;
const OSR: u32 = 16;
const SBR: u32 = 13;
const AIPS_SLOW_HZ: u32 = 24_000_000;

/// The RGB LED pads, as MSCR indices (32 * port + pin), with their names.
const LEDS: [(u32, &str); 6] = [
    (177, "ptf17"),
    (178, "ptf18"),
    (179, "ptf19"),
    (182, "ptf22"),
    (183, "ptf23"),
    (184, "ptf24"),
];
const PTF23: u32 = 183;
const PTA1: u32 = 1;

fn rd(base: u32, offset: u32) -> u32 {
    unsafe { read_volatile((base + offset) as *const u32) }
}

fn wr(base: u32, offset: u32, value: u32) {
    unsafe { write_volatile((base + offset) as *mut u32, value) }
}

fn rd16(addr: u32) -> u16 {
    unsafe { read_volatile(addr as *const u16) }
}

fn wr16(addr: u32, value: u16) {
    unsafe { write_volatile(addr as *mut u16, value) }
}

fn mscr(pin: u32) -> u32 {
    0x240 + 4 * pin
}

fn gpdo_write(pin: u32, level: bool) {
    unsafe { write_volatile((SIUL2 + 0x1300 + (pin ^ 3)) as *mut u8, level as u8) }
}

fn gpdi(pin: u32) -> u32 {
    unsafe { read_volatile((SIUL2 + 0x1500 + (pin ^ 3)) as *const u8) as u32 }
}

/// PGPDO/PGPDI bit for pin `16 * port + k` is `15 - k`.
fn port11_bit(pin: u32) -> u16 {
    1 << (15 - (pin - 176))
}

fn putc(byte: u8) {
    while rd(LPUART2, STAT) & STAT_TDRE == 0 {}
    wr(LPUART2, DATA, byte as u32);
}

fn puts(text: &str) {
    for byte in text.bytes() {
        putc(byte);
    }
}

fn put_hex(value: u32, digits: u32) {
    for shift in (0..digits).rev() {
        let nibble = ((value >> (shift * 4)) & 0xF) as u8;
        putc(match nibble {
            0..=9 => b'0' + nibble,
            _ => b'a' + nibble - 10,
        });
    }
}

fn put_dec32(value: u32) {
    let mut digits = [0u8; 10];
    let mut remaining = value;
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    for &digit in &digits[start..] {
        putc(digit);
    }
}

#[inline(never)]
fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

fn report_port11(label: &str) {
    puts(label);
    puts(" pgpdo11=");
    put_hex(rd16(SIUL2 + PGPDO11) as u32, 4);
    puts(" pgpdi11=");
    put_hex(rd16(SIUL2 + PGPDI11) as u32, 4);
    putc(b'\n');
}

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(SWT0_SR, 0xC520);
        write_volatile(SWT0_SR, 0xD928);
        let cr = read_volatile(SWT0_CR);
        write_volatile(SWT0_CR, cr & !1);
    }

    wr(
        MC_ME,
        MC_ME_PRTN1_COFB1_CLKEN,
        rd(MC_ME, MC_ME_PRTN1_COFB1_CLKEN) | REQ42_SIUL2,
    );
    wr(
        MC_ME,
        MC_ME_PRTN1_COFB2_CLKEN,
        rd(MC_ME, MC_ME_PRTN1_COFB2_CLKEN) | REQ76_LPUART2,
    );
    wr(MC_ME, MC_ME_PRTN1_PCONF, 1);
    wr(MC_ME, MC_ME_PRTN1_PUPD, 1);
    wr(MC_ME, MC_ME_CTL_KEY, 0x5AF0);
    wr(MC_ME, MC_ME_CTL_KEY, 0xA50F);

    // LPUART2: transmitter/receiver off, baud, 8N1, then on.
    wr(LPUART2, CTRL, 0);
    wr(LPUART2, BAUD, ((OSR - 1) << 24) | SBR);
    wr(LPUART2, CTRL, CTRL_TE | CTRL_RE);

    for &(pin, _) in LEDS.iter() {
        gpdo_write(pin, false);
        wr(SIUL2, mscr(pin), MSCR_OBE | MSCR_IBE);
    }

    puts("S32K388 EVB SMOKE\n");
    puts("cpu=cortex-m7 core=cm7_0 uart=lpuart2\n");
    puts("ivt marker=");
    put_hex(rd(IVT_BASE, 0), 8);
    puts(" cm7_0=");
    put_hex(rd(IVT_BASE, 0x0C), 8);
    puts(" vtor=");
    put_hex(unsafe { read_volatile(SCB_VTOR) }, 8);
    putc(b'\n');
    puts("clk aips_slow=");
    put_dec32(AIPS_SLOW_HZ);
    puts(" osr=");
    put_dec32(OSR);
    puts(" sbr=");
    put_dec32(SBR);
    puts(" baud=");
    put_dec32(AIPS_SLOW_HZ / (OSR * SBR));
    putc(b'\n');

    report_port11("leds=off");
    for &(pin, name) in LEDS.iter() {
        gpdo_write(pin, true);
        puts("on ");
        puts(name);
        puts(" gpdi=");
        put_dec32(gpdi(pin));
        report_port11("");
    }
    // Clear PTF17-19 through MPGPDO11 (MASK = bits 31:16, data 0).
    let mask = port11_bit(177) | port11_bit(178) | port11_bit(179);
    wr(SIUL2, MPGPDO11, (mask as u32) << 16);
    report_port11("mpgpdo");
    wr16(SIUL2 + PGPDO11, 0);
    report_port11("pgpdo");

    wr(SIUL2, mscr(PTA1), MSCR_IBE | MSCR_PUE | MSCR_PUS);
    puts("pta1 pu=");
    put_dec32(gpdi(PTA1));
    wr(SIUL2, mscr(PTA1), MSCR_IBE | MSCR_PUE);
    puts(" pd=");
    put_dec32(gpdi(PTA1));
    putc(b'\n');

    let mut heartbeat: u32 = 0;
    loop {
        // Toggle the green LED (PTF23) through PGPDO11 each beat.
        let out = rd16(SIUL2 + PGPDO11) ^ port11_bit(PTF23);
        wr16(SIUL2 + PGPDO11, out);
        puts("hb=");
        put_dec32(heartbeat);
        puts(" ptf23=");
        put_dec32(gpdi(PTF23));
        putc(b'\n');
        spin(100_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
