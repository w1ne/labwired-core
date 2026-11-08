//! EFM32 Series-1 GPIO checks, shared by the JG1/JG12/PG1/PG12 smokes
//! (`#[path]`-included).
//!
//! Checks the Series-1 GPIO ports (`efm32s1_gpioport`): GPIO_P_TypeDef at a
//! 0x30 stride from 0x4000A000 (Gecko SDK `efm32pg1b_gpio_p.h`). Unlike
//! Series 0 there are no DOUTSET/DOUTCLR words — 0x10/0x14 are reserved, so
//! this firmware sets and clears through DOUT and toggles through DOUTTGL —
//! and OVTDIS sits at 0x28. USART0 (0x40010000) is the console.
#![allow(dead_code)]

use core::ptr::{read_volatile, write_volatile};

// EFM32 USART register map: CMD@0x0C, STATUS@0x10, TXDATA@0x34.
const USART0: u32 = 0x4001_0000;
const U_CMD: u32 = 0x0C;
const U_STATUS: u32 = 0x10;
const U_TXDATA: u32 = 0x34;
const CMD_TXEN: u32 = 1 << 0;
const STATUS_TXBL: u32 = 1 << 6;

// GPIO_P_TypeDef (efm32pg1b_gpio_p.h), ports at a 0x30 stride.
const GPIO: u32 = 0x4000_A000;
const PORT_A: u32 = 0;
const P_CTRL: u32 = 0x00;
const P_MODEL: u32 = 0x04;
const P_MODEH: u32 = 0x08;
const P_DOUT: u32 = 0x0C;
const P_RESERVED0: u32 = 0x10;
const P_RESERVED1: u32 = 0x14;
const P_DOUTTGL: u32 = 0x18;
const P_DIN: u32 = 0x1C;
const P_PINLOCKN: u32 = 0x20;
const P_OVTDIS: u32 = 0x28;
/// _GPIO_P_CTRL_RESETVALUE (efm32pg1b_gpio.h).
const CTRL_RESET: u32 = 0x0050_0050;

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn port(n: u32, reg: u32) -> u32 {
    GPIO + n * 0x30 + reg
}

fn putc(byte: u8) {
    let mut remaining = 100_000u32;
    while rd(USART0 + U_STATUS) & STATUS_TXBL == 0 && remaining > 0 {
        remaining -= 1;
    }
    wr(USART0 + U_TXDATA, byte as u32);
}

fn puts(s: &str) {
    for b in s.bytes() {
        putc(b);
    }
}

fn hex(value: u32, digits: u32) {
    for i in (0..digits).rev() {
        let nib = ((value >> (i * 4)) & 0xF) as u8;
        putc(if nib < 10 {
            b'0' + nib
        } else {
            b'a' + nib - 10
        });
    }
}

/// "pf4": lower-case port letter and decimal pin number.
fn pin_name(p: u32, pin: u32) {
    putc(b'p');
    putc(b'a' + p as u8);
    if pin >= 10 {
        putc(b'1');
    }
    putc(b'0' + (pin % 10) as u8);
}

fn flag(name: &str, value: bool) {
    puts(name);
    putc(if value { b'1' } else { b'0' });
    putc(b' ');
}

fn verdict(ok: bool) -> bool {
    puts(if ok { "ok\r\n" } else { "FAIL\r\n" });
    ok
}

fn bit(addr: u32, n: u32) -> bool {
    (rd(addr) >> n) & 1 != 0
}

fn set_mode(p: u32, pin: u32, mode: u32) {
    let reg = port(p, if pin < 8 { P_MODEL } else { P_MODEH });
    let shift = (pin % 8) * 4;
    wr(reg, (rd(reg) & !(0xF << shift)) | (mode << shift));
}

fn modify_dout(p: u32, set: u32, clear: u32) {
    wr(port(p, P_DOUT), (rd(port(p, P_DOUT)) & !clear) | set);
}

/// LED0/LED1 push-pull: set via DOUT, toggle via DOUTTGL, clear via DOUT;
/// DOUTTGL is write-only.
fn check_pushpull(board: &Board) -> bool {
    let (p, pin) = board.led;
    let lo = 1 << pin;
    let hi = 1 << (pin + 1);
    let both = lo | hi;
    set_mode(p, pin, 4);
    set_mode(p, pin + 1, 4);
    modify_dout(p, lo, hi);
    let set = rd(port(p, P_DIN)) & both == lo;
    wr(port(p, P_DOUTTGL), both);
    let tgl = rd(port(p, P_DOUT)) & both == hi && rd(port(p, P_DIN)) & both == hi;
    modify_dout(p, 0, hi);
    let clr = rd(port(p, P_DIN)) & both == 0;
    let wo = rd(port(p, P_DOUTTGL)) == 0;
    puts("gpio ");
    pin_name(p, pin);
    putc(b'/');
    pin_name(p, pin + 1);
    puts(" pushpull ");
    flag("set=", set);
    flag("tgl=", tgl);
    flag("clr=", clr);
    flag("wo=", wo);
    verdict(set && tgl && clr && wo)
}

/// 0x10/0x14 are RESERVED0 on Series 1: a Series-0-style DOUTSET/DOUTCLR
/// write must not move DOUT.
fn check_reserved(board: &Board) -> bool {
    let (p, pin) = board.led;
    let lo = 1 << pin;
    modify_dout(p, 0, lo);
    wr(port(p, P_RESERVED0), lo);
    let no_set = rd(port(p, P_DOUT)) & lo == 0;
    modify_dout(p, lo, 0);
    wr(port(p, P_RESERVED1), lo);
    let no_clr = rd(port(p, P_DOUT)) & lo == lo;
    modify_dout(p, 0, lo);
    let read0 = rd(port(p, P_RESERVED0)) == 0 && rd(port(p, P_RESERVED1)) == 0;
    puts("gpio reserved 0x10/0x14 ");
    flag("no_set=", no_set);
    flag("no_clr=", no_clr);
    flag("read0=", read0);
    verdict(no_set && no_clr && read0)
}

fn check_disabled_and_pull() -> bool {
    wr(port(PORT_A, P_MODEL), 0x0);
    modify_dout(PORT_A, 1, 0);
    let disabled = !bit(port(PORT_A, P_DIN), 0);
    wr(port(PORT_A, P_MODEL), 0x2);
    let up = bit(port(PORT_A, P_DIN), 0);
    modify_dout(PORT_A, 0, 1);
    let down = !bit(port(PORT_A, P_DIN), 0);
    puts("gpio pa0 ");
    flag("disabled_din0=", disabled);
    flag("pullup=", up);
    flag("pulldown=", down);
    verdict(disabled && up && down)
}

fn check_wired() -> bool {
    // PA1 WIREDANDPULLUP (0xA), PA2 WIREDOR (0x6).
    wr(port(PORT_A, P_MODEL), (0xA << 4) | (0x6 << 8));
    wr(port(PORT_A, P_DOUT), 0b110);
    let din_hi = (rd(port(PORT_A, P_DIN)) >> 1) & 3;
    wr(port(PORT_A, P_DOUT), 0);
    let din_lo = (rd(port(PORT_A, P_DIN)) >> 1) & 3;
    puts("gpio pa1 wiredand pa2 wiredor hi=");
    hex(din_hi, 1);
    puts(" lo=");
    hex(din_lo, 1);
    putc(b' ');
    verdict(din_hi == 0b11 && din_lo == 0b00)
}

fn check_pinlock() -> bool {
    let reset = rd(port(PORT_A, P_PINLOCKN));
    wr(port(PORT_A, P_PINLOCKN), 0xFFFE);
    wr(port(PORT_A, P_PINLOCKN), 0xFFFF);
    let after = rd(port(PORT_A, P_PINLOCKN));
    puts("gpio pa pinlockn reset=");
    hex(reset, 4);
    puts(" after=");
    hex(after, 4);
    putc(b' ');
    verdict(reset == 0xFFFF && after == 0xFFFE)
}

/// CTRL resets to 0x00500050; OVTDIS is a 16-bit read/write word.
fn check_ctrl_ovtdis() -> bool {
    let ctrl = rd(port(PORT_A, P_CTRL));
    wr(port(PORT_A, P_OVTDIS), 0xFFFF_FFFF);
    let ovt = rd(port(PORT_A, P_OVTDIS));
    wr(port(PORT_A, P_OVTDIS), 0);
    puts("gpio pa ctrl=");
    hex(ctrl, 8);
    puts(" ovtdis=");
    hex(ovt, 4);
    putc(b' ');
    verdict(ctrl == CTRL_RESET && ovt == 0xFFFF)
}

pub struct Board {
    /// First UART line, e.g. "EFM32PG1 RENODE SMOKE".
    pub banner: &'static str,
    /// Verdict prefix, e.g. "EFM32PG1" -> "EFM32PG1 PASS".
    pub family: &'static str,
    /// Summary word address, kept above the linker's RAM.
    pub summary: u32,
    /// LED0 as (port index, pin); LED1 is the next pin on the same port.
    pub led: (u32, u32),
}

/// Run every check, store the summary word (0xEF310000 | one bit per check
/// = 0xEF31003F on a pass), then toggle LED0 forever on a busy-wait delay.
pub fn run(board: &Board) -> ! {
    wr(USART0 + U_CMD, CMD_TXEN);
    puts(board.banner);
    puts("\r\n");
    let checks = [
        check_pushpull(board),
        check_reserved(board),
        check_disabled_and_pull(),
        check_wired(),
        check_pinlock(),
        check_ctrl_ovtdis(),
    ];
    let mut summary = 0xEF31_0000u32;
    for (i, ok) in checks.iter().enumerate() {
        if *ok {
            summary |= 1 << i;
        }
    }
    wr(board.summary, summary);
    puts(board.family);
    if summary == 0xEF31_003F {
        puts(" PASS\r\n");
    } else {
        puts(" FAIL\r\n");
    }
    loop {
        for _ in 0..2_000u32 {
            core::hint::spin_loop();
        }
        wr(port(board.led.0, P_DOUTTGL), 1 << board.led.1);
    }
}
