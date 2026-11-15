// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
//! FocalTech FT9001 (Cortex-M4F) onboarding smoke: USART2 polled TX/RX,
//! the CPM clock-status handshake, the TRNG word path and a software reset
//! through the reset controller, in bare registers.
//!
//! The register block is the one Renode's `UART.FT9001_USART` model exposes
//! (renode-infrastructure `FT9001_USART.cs`), at USART2 0x4001_4000:
//!   BDL 0x00, BDH 0x01, CR2 0x02 (RE bit 2, TE bit 3), CR1 0x03,
//!   DRL 0x06, BRDF 0x0A, FCR 0x0E, FSR 0x11 (REMPTY 0, TEMPTY 2, TFULL 3,
//!   RFTS 5, FTC 6), FCR2 0x13 (RXFCLR bit 0, write-only).
//! The BDL/BDH/BRDF values written here are read-back probes only; no baud
//! rate is derived from them.
//!
//! The smoke script injects "AB" at start and "Z" later. The run proves:
//! a byte written before TE is dropped, control registers read back, FSR
//! tracks the RX FIFO, RXFCLR empties it, a later byte is received, and
//! clearing TE drops TX again.
//!
//! CPM (0x4000_4000), reset (0x4000_2000) and TRNG (0x4003_B000) follow the
//! FocalTech HAL (`cpm_reg.h`/`cpm_drv.c`, `reset_reg.h`, `trng_reg.h`/
//! `trng_drv.c`) and Zephyr `hwinfo_ft.c` (RSR bit 3 POR, bit 5 software).
//! Boot 1 (RSR = POR) runs every check and then sets RCR.SOFTRST. Boot 2
//! (RSR = software) checks that RSR kept the cause and that CPM and TRNG
//! came back at their reset values.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const USART2: u32 = 0x4001_4000;
const BDL: u32 = USART2;
const BDH: u32 = USART2 + 0x01;
const CR2: u32 = USART2 + 0x02;
const CR1: u32 = USART2 + 0x03;
const DRL: u32 = USART2 + 0x06;
const BRDF: u32 = USART2 + 0x0A;
const FCR: u32 = USART2 + 0x0E;
const FSR: u32 = USART2 + 0x11;
const FCR2: u32 = USART2 + 0x13;

const RST: u32 = 0x4000_2000;
const RST_RCR: u32 = RST;
const RST_RSR: u32 = RST + 0x07;
const RCR_SOFTRST: u32 = 1 << 31;
const RSR_POR: u8 = 1 << 3;
const RSR_SOFTWARE: u8 = 1 << 5;

const CPM: u32 = 0x4000_4000;
const CPM_CDER: u32 = CPM + 0x1C;
const CPM_OCSR: u32 = CPM + 0x20;
const CPM_CSWCFGR: u32 = CPM + 0x24;
const OCSR_OSC320M_CLK_EN: u32 = 1 << 3;
const OCSR_OSC320M_STABLE: u32 = 1 << 11;
const CSWCFGR_OSC8M_SELECT: u32 = 1 << 8;
const CSWCFGR_OSC320M_SELECT: u32 = 1 << 9;
const CSWCFGR_SOC_CLK_SOURCE_MASK: u32 = 0xFFFF_FFFC;

const TRNG: u32 = 0x4003_B000;
const TRNG_CTRL: u32 = TRNG;
const TRNG_DATA: u32 = TRNG + 0x04;
const TRNG_EN: u32 = 1 << 8;
const TRNG_CLR_IT: u32 = 1 << 10;
const TRNG_IT: u32 = 1 << 11;

const CR2_RE: u8 = 1 << 2;
const CR2_TE: u8 = 1 << 3;
const FCR_TFEN: u8 = 1 << 6;
const FCR_RFEN: u8 = 1 << 7;
const FCR2_RXFCLR: u8 = 1 << 0;
const FSR_TFULL: u8 = 1 << 3;
const FSR_RFTS: u8 = 1 << 5;

fn rd(addr: u32) -> u8 {
    unsafe { read_volatile(addr as *const u8) }
}

fn wr(addr: u32, value: u8) {
    unsafe { write_volatile(addr as *mut u8, value) }
}

fn rd32(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr32(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn putc(c: u8) {
    while rd(FSR) & FSR_TFULL != 0 {}
    wr(DRL, c);
}

fn puts(s: &str) {
    for b in s.bytes() {
        putc(b);
    }
}

fn hex8(v: u8) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    putc(DIGITS[(v >> 4) as usize]);
    putc(DIGITS[(v & 0xF) as usize]);
}

fn hex32(v: u32) {
    for shift in [24, 16, 8, 0] {
        hex8((v >> shift) as u8);
    }
}

fn field32(name: &str, v: u32) {
    puts(name);
    putc(b'=');
    hex32(v);
}

fn field(name: &str, v: u8) {
    puts(name);
    putc(b'=');
    hex8(v);
}

/// HAL `CPM_OSC320M_Init`-style switch: enable OSC320M, wait STABLE, select
/// source 1 and wait for the select status. Source 0 is visited first.
fn cpm_checks() {
    field32("cpm ocsr", rd32(CPM_OCSR));
    field32(" csw", rd32(CPM_CSWCFGR));
    wr32(CPM_CSWCFGR, rd32(CPM_CSWCFGR) & CSWCFGR_SOC_CLK_SOURCE_MASK);
    let osc8m = rd32(CPM_CSWCFGR) & CSWCFGR_OSC8M_SELECT != 0;
    wr32(CPM_OCSR, rd32(CPM_OCSR) | OCSR_OSC320M_CLK_EN);
    while rd32(CPM_OCSR) & OCSR_OSC320M_STABLE == 0 {}
    let data = (rd32(CPM_CSWCFGR) & CSWCFGR_SOC_CLK_SOURCE_MASK) | 1;
    wr32(CPM_CSWCFGR, rd32(CPM_CSWCFGR) | data);
    while rd32(CPM_CSWCFGR) & CSWCFGR_OSC320M_SELECT == 0 {}
    wr32(CPM_CDER, 1);
    puts(if osc8m { " src0=osc8m" } else { " src0=?" });
    field32(" ocsr", rd32(CPM_OCSR));
    field32(" csw", rd32(CPM_CSWCFGR));
    field32(" cder", rd32(CPM_CDER));
    puts("\r\n");
}

/// HAL `HAL_TRNG_Init` + `DRV_GetRandomWord` x3.
fn trng_checks() {
    wr32(TRNG_CTRL, rd32(TRNG_CTRL) & !TRNG_EN);
    wr32(TRNG_CTRL, (rd32(TRNG_CTRL) & 0xFFFF_FF00) | 59);
    wr32(TRNG_CTRL, (rd32(TRNG_CTRL) & 0xFF0F_FFFF) | (0x0F << 20));
    wr32(TRNG_CTRL, (rd32(TRNG_CTRL) & 0xFFF0_FFFF) | (0x0F << 16));
    wr32(TRNG_CTRL, rd32(TRNG_CTRL) | TRNG_CLR_IT);
    wr32(TRNG_CTRL, rd32(TRNG_CTRL) | TRNG_EN);
    field32("trng ctrl", rd32(TRNG_CTRL));
    let mut words = [0u32; 3];
    for w in words.iter_mut() {
        while rd32(TRNG_CTRL) & TRNG_IT == 0 {}
        *w = rd32(TRNG_DATA);
        wr32(TRNG_CTRL, rd32(TRNG_CTRL) | TRNG_CLR_IT);
    }
    for w in words {
        field32(" w", w);
    }
    let distinct = words[0] != words[1] && words[1] != words[2] && words[0] != words[2];
    puts(if distinct {
        " distinct=1"
    } else {
        " distinct=0"
    });
    puts("\r\n");
}

/// Boot 2: the software reset happened; RSR kept the cause.
fn after_soft_reset(rsr: u8) -> ! {
    wr(CR2, CR2_TE | CR2_RE);
    field("boot2 rsr", rsr);
    field32(" rcr", rd32(RST_RCR));
    field32(" csw", rd32(CPM_CSWCFGR));
    field32(" cder", rd32(CPM_CDER));
    field32(" trng", rd32(TRNG_CTRL));
    puts("\r\n");
    trng_checks();
    puts("FT9001 SMOKE DONE\r\n");
    loop {
        core::hint::spin_loop();
    }
}

#[entry]
fn main() -> ! {
    let rsr = rd(RST_RSR);
    if rsr & RSR_SOFTWARE != 0 {
        after_soft_reset(rsr);
    }

    // TE is clear out of reset: this byte must not reach the line.
    wr(DRL, b'X');

    wr(CR2, CR2_TE | CR2_RE);
    puts("FT9001 SMOKE\r\n");
    puts("cpu=cortex-m4f uart=usart2\r\n");

    wr(BDL, 0x56);
    wr(BDH, 0x01);
    wr(BRDF, 0x1A);
    wr(CR1, 0x00);
    wr(FCR, FCR_TFEN | FCR_RFEN);
    field("cr2", rd(CR2));
    field(" cr1", rd(CR1));
    field(" bdh", rd(BDH));
    field(" bdl", rd(BDL));
    field(" brdf", rd(BRDF));
    field(" fcr", rd(FCR));
    puts("\r\n");

    // "AB" was injected before the first instruction.
    field("rx fsr", rd(FSR));
    let first = rd(DRL);
    puts(" drl=");
    putc(first);
    field(" fsr", rd(FSR));
    puts("\r\n");

    wr(FCR2, FCR2_RXFCLR);
    field("rxfclr fsr", rd(FSR));
    field(" drl", rd(DRL));
    field(" fcr2", rd(FCR2));
    puts("\r\n");

    // The second injection lands later in the run.
    while rd(FSR) & FSR_RFTS == 0 {}
    let late = rd(DRL);
    puts("late drl=");
    putc(late);
    field(" fsr", rd(FSR));
    puts("\r\n");

    puts("te=off");
    wr(CR2, CR2_RE);
    wr(DRL, b'Y');
    wr(CR2, CR2_TE | CR2_RE);
    puts("|te=on\r\n");

    cpm_checks();
    trng_checks();

    field("rsr", rsr);
    puts(if rsr & RSR_POR != 0 {
        " por=1"
    } else {
        " por=0"
    });
    puts(" softrst\r\n");
    wr32(RST_RCR, rd32(RST_RCR) | RCR_SOFTRST);
    for _ in 0..1000 {
        core::hint::spin_loop();
    }
    puts("SOFTRST IGNORED\r\n");
    loop {
        core::hint::spin_loop();
    }
}
