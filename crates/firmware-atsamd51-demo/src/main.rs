// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// SAMD51J19A smoke firmware for Adafruit Metro M4: configure Serial1 on
// SERCOM3 PAD0/PAD1 (PA22/PA23), print a marker, and drive D13 (PA16).
// Bare-register driver — no HAL.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// MCLK APBBMASK @ 0x40000800 + 0x18; SERCOM3 is bit 10 (DS60001507).
const MCLK_APBBMASK: *mut u32 = 0x4000_0818 as *mut u32;
// GCLK PCHCTRL[24] @ 0x40001C00 + 0x80 + 4*24 (SERCOM3_CORE).
const GCLK_GENCTRL0: *mut u32 = 0x4000_1C20 as *mut u32;
const GCLK_PCHCTRL24: *mut u32 = 0x4000_1CE0 as *mut u32;
// PORTA PMUX[11] pairs PA22/PA23; PINCFG enables peripheral muxing.
const PORTA_PMUX11: *mut u8 = 0x4100_803B as *mut u8;
const PORTA_PINCFG22: *mut u8 = 0x4100_8056 as *mut u8;
const PORTA_PINCFG23: *mut u8 = 0x4100_8057 as *mut u8;
// PORTA DIRSET / OUTTGL (sam_port profile @ 0x41008000).
const PORTA_DIRSET: *mut u32 = 0x4100_8008 as *mut u32;
const PORTA_OUTTGL: *mut u32 = 0x4100_801C as *mut u32;

// SERCOM3 USART: CTRLA/B, BAUD, INTFLAG, SYNCBUSY, DATA.
const SERCOM3_CTRLA: *mut u32 = 0x4101_4000 as *mut u32;
const SERCOM3_CTRLB: *mut u32 = 0x4101_4004 as *mut u32;
const SERCOM3_BAUD: *mut u16 = 0x4101_400C as *mut u16;
const SERCOM3_INTFLAG: *const u8 = 0x4101_4018 as *const u8;
const SERCOM3_SYNCBUSY: *const u32 = 0x4101_401C as *const u32;
const SERCOM3_DATA: *mut u16 = 0x4101_4028 as *mut u16;

const LED: u32 = 1 << 16; // PA16 — D13 / LED_BUILTIN
// PCHCTRL: GEN=0 | CHEN=1 → bit 6.
const GCLK_PCHCTRL_CHEN: u32 = 1 << 6;
const CTRLA_MODE_USART_INT_CLK: u32 = 1 << 2;
const CTRLA_RXPO_PAD1: u32 = 1 << 20;
const CTRLA_DORD_LSB: u32 = 1 << 30;
const CTRLA_ENABLE: u32 = 1 << 1;
const CTRLB_TXEN: u32 = 1 << 16;
const CTRLB_RXEN: u32 = 1 << 17;
const INTFLAG_DRE: u8 = 1;
// Arithmetic 16x sampling: 65536 * (1 - 16*115200/48_000_000).
const BAUD_115200_AT_48MHZ: u16 = 63_020;

fn sercom_sync() {
    for _ in 0..10_000 {
        if unsafe { read_volatile(SERCOM3_SYNCBUSY) } == 0 {
            return;
        }
    }
}

fn putc(byte: u8) {
    for _ in 0..10_000 {
        if unsafe { read_volatile(SERCOM3_INTFLAG) } & INTFLAG_DRE != 0 {
            break;
        }
    }
    unsafe { write_volatile(SERCOM3_DATA, u16::from(byte)) };
}

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(MCLK_APBBMASK, 1 << 10);
        // Generator 0 uses the DFLL48M (SRC=6) and must be enabled before
        // SERCOM3's PCHCTRL channel can resolve a baud clock.
        write_volatile(GCLK_GENCTRL0, 6 | (1 << 8));
        write_volatile(GCLK_PCHCTRL24, GCLK_PCHCTRL_CHEN);

        // Function C on PA22/PA23 selects SERCOM3 PAD0/PAD1 (Serial1 D1/D0).
        write_volatile(PORTA_PMUX11, 0x22);
        write_volatile(PORTA_PINCFG22, 1); // PMUXEN
        write_volatile(PORTA_PINCFG23, 1); // PMUXEN

        // Configure SERCOM3 as an internally-clocked 8N1 USART before enabling
        // it. GCLK0 is the reset-enabled DFLL48M channel in this profile.
        write_volatile(
            SERCOM3_CTRLA,
            CTRLA_MODE_USART_INT_CLK | CTRLA_RXPO_PAD1 | CTRLA_DORD_LSB,
        );
        write_volatile(SERCOM3_CTRLB, CTRLB_TXEN | CTRLB_RXEN);
        sercom_sync();
        write_volatile(SERCOM3_BAUD, BAUD_115200_AT_48MHZ);
        let ctrla = read_volatile(SERCOM3_CTRLA) | CTRLA_ENABLE;
        write_volatile(SERCOM3_CTRLA, ctrla);
        sercom_sync();

        write_volatile(PORTA_DIRSET, LED);
    }

    for b in b"METRO_M4_OK\n" {
        putc(*b);
    }

    unsafe {
        write_volatile(PORTA_OUTTGL, LED);
    }
    loop {}
}
