// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! EK-RA8M1 (R7FA8M1AH, Cortex-M85) smoke firmware: a running light over the
//! three user LEDs plus a heartbeat line on the SCI9 console, in bare
//! registers.
//!
//! Board facts (Zephyr `ek_ra8m1` ek_ra8m1.dts / ek_ra8m1-pinctrl.dtsi):
//!   LED1 (blue) P600, LED2 (green) P414, LED3 (red) P107 — active high
//!   S1 P009, S2 P008 — active low, pulled up
//!   Console SCI9 PA14 (TXD9) / PA15 (RXD9), J-Link OB virtual COM port
//!
//! The RA8M1 resets onto MOCO (8 MHz) with every SCKDIVCR divider at /1 (FSP
//! ra8m1 bsp_feature.h `BSP_FEATURE_CGC_STARTUP_SCKDIVCR` = 0). The SCI_B
//! channels run from their own SCICLK, so the firmware switches SCICLK to
//! MOCO /1 with the FSP `bsp_peripheral_clock_set` CKSREQ/CKSRDY handshake
//! under PRCR.PRC0, takes SCI9 out of module stop (MSTPCRB.MSTPB22), routes
//! PA14/PA15 to SCI9 and enables the S1/S2 pull-ups through PmnPFS (behind
//! PWPRS, the secure write-protect register, since this image runs secure),
//! programs SCI9 the way FSP `r_sci_b_uart.c` does and waits for
//! CESR.TIST/RIST, walks the LEDs and prints the PODR readback, then reads
//! the switches and heartbeats.
//!
//! Baud: SCICLK = MOCO 8 MHz / 1. Async, CCR2.CKS = 0, ABCS = BGDM = 0:
//! BRR = SCICLK / (32 * 9600) - 1 = 25.04 -> 25 (9615 baud, +0.16 %).
//!
//! Built for `thumbv8m.main-none-eabi`: Armv8-M Mainline code with no
//! Helium/MVE or low-overhead-branch instructions.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// R_SYSTEM @ 0x4001E000 (R7FA8M1AH.h): SCICKDIVCR @ 0x54, SCICKCR @ 0x55,
// PRCR (secure) @ 0x3FA.
const SYSTEM: u32 = 0x4001_E000;
const SCICKDIVCR: *mut u8 = (SYSTEM + 0x54) as *mut u8;
const SCICKCR: *mut u8 = (SYSTEM + 0x55) as *mut u8;
const SCICKCR_CKSREQ: u8 = 1 << 6;
const SCICKCR_CKSRDY: u8 = 1 << 7;
// FSP BSP_CLOCKS_SOURCE_CLOCK_MOCO.
const SCICKSEL_MOCO: u8 = 1;
const PRCR: *mut u16 = (SYSTEM + 0x3FA) as *mut u16;
const PRCR_KEY: u16 = 0xA500;
const PRCR_PRC0: u16 = 1 << 0;

// R_MSTP @ 0x40203000: MSTPCRB @ 0x04; SCIn is MSTPB(31 - n) (FSP
// bsp_module_stop.h BSP_MSTP_BIT_FSP_IP_SCI). 0 = module running.
const MSTPCRB: *mut u32 = 0x4020_3004 as *mut u32;
const MSTPB22_SCI9: u32 = 1 << (31 - 9);

// PORTn @ 0x40400000 + 0x20*n: PCNTR1 PDR[15:0]/PODR[31:16] @ 0x00, PCNTR2
// PIDR @ 0x04, PCNTR3 POSR[15:0]/PORR[31:16] @ 0x08.
const PORT_BASE: u32 = 0x4040_0000;
const PCNTR1: u32 = 0x00;
const PCNTR2: u32 = 0x04;
const PCNTR3: u32 = 0x08;

// PFS @ 0x40400800: PmnPFS at 0x40*m + 4*n. PMISC @ 0x40400D00: PWPR at
// +0x0C, PWPRS (secure) at +0x14.
const PFS: u32 = 0x4040_0800;
const PWPRS: *mut u8 = 0x4040_0D14 as *mut u8;
const PWPR_B0WI: u8 = 1 << 7;
const PWPR_PFSWE: u8 = 1 << 6;
const PFS_PCR: u32 = 1 << 4;
const PFS_PMR: u32 = 1 << 16;
// FSP IOPORT_PERIPHERAL_SCI1_3_5_7_9 = PSEL 0b00101.
const PFS_PSEL_SCI9: u32 = 0x05 << 24;
const PORT_A: u32 = 10;

// SCI9 (SCI_B) @ 0x40358900 (R_SCI_B0_Type): TDR +0x04, CCR0-3 +0x08..+0x14,
// CESR +0x1C, CSR +0x48, CFCLR +0x68.
const SCI9: u32 = 0x4035_8900;
const SCI_TDR: *mut u32 = (SCI9 + 0x04) as *mut u32;
const SCI_CCR0: *mut u32 = (SCI9 + 0x08) as *mut u32;
const SCI_CCR1: *mut u32 = (SCI9 + 0x0C) as *mut u32;
const SCI_CCR2: *mut u32 = (SCI9 + 0x10) as *mut u32;
const SCI_CCR3: *mut u32 = (SCI9 + 0x14) as *mut u32;
const SCI_CESR: *const u32 = (SCI9 + 0x1C) as *const u32;
const SCI_CSR: *const u32 = (SCI9 + 0x48) as *const u32;
const SCI_CFCLR: *mut u32 = (SCI9 + 0x68) as *mut u32;
const CCR0_RE: u32 = 1 << 0;
const CCR0_TE: u32 = 1 << 4;
// CCR1.SPB2DT: TXD idles high while TE is off (FSP default).
const CCR1_SPB2DT: u32 = 1 << 4;
// CCR3: CHR = 0b10 (8 bits, FSP UART_DATA_BITS_8), LSBF, MOD = 0 (async).
const CCR3_CHR_8BIT: u32 = 2 << 8;
const CCR3_LSBF: u32 = 1 << 12;
const CESR_TIST: u32 = 1 << 4;
const CESR_RIST: u32 = 1 << 0;
const CSR_TDRE: u32 = 1 << 29;
// FSP SCI_B_UART_CFCLR_CLEAR_ALL_MASK.
const CFCLR_ALL: u32 = 0x9D07_0010;

const MOCO_HZ: u32 = 8_000_000;
const BAUD: u32 = 9600;
const BRR: u32 = MOCO_HZ / (32 * BAUD) - 1;

#[derive(Clone, Copy)]
struct Led {
    name: &'static [u8],
    port: u32,
    pin: u32,
}

const LEDS: [Led; 3] = [
    Led {
        name: b"blue",
        port: 6,
        pin: 0,
    },
    Led {
        name: b"green",
        port: 4,
        pin: 14,
    },
    Led {
        name: b"red",
        port: 1,
        pin: 7,
    },
];

const S1_PIN: u32 = 9;
const S2_PIN: u32 = 8;

fn putc(byte: u8) {
    unsafe {
        while read_volatile(SCI_CSR) & CSR_TDRE == 0 {}
        write_volatile(SCI_TDR, u32::from(byte));
    }
}

fn puts(text: &[u8]) {
    for byte in text {
        putc(*byte);
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

fn port(n: u32) -> u32 {
    PORT_BASE + 0x20 * n
}

fn pfs(port: u32, pin: u32) -> *mut u32 {
    (PFS + 0x40 * port + 4 * pin) as *mut u32
}

fn podr(n: u32) -> u32 {
    unsafe { read_volatile((port(n) + PCNTR1) as *const u32) >> 16 }
}

fn pidr(n: u32) -> u32 {
    unsafe { read_volatile((port(n) + PCNTR2) as *const u32) & 0xFFFF }
}

fn led_drive(led: Led, on: bool) {
    // PCNTR3: POSR[15:0] sets PODR, PORR[31:16] clears it.
    let bit = if on {
        1 << led.pin
    } else {
        1 << (led.pin + 16)
    };
    unsafe { write_volatile((port(led.port) + PCNTR3) as *mut u32, bit) };
}

fn report(on: bool, led: Led) {
    let level = (podr(led.port) >> led.pin) & 1;
    puts(b"led=");
    puts(led.name);
    puts(if on { b" on pin=" } else { b" off pin=" });
    put_dec32(level);
    puts(b" port=");
    put_dec32(led.port);
    puts(b" podr=");
    put_hex(podr(led.port), 4);
    putc(b'\n');
}

fn switches() {
    let inputs = pidr(0);
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
    let sciclk;
    unsafe {
        // SCICLK = MOCO / 1 (SCICKCR/SCICKDIVCR are PRC0-protected).
        write_volatile(PRCR, PRCR_KEY | PRCR_PRC0);
        write_volatile(SCICKCR, read_volatile(SCICKCR) | SCICKCR_CKSREQ);
        while read_volatile(SCICKCR) & SCICKCR_CKSRDY == 0 {}
        write_volatile(SCICKDIVCR, 0);
        // FSP's read-modify-write writes CKSRDY back as 1. CKSRDY is
        // read-only, so the write has no effect on it.
        write_volatile(SCICKCR, SCICKSEL_MOCO | SCICKCR_CKSREQ | SCICKCR_CKSRDY);
        write_volatile(SCICKCR, read_volatile(SCICKCR) & !SCICKCR_CKSREQ);
        while read_volatile(SCICKCR) & SCICKCR_CKSRDY != 0 {}
        sciclk = read_volatile(SCICKCR);
        write_volatile(PRCR, PRCR_KEY);

        // Release SCI9 from module stop.
        write_volatile(MSTPCRB, read_volatile(MSTPCRB) & !MSTPB22_SCI9);

        // PmnPFS writes need PWPRS.B0WI = 0, then PWPRS.PFSWE = 1.
        write_volatile(PWPRS, 0);
        write_volatile(PWPRS, PWPR_PFSWE);
        // PA14 TXD9 / PA15 RXD9: peripheral mode, PSEL = SCI9.
        write_volatile(pfs(PORT_A, 14), PFS_PMR | PFS_PSEL_SCI9);
        write_volatile(pfs(PORT_A, 15), PFS_PMR | PFS_PSEL_SCI9);
        // S1/S2 inputs with the on-chip pull-up.
        write_volatile(pfs(0, S1_PIN), PFS_PCR);
        write_volatile(pfs(0, S2_PIN), PFS_PCR);
        write_volatile(PWPRS, 0);
        write_volatile(PWPRS, PWPR_B0WI);

        // LEDs as outputs, initially low.
        for led in LEDS {
            let pcntr1 = (port(led.port) + PCNTR1) as *mut u32;
            let mut v = read_volatile(pcntr1);
            v |= 1 << led.pin;
            v &= !(1 << (led.pin + 16));
            write_volatile(pcntr1, v);
        }

        // SCI9 async 8N1, in the r_sci_b_uart.c R_SCI_B_UART_Open order:
        // TE/RE off, clear the flags, CCR1-3, then TE|RE and wait for
        // CESR.TIST/RIST before the first TDR write.
        write_volatile(SCI_CCR0, 0);
        while read_volatile(SCI_CESR) & (CESR_TIST | CESR_RIST) != 0 {}
        write_volatile(SCI_CFCLR, CFCLR_ALL);
        write_volatile(SCI_CCR1, CCR1_SPB2DT);
        write_volatile(SCI_CCR2, BRR << 8);
        write_volatile(SCI_CCR3, CCR3_CHR_8BIT | CCR3_LSBF);
        spin(1_000);
        write_volatile(SCI_CCR0, CCR0_TE | CCR0_RE);
        while read_volatile(SCI_CESR) & (CESR_TIST | CESR_RIST) != (CESR_TIST | CESR_RIST) {}
    }

    puts(b"EK-RA8M1 SMOKE\n");
    puts(b"cpu=cortex-m85 uart=sci9 (sci_b)\n");
    puts(b"clk src=moco sciclk=");
    put_dec32(MOCO_HZ);
    puts(b" scickcr=");
    put_hex(u32::from(sciclk), 2);
    puts(b" brr=");
    put_dec32((unsafe { read_volatile(SCI_CCR2) } >> 8) & 0xFF);
    puts(b" baud=");
    put_dec32(BAUD);
    putc(b'\n');
    puts(b"ccr3=");
    put_hex(unsafe { read_volatile(SCI_CCR3) }, 8);
    puts(b" cesr=");
    put_hex(unsafe { read_volatile(SCI_CESR) }, 2);
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
        puts(b" podr6=");
        put_hex(podr(6), 4);
        putc(b' ');
        switches();
        putc(b'\n');
        spin(200_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
