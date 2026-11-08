//! S32K118 onboarding smoke: Cortex-M0+ MULS, LPUART0 polled 8N1 TX and
//! GPIO port D PSOR/PCOR/PTOR, on the Renode S32K118 map
//! (LPUART0 @ 0x4006A000, PTD data block @ 0x400FF0C0).
//!
//! PCC clock gating and the PORT pin-mux blocks are not modelled, so the
//! firmware does not touch PCC_LPUART0 or PORTD_PCR0; on silicon both must be
//! programmed first. Renode tags PCC_LPUART0 as a constant for the same reason.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RESULT: *mut u32 = 0x2000_0000 as *mut u32;
const PASS: *mut u32 = 0x2000_0004 as *mut u32;

const LPUART0: usize = 0x4006_A000;
const VERID: usize = 0x00;
const PARAM: usize = 0x04;
const BAUD: usize = 0x10;
const STAT: usize = 0x14;
const CTRL: usize = 0x18;
const DATA: usize = 0x1C;
const STAT_TDRE: u32 = 1 << 23;
const STAT_TC: u32 = 1 << 22;
const CTRL_TE: u32 = 1 << 19;

const PTD: usize = 0x400F_F0C0;
const PDOR: usize = 0x00;
const PSOR: usize = 0x04;
const PCOR: usize = 0x08;
const PTOR: usize = 0x0C;
const PDDR: usize = 0x14;

fn rd(base: usize, off: usize) -> u32 {
    unsafe { read_volatile((base + off) as *const u32) }
}

fn wr(base: usize, off: usize, v: u32) {
    unsafe { write_volatile((base + off) as *mut u32, v) }
}

fn putc(b: u8) {
    while rd(LPUART0, STAT) & STAT_TDRE == 0 {}
    wr(LPUART0, DATA, b as u32);
}

fn puts(s: &str) {
    for b in s.bytes() {
        putc(b);
    }
}

fn hex(v: u32, digits: u32) {
    for i in (0..digits).rev() {
        let n = ((v >> (i * 4)) & 0xF) as u8;
        putc(if n < 10 { b'0' + n } else { b'a' + n - 10 });
    }
}

#[entry]
fn main() -> ! {
    let product: u32;
    unsafe {
        core::arch::asm!(
            "muls {product}, {right}",
            product = inout(reg) 17u32 => product,
            right = in(reg) 19u32,
            options(nomem, nostack),
        );
        write_volatile(RESULT, product);
    }

    let mut pass = 0u32;

    // 115200-ish 8N1 from a 48 MHz FIRC functional clock: OSR = 15 (16x),
    // SBR = 26 -> 48e6 / (16 * 26) = 115384 Bd.
    wr(LPUART0, BAUD, (15 << 24) | 26);
    wr(LPUART0, CTRL, CTRL_TE);
    puts("S32K118 LPUART0 SMOKE\r\n");

    let verid = rd(LPUART0, VERID);
    let param = rd(LPUART0, PARAM);
    let baud = rd(LPUART0, BAUD);
    let ctrl = rd(LPUART0, CTRL);
    puts("lpuart0 verid=");
    hex(verid, 8);
    puts(" param=");
    hex(param, 8);
    puts(" baud=");
    hex(baud, 8);
    puts(" ctrl=");
    hex(ctrl, 8);
    puts("\r\n");
    if verid == 0x0401_0003 && param == 0x0202 {
        pass |= 1 << 0;
    }
    if baud == (15 << 24) | 26 && ctrl == CTRL_TE {
        pass |= 1 << 1;
    }

    puts("muls=");
    hex(product, 8);
    puts("\r\n");
    if product == 17 * 19 {
        pass |= 1 << 2;
    }

    // PTD0 output: set, clear, then toggle it and PTD3 together.
    wr(PTD, PDDR, 0x9);
    wr(PTD, PSOR, 0x1);
    let after_set = rd(PTD, PDOR);
    wr(PTD, PCOR, 0x1);
    let after_clear = rd(PTD, PDOR);
    wr(PTD, PTOR, 0x9);
    let after_toggle = rd(PTD, PDOR);
    puts("ptd pddr=");
    hex(rd(PTD, PDDR), 8);
    puts(" set=");
    hex(after_set, 8);
    puts(" clr=");
    hex(after_clear, 8);
    puts(" tog=");
    hex(after_toggle, 8);
    puts("\r\n");
    if after_set == 0x1 && after_clear == 0 && after_toggle == 0x9 {
        pass |= 1 << 3;
    }

    // Wait for the last stop bit before reporting.
    while rd(LPUART0, STAT) & STAT_TC == 0 {}
    puts("pass=");
    hex(pass, 2);
    puts("\r\n");
    unsafe { write_volatile(PASS, 0x5118_0000 | pass) };

    let mut n = 0u32;
    loop {
        // Heartbeat on PTD0 so the GPIO edge assertion sees the pin move.
        n = n.wrapping_add(1);
        if n % 16384 == 0 {
            wr(PTD, PTOR, 0x1);
        }
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
}
