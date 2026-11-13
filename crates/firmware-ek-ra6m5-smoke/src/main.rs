// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! EK-RA6M5 (R7FA6M5BH, Cortex-M33) smoke firmware: a running light over the
//! three user LEDs plus a heartbeat line on the SCI0 console, in bare
//! registers.
//!
//! Board facts (Zephyr `ek_ra6m5` ek_ra6m5.dts / ek_ra6m5-pinctrl.dtsi):
//!   LED1 (blue) P006, LED2 (green) P007, LED3 (red) P008 — active high
//!   S1 P005, S2 P004 — active low, pulled up
//!   Console SCI0 P411 (TXD0) / P410 (RXD0), J-Link OB virtual COM port
//!
//! The RA6M5 resets onto MOCO (8 MHz, SCKSCR 0x01) with every divider at /4
//! (SCKDIVCR 0x22022222; FSP ra6m5 bsp_feature.h). The firmware unlocks
//! PRCR.PRC0 and sets SCKDIVCR to /1, takes SCI0 out of module stop
//! (MSTPCRB.MSTPB31), routes P410/P411 to SCI0 and enables the S1/S2 pull-ups
//! through PmnPFS, walks the LEDs and prints the PODR readback, then reads the
//! switches and heartbeats.
//!
//! Baud: PCLKA = MOCO 8 MHz / 1. Async, SMR.CKS = 0, SEMR.ABCS = BGDM = 0:
//! BRR = PCLKA / (32 * 9600) - 1 = 25.04 -> 25 (9615 baud, +0.16 %). 115200
//! is not reachable from 8 MHz without the MDDR modulator, so the console runs
//! at 9600 8N1.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// R_SYSTEM @ 0x4001E000 (R7FA6M5BH.h): SCKDIVCR @ 0x20, PRCR @ 0x3FE.
const SYSTEM: u32 = 0x4001_E000;
const SCKDIVCR: *mut u32 = (SYSTEM + 0x20) as *mut u32;
const PRCR: *mut u16 = (SYSTEM + 0x3FE) as *mut u16;
const PRCR_KEY: u16 = 0xA500;
const PRCR_PRC0: u16 = 1 << 0;

// R_MSTP @ 0x40084000: MSTPCRB @ 0x04, SCI0 is MSTPB31 (Zephyr
// `clocks = <&pclka MSTPB 31>`; 0 = module running).
const MSTPCRB: *mut u32 = 0x4008_4004 as *mut u32;
const MSTPB31_SCI0: u32 = 1 << 31;

// PORT0 @ 0x40080000: PCNTR1 PDR[15:0]/PODR[31:16] @ 0x00, PCNTR2 PIDR @
// 0x04, PCNTR3 POSR[15:0]/PORR[31:16] @ 0x08.
const PORT0: u32 = 0x4008_0000;
const PCNTR1: u32 = 0x00;
const PCNTR2: u32 = 0x04;
const PCNTR3: u32 = 0x08;

// PFS @ 0x40080800: PmnPFS at 0x40*m + 4*n; PWPR (PMISC) at 0x40080D03.
const PFS: u32 = 0x4008_0800;
const PWPR: *mut u8 = 0x4008_0D03 as *mut u8;
const PWPR_B0WI: u8 = 1 << 7;
const PWPR_PFSWE: u8 = 1 << 6;
const PFS_PCR: u32 = 1 << 4;
const PFS_PMR: u32 = 1 << 16;
// FSP IOPORT_PERIPHERAL_SCI0_2_4_6_8 = PSEL 0b00100.
const PFS_PSEL_SCI0: u32 = 0x04 << 24;

// SCI0 @ 0x40118000: SMR +0, BRR +1, SCR +2, TDR +3, SSR +4, SEMR +7.
const SCI0: u32 = 0x4011_8000;
const SCI_SMR: *mut u8 = SCI0 as *mut u8;
const SCI_BRR: *mut u8 = (SCI0 + 0x01) as *mut u8;
const SCI_SCR: *mut u8 = (SCI0 + 0x02) as *mut u8;
const SCI_TDR: *mut u8 = (SCI0 + 0x03) as *mut u8;
const SCI_SSR: *const u8 = (SCI0 + 0x04) as *const u8;
const SCI_SEMR: *mut u8 = (SCI0 + 0x07) as *mut u8;
const SCR_TE: u8 = 1 << 5;
const SCR_RE: u8 = 1 << 4;
const SSR_TDRE: u8 = 1 << 7;

const MOCO_HZ: u32 = 8_000_000;
const BAUD: u32 = 9600;
const BRR: u32 = MOCO_HZ / (32 * BAUD) - 1;

#[derive(Clone, Copy)]
struct Led {
    name: &'static [u8],
    pin: u32,
}

const LEDS: [Led; 3] = [
    Led {
        name: b"blue",
        pin: 6,
    },
    Led {
        name: b"green",
        pin: 7,
    },
    Led {
        name: b"red",
        pin: 8,
    },
];

const S1_PIN: u32 = 5;
const S2_PIN: u32 = 4;

fn putc(byte: u8) {
    unsafe {
        while read_volatile(SCI_SSR) & SSR_TDRE == 0 {}
        write_volatile(SCI_TDR, byte);
    }
}

fn puts(text: &[u8]) {
    for byte in text {
        putc(*byte);
    }
}

fn put_hex16(value: u32) {
    for shift in (0..4).rev() {
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

fn pfs(port: u32, pin: u32) -> *mut u32 {
    (PFS + 0x40 * port + 4 * pin) as *mut u32
}

fn podr() -> u32 {
    unsafe { read_volatile((PORT0 + PCNTR1) as *const u32) >> 16 }
}

fn pidr() -> u32 {
    unsafe { read_volatile((PORT0 + PCNTR2) as *const u32) & 0xFFFF }
}

fn led_drive(led: Led, on: bool) {
    // PCNTR3: POSR[15:0] sets PODR, PORR[31:16] clears it.
    let bit = if on {
        1 << led.pin
    } else {
        1 << (led.pin + 16)
    };
    unsafe { write_volatile((PORT0 + PCNTR3) as *mut u32, bit) };
}

fn report(on: bool, led: Led) {
    let level = (podr() >> led.pin) & 1;
    puts(b"led=");
    puts(led.name);
    puts(if on { b" on pin=" } else { b" off pin=" });
    put_dec32(level);
    puts(b" podr=");
    put_hex16(podr());
    putc(b'\n');
}

fn switches() {
    let inputs = pidr();
    puts(b"s1=");
    put_dec32((inputs >> S1_PIN) & 1);
    puts(b" s2=");
    put_dec32((inputs >> S2_PIN) & 1);
}

#[inline(never)]
fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // All clocks /1 from the reset MOCO (SCKDIVCR is PRC0-protected).
        write_volatile(PRCR, PRCR_KEY | PRCR_PRC0);
        write_volatile(SCKDIVCR, 0);
        write_volatile(PRCR, PRCR_KEY);

        // Release SCI0 from module stop.
        write_volatile(MSTPCRB, read_volatile(MSTPCRB) & !MSTPB31_SCI0);

        // PmnPFS writes need PWPR.B0WI = 0, then PWPR.PFSWE = 1.
        write_volatile(PWPR, 0);
        write_volatile(PWPR, PWPR_PFSWE);
        // P410 RXD0 / P411 TXD0: peripheral mode, PSEL = SCI0.
        write_volatile(pfs(4, 10), PFS_PMR | PFS_PSEL_SCI0);
        write_volatile(pfs(4, 11), PFS_PMR | PFS_PSEL_SCI0);
        // S1/S2 inputs with the on-chip pull-up.
        write_volatile(pfs(0, S1_PIN), PFS_PCR);
        write_volatile(pfs(0, S2_PIN), PFS_PCR);
        write_volatile(PWPR, 0);
        write_volatile(PWPR, PWPR_B0WI);

        // LEDs as outputs, initially low.
        let pcntr1 = (PORT0 + PCNTR1) as *mut u32;
        let mut dir = read_volatile(pcntr1);
        for led in LEDS {
            dir |= 1 << led.pin;
            dir &= !(1 << (led.pin + 16));
        }
        write_volatile(pcntr1, dir);

        // SCI0 async 8N1: TE/RE off, SMR/SEMR defaults, BRR, then wait at
        // least one bit time (104 us at 9600) before enabling TE/RE.
        write_volatile(SCI_SCR, 0);
        write_volatile(SCI_SMR, 0);
        write_volatile(SCI_SEMR, 0);
        write_volatile(SCI_BRR, BRR as u8);
        spin(1_000);
        write_volatile(SCI_SCR, SCR_TE | SCR_RE);
    }

    puts(b"EK-RA6M5 SMOKE\n");
    puts(b"cpu=cortex-m33 uart=sci0\n");
    puts(b"clk src=moco pclka=");
    put_dec32(MOCO_HZ);
    puts(b" brr=");
    put_dec32(BRR);
    puts(b" baud=");
    put_dec32(BAUD);
    putc(b'\n');

    for led in LEDS {
        led_drive(led, true);
        report(true, led);
        led_drive(led, false);
        report(false, led);
    }

    // Leave LED1 lit so the heartbeat's PODR readback is non-zero.
    led_drive(LEDS[0], true);

    switches();
    putc(b'\n');

    let mut heartbeat: u32 = 0;
    loop {
        puts(b"hb=");
        put_dec32(heartbeat);
        puts(b" podr=");
        put_hex16(podr());
        putc(b' ');
        switches();
        putc(b'\n');
        spin(200_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
