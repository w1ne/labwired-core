//! EFM32GG995 (Giant Gecko, Cortex-M3) Renode-parity smoke.
//!
//! Checks the blocks Renode's efm32gg942.repl maps and the chip descriptor
//! claims: USART0 as the console, the Series-0 GPIO ports (`efm32s0_gpioport`)
//! and TIMER0..TIMER3 (`efm32s0_timer`). PE2 (STK3700 LED0) toggles in the
//! idle loop, paced by TIMER0 overflows. Summary word at 0x2001FF00.
#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// EFM32G USART0 register map: CMD@0x0C, STATUS@0x10, TXDATA@0x34.
const USART0_CMD: u32 = 0x4000_C00C;
const USART0_STATUS: u32 = 0x4000_C010;
const USART0_TXDATA: u32 = 0x4000_C034;
const CMD_TXEN: u32 = 1 << 0;
const STATUS_TXBL: u32 = 1 << 6;

// GPIO_P_TypeDef (efm32gg_gpio_p.h), ports at a 0x24 stride from 0x40006000.
const GPIO: u32 = 0x4000_6000;
const PORT_A: u32 = 0;
const PORT_E: u32 = 4;
const P_MODEL: u32 = 0x04;
const P_DOUT: u32 = 0x0C;
const P_DOUTSET: u32 = 0x10;
const P_DOUTCLR: u32 = 0x14;
const P_DOUTTGL: u32 = 0x18;
const P_DIN: u32 = 0x1C;
const P_PINLOCKN: u32 = 0x20;

// TIMER_TypeDef (efm32gg_timer.h), TIMER0..3 at 0x40010000 + n * 0x400.
const TIMER: u32 = 0x4001_0000;
const T_CTRL: u32 = 0x00;
const T_CMD: u32 = 0x04;
const T_STATUS: u32 = 0x08;
const T_IEN: u32 = 0x0C;
const T_IF: u32 = 0x10;
const T_IFC: u32 = 0x18;
const T_TOP: u32 = 0x1C;
const T_TOPB: u32 = 0x20;
const T_CNT: u32 = 0x24;
const T_CC0_CTRL: u32 = 0x30;
const T_CC0_CCV: u32 = 0x34;
const START: u32 = 1;
const STOP: u32 = 2;
const IF_OF: u32 = 1;
const IF_UF: u32 = 2;
const IF_CC0: u32 = 1 << 4;
const ST_RUNNING: u32 = 1;
const ST_DIR: u32 = 2;
const ST_TOPBV: u32 = 4;

const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ICPR0: u32 = 0xE000_E280;
const TIMER0_IRQ: u32 = 2;

const SUMMARY: u32 = 0x2001_FF00;

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn port(n: u32, reg: u32) -> u32 {
    GPIO + n * 0x24 + reg
}

fn timer(n: u32, reg: u32) -> u32 {
    TIMER + n * 0x400 + reg
}

fn putc(byte: u8) {
    let mut remaining = 100_000u32;
    while rd(USART0_STATUS) & STATUS_TXBL == 0 && remaining > 0 {
        remaining -= 1;
    }
    wr(USART0_TXDATA, byte as u32);
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

fn flag(name: &str, value: bool) {
    puts(name);
    putc(if value { b'1' } else { b'0' });
    putc(b' ');
}

fn verdict(ok: bool) -> bool {
    puts(if ok { "ok\r\n" } else { "FAIL\r\n" });
    ok
}

/// Poll `cond` a bounded number of times.
fn wait(cond: impl Fn() -> bool) -> bool {
    for _ in 0..200_000u32 {
        if cond() {
            return true;
        }
    }
    false
}

fn bit(addr: u32, n: u32) -> bool {
    (rd(addr) >> n) & 1 != 0
}

/// PE2/PE3 PUSHPULL: SET/CLR/TGL act on DOUT, DIN reads the driven pins,
/// and the command words read 0.
fn check_pushpull() -> bool {
    wr(port(PORT_E, P_MODEL), 0x4400);
    wr(port(PORT_E, P_DOUTSET), 1 << 2);
    let set = rd(port(PORT_E, P_DIN)) & 0xC == 0x4;
    wr(port(PORT_E, P_DOUTTGL), 0xC);
    let tgl = rd(port(PORT_E, P_DOUT)) & 0xC == 0x8 && rd(port(PORT_E, P_DIN)) & 0xC == 0x8;
    wr(port(PORT_E, P_DOUTCLR), 1 << 3);
    let clr = rd(port(PORT_E, P_DIN)) & 0xC == 0;
    let wo = rd(port(PORT_E, P_DOUTSET)) == 0 && rd(port(PORT_E, P_DOUTTGL)) == 0;
    puts("gpio pe2/pe3 pushpull ");
    flag("set=", set);
    flag("tgl=", tgl);
    flag("clr=", clr);
    flag("wo=", wo);
    verdict(set && tgl && clr && wo)
}

/// PA0: DISABLED reads 0 in DIN even with DOUT = 1; INPUTPULL reads the
/// rail DOUT picks.
fn check_disabled_and_pull() -> bool {
    wr(port(PORT_A, P_MODEL), 0x0);
    wr(port(PORT_A, P_DOUTSET), 1);
    let disabled = !bit(port(PORT_A, P_DIN), 0);
    wr(port(PORT_A, P_MODEL), 0x2);
    let up = bit(port(PORT_A, P_DIN), 0);
    wr(port(PORT_A, P_DOUTCLR), 1);
    let down = !bit(port(PORT_A, P_DIN), 0);
    puts("gpio pa0 ");
    flag("disabled_din0=", disabled);
    flag("pullup=", up);
    flag("pulldown=", down);
    verdict(disabled && up && down)
}

/// PA1 WIREDANDPULLUP (open-drain + pull-up) and PA2 WIREDOR (open-source):
/// each drives only one level and releases the other.
fn check_wired() -> bool {
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
    // Released WIREDAND pulled up reads 1, WIREDOR driving 1 reads 1;
    // WIREDAND driving 0 reads 0, released WIREDOR (nothing outside) 0.
    verdict(din_hi == 0b11 && din_lo == 0b00)
}

/// PINLOCKN only clears.
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

/// TIMER0 UP: OF pends NVIC line 2 (IEN.OF), IFC releases it.
fn check_timer0() -> bool {
    wr(timer(0, T_TOP), 999);
    wr(timer(0, T_IEN), IF_OF);
    wr(timer(0, T_CMD), START);
    let of = wait(|| rd(timer(0, T_IF)) & IF_OF != 0);
    let pend = bit(NVIC_ISPR0, TIMER0_IRQ);
    wr(timer(0, T_IFC), IF_OF);
    wr(NVIC_ICPR0, 1 << TIMER0_IRQ);
    let released = !bit(NVIC_ISPR0, TIMER0_IRQ);
    let running = rd(timer(0, T_STATUS)) & ST_RUNNING != 0;
    puts("tim0 up ");
    flag("of=", of);
    flag("pend=", pend);
    flag("released=", released);
    flag("running=", running);
    verdict(of && pend && released && running)
}

/// TIMER1 DOWN + one-shot: underflows to TOP once, raises UF, stops.
fn check_timer1() -> bool {
    wr(timer(1, T_CTRL), 1 | (1 << 4));
    wr(timer(1, T_TOP), 200);
    wr(timer(1, T_CNT), 50);
    let dir = rd(timer(1, T_STATUS)) & ST_DIR != 0;
    wr(timer(1, T_CMD), START);
    let stopped = wait(|| rd(timer(1, T_STATUS)) & ST_RUNNING == 0);
    let flags = rd(timer(1, T_IF)) & (IF_OF | IF_UF);
    let cnt = rd(timer(1, T_CNT));
    puts("tim1 down oneshot ");
    flag("dir=", dir);
    flag("stopped=", stopped);
    puts("if=");
    hex(flags, 1);
    puts(" cnt=");
    hex(cnt, 4);
    putc(b' ');
    verdict(dir && stopped && flags == IF_UF && cnt == 200)
}

/// TIMER2: CC0 output compare flags at CCV; TOPB loads at the next wrap.
fn check_timer2() -> bool {
    wr(timer(2, T_TOP), 1000);
    wr(timer(2, T_CC0_CTRL), 2);
    wr(timer(2, T_CC0_CCV), 100);
    wr(timer(2, T_CMD), START);
    let cc0 = wait(|| rd(timer(2, T_IF)) & IF_CC0 != 0);
    let past = rd(timer(2, T_CNT)) >= 100;
    wr(timer(2, T_TOPB), 500);
    let topbv = rd(timer(2, T_STATUS)) & ST_TOPBV != 0;
    let old_top = rd(timer(2, T_TOP));
    let of = wait(|| rd(timer(2, T_IF)) & IF_OF != 0);
    let top = rd(timer(2, T_TOP));
    let loaded = rd(timer(2, T_STATUS)) & ST_TOPBV == 0;
    wr(timer(2, T_CMD), STOP);
    puts("tim2 cc0 ");
    flag("hit=", cc0 && past);
    flag("topbv=", topbv);
    flag("of=", of);
    puts("top=");
    hex(old_top, 4);
    puts("->");
    hex(top, 4);
    putc(b' ');
    verdict(cc0 && past && topbv && of && old_top == 1000 && top == 500 && loaded)
}

/// TIMER3 UP/DOWN: OF at TOP turns it down (STATUS.DIR), UF at 0 turns it up.
fn check_timer3() -> bool {
    wr(timer(3, T_CTRL), 2);
    wr(timer(3, T_TOP), 40);
    wr(timer(3, T_CMD), START);
    let of = wait(|| rd(timer(3, T_IF)) & IF_OF != 0);
    let down = rd(timer(3, T_STATUS)) & ST_DIR != 0;
    let uf = wait(|| rd(timer(3, T_IF)) & IF_UF != 0);
    // Sampled right after UF, well before the next turn at TOP.
    let up = rd(timer(3, T_STATUS)) & ST_DIR == 0;
    wr(timer(3, T_CMD), STOP);
    puts("tim3 updown ");
    flag("of=", of);
    flag("dir_down=", down);
    flag("uf=", uf);
    flag("up_again=", up);
    verdict(of && down && uf && up)
}

#[entry]
fn main() -> ! {
    wr(USART0_CMD, CMD_TXEN);
    puts("EFM32GG995 RENODE SMOKE\r\n");
    let checks = [
        check_pushpull(),
        check_disabled_and_pull(),
        check_wired(),
        check_pinlock(),
        check_timer0(),
        check_timer1(),
        check_timer2(),
        check_timer3(),
    ];
    let mut summary = 0xEF32_0000u32;
    for (i, ok) in checks.iter().enumerate() {
        if *ok {
            summary |= 1 << i;
        }
    }
    wr(SUMMARY, summary);
    if summary == 0xEF32_00FF {
        puts("EFM32GG PASS\r\n");
    } else {
        puts("EFM32GG FAIL\r\n");
    }
    // Idle: toggle PE2 (STK3700 LED0) on every TIMER0 overflow.
    wr(timer(0, T_IEN), 0);
    loop {
        if rd(timer(0, T_IF)) & IF_OF != 0 {
            wr(timer(0, T_IFC), IF_OF);
            wr(port(PORT_E, P_DOUTTGL), 1 << 2);
        }
    }
}
