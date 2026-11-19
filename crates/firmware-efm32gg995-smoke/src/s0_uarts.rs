//! Secondary (U)SART/UART probe shared by the EFM32/EZR32 Series-0 smokes
//! (`#[path]`-included from `firmware-efm32gg995-smoke/src/s0_uarts.rs`).
//!
//! For each instance: STATUS reads TXBL (bit 6) out of reset, CMD.TXEN is
//! set, and one line "<name> status=<hex> ok" is sent on that instance.
#![allow(dead_code)]

use core::ptr::{read_volatile, write_volatile};

// EFM32 (U)SART register map: CMD@0x0C, STATUS@0x10, TXDATA@0x34.
const U_CMD: u32 = 0x0C;
const U_STATUS: u32 = 0x10;
const U_TXDATA: u32 = 0x34;
const CMD_TXEN: u32 = 1 << 0;
const STATUS_TXBL: u32 = 1 << 6;

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn putc(base: u32, byte: u8) {
    let mut remaining = 100_000u32;
    while rd(base + U_STATUS) & STATUS_TXBL == 0 && remaining > 0 {
        remaining -= 1;
    }
    wr(base + U_TXDATA, byte as u32);
}

fn puts(base: u32, s: &str) {
    for b in s.bytes() {
        putc(base, b);
    }
}

pub fn announce(uarts: &[(&str, u32)]) {
    for &(name, base) in uarts {
        let status = rd(base + U_STATUS);
        wr(base + U_CMD, CMD_TXEN);
        puts(base, name);
        puts(base, " status=");
        for i in (0..2).rev() {
            let nib = ((status >> (i * 4)) & 0xF) as u8;
            putc(
                base,
                if nib < 10 {
                    b'0' + nib
                } else {
                    b'a' + nib - 10
                },
            );
        }
        puts(
            base,
            if status & STATUS_TXBL != 0 {
                " ok\r\n"
            } else {
                " FAIL\r\n"
            },
        );
    }
}
