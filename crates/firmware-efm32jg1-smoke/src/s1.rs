//! EFM32 Series-1 GPIO checks, shared by the JG1/JG12/PG1/PG12 smokes
//! (`#[path]`-included).
//!
//! Checks the Series-1 GPIO ports (`efm32s1_gpioport`): GPIO_P_TypeDef at a
//! 0x30 stride from 0x4000A000 (Gecko SDK `efm32pg1b_gpio_p.h`). Unlike
//! Series 0 there are no DOUTSET/DOUTCLR words — 0x10/0x14 are reserved, so
//! this firmware sets and clears through DOUT and toggles through DOUTTGL —
//! and OVTDIS sits at 0x28. USART0 (0x40010000) is the console.
//!
//! Also checks TIMER0/TIMER1 (`efm32s1_timer`, Gecko SDK
//! `efm32pg1b_timer.h`): the Series-1 layout puts IF/IFS/IFC/IEN at
//! 0x0C/0x10/0x14/0x18 (Series 0: IEN 0x0C, IF 0x10, IFC 0x18) and four
//! compare channels at 0x60, and raises IF.DIRCHG when UP/DOWN turns.
//!
//! On parts with WTIMERs (`efm32s1_wtimer`, PG12/JG12/MG12: WTIMER0/1 at
//! 0x4001A000/0x4001A400; MG13: WTIMER0 only) it checks the 32-bit counter:
//! counting up from 0xFFF0, CC0 at 0x10000 fires before any overflow (a
//! 16-bit counter would wrap first) and OF comes at a TOP above 0xFFFF.
//!
//! On parts where Renode models the RTCC (`efm32s1_rtcc`, MG12 IRQ 30, MG13
//! IRQ 31; Gecko SDK `efr32mg12p_rtcc.h`) it checks the 32 retention words,
//! CNT wrapping to OF just before CC1 matches, CNTTICK, the IEN-gated NVIC
//! line, COMBCNT = PRECNT | CNT << 15, and the CNTPRESC pre-counter tap.
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

// TIMER_TypeDef (efm32pg1b_timer.h), TIMER0/1 at 0x40018000 + n * 0x400.
const TIMER: u32 = 0x4001_8000;
const T_CTRL: u32 = 0x00;
const T_CMD: u32 = 0x04;
const T_STATUS: u32 = 0x08;
const T_IF: u32 = 0x0C;
const T_IFC: u32 = 0x14;
const T_IEN: u32 = 0x18;
const T_TOP: u32 = 0x1C;
const T_CNT: u32 = 0x24;
const T_CC3_CTRL: u32 = 0x90;
const T_CC3_CCV: u32 = 0x94;
const START: u32 = 1;
const STOP: u32 = 2;
const IF_OF: u32 = 1;
const IF_UF: u32 = 2;
const IF_DIRCHG: u32 = 4;
const IF_CC3: u32 = 1 << 7;
const ST_RUNNING: u32 = 1;
const ST_DIR: u32 = 2;

// WTIMER_TypeDef = TIMER_TypeDef, WTIMER0/1 at 0x4001A000 + n * 0x400.
const WTIMER: u32 = 0x4001_A000;
const T_IFS: u32 = 0x10;
const T_CC0_CTRL: u32 = 0x60;
const T_CC0_CCV: u32 = 0x64;
const IF_CC0: u32 = 1 << 4;
const W_TOP: u32 = 0x0001_1000;

// RTCC_TypeDef (efr32mg12p_rtcc.h) at 0x40042000.
const RTCC: u32 = 0x4004_2000;
const R_CTRL: u32 = 0x00;
const R_PRECNT: u32 = 0x04;
const R_CNT: u32 = 0x08;
const R_COMBCNT: u32 = 0x0C;
const R_IF: u32 = 0x18;
const R_IFC: u32 = 0x20;
const R_IEN: u32 = 0x24;
const R_LOCK: u32 = 0x38;
const R_CC1_CTRL: u32 = 0x50;
const R_CC1_CCV: u32 = 0x54;
const R_RET0: u32 = 0x104;
const R_RET31: u32 = 0x180;
const R_ENABLE: u32 = 1;
const R_IF_OF: u32 = 1;
const R_IF_CC1: u32 = 1 << 2;
const R_IF_CNTTICK: u32 = 1 << 5;
/// _RTCC_LOCK_LOCKKEY_UNLOCK.
const R_UNLOCK: u32 = 0xAEE8;

const NVIC_ISPR0: u32 = 0xE000_E200;
const NVIC_ICPR0: u32 = 0xE000_E280;

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn port(n: u32, reg: u32) -> u32 {
    GPIO + n * 0x30 + reg
}

fn timer(n: u32, reg: u32) -> u32 {
    TIMER + n * 0x400 + reg
}

fn wtimer(n: u32, reg: u32) -> u32 {
    WTIMER + n * 0x400 + reg
}

/// NVIC set/clear-pending word and bit for `irq` (WTIMERs sit above 31).
fn nvic(base: u32, irq: u32) -> (u32, u32) {
    (base + 4 * (irq / 32), irq % 32)
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

/// TIMER0 UP: OF latches in IF (0x0C), pends the NVIC line only once IEN
/// (0x18) enables it, and IFC (0x14) releases it.
fn check_timer0(board: &Board) -> bool {
    let irq = board.timer0_irq;
    wr(timer(0, T_TOP), 999);
    wr(timer(0, T_CMD), START);
    let of = wait(|| rd(timer(0, T_IF)) & IF_OF != 0);
    let masked = !bit(NVIC_ISPR0, irq);
    wr(timer(0, T_IEN), IF_OF);
    let pend = bit(NVIC_ISPR0, irq);
    wr(timer(0, T_IFC), IF_OF);
    wr(NVIC_ICPR0, 1 << irq);
    let released = !bit(NVIC_ISPR0, irq);
    let running = rd(timer(0, T_STATUS)) & ST_RUNNING != 0;
    wr(timer(0, T_IEN), 0);
    puts("tim0 up ");
    flag("of=", of);
    flag("masked=", masked);
    flag("pend=", pend);
    flag("released=", released);
    flag("running=", running);
    verdict(of && masked && pend && released && running)
}

/// TIMER1 DOWN + one-shot: underflows to TOP once, raises UF, stops.
fn check_timer1_oneshot() -> bool {
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

/// TIMER1 UP/DOWN with CC3 (0x90, Series 1 only) in output compare: CC3
/// flags on the way up, OF at TOP turns it down and raises DIRCHG.
fn check_timer1_updown_cc3() -> bool {
    wr(timer(1, T_IFC), 0xFFF);
    wr(timer(1, T_CTRL), 2);
    wr(timer(1, T_TOP), 400);
    wr(timer(1, T_CNT), 0);
    wr(timer(1, T_CC3_CTRL), 2);
    wr(timer(1, T_CC3_CCV), 100);
    wr(timer(1, T_CMD), START);
    let cc3 = wait(|| rd(timer(1, T_IF)) & IF_CC3 != 0);
    let of = wait(|| rd(timer(1, T_IF)) & IF_OF != 0);
    let dirchg = rd(timer(1, T_IF)) & IF_DIRCHG != 0;
    let down = rd(timer(1, T_STATUS)) & ST_DIR != 0;
    wr(timer(1, T_CMD), STOP);
    puts("tim1 updown ");
    flag("cc3=", cc3);
    flag("of=", of);
    flag("dirchg=", dirchg);
    flag("dir_down=", down);
    verdict(cc3 && of && dirchg && down)
}

/// WTIMERn UP across the 16-bit boundary: TOP/CNT hold 32 bits, CC0 at
/// 0x10000 flags with no OF yet, OF comes at TOP 0x11000, and IEN pends the
/// NVIC line (36/37: ISPR1).
fn check_wtimer(n: u32, irq: u32) -> bool {
    wr(wtimer(n, T_IFC), 0xFFF);
    wr(wtimer(n, T_TOP), W_TOP);
    let top = rd(wtimer(n, T_TOP)) == W_TOP;
    wr(wtimer(n, T_CNT), 0xFFF0);
    wr(wtimer(n, T_CC0_CTRL), 2);
    wr(wtimer(n, T_CC0_CCV), 0x0001_0000);
    wr(wtimer(n, T_CMD), START);
    let cc0 = wait(|| rd(wtimer(n, T_IF)) & IF_CC0 != 0);
    let no_early_of = rd(wtimer(n, T_IF)) & IF_OF == 0;
    let cnt_hi = rd(wtimer(n, T_CNT)) >= 0x0001_0000;
    let of = wait(|| rd(wtimer(n, T_IF)) & IF_OF != 0);
    wr(wtimer(n, T_CMD), STOP);
    let (ispr, b) = nvic(NVIC_ISPR0, irq);
    let (icpr, _) = nvic(NVIC_ICPR0, irq);
    let masked = !bit(ispr, b);
    wr(wtimer(n, T_IEN), IF_OF);
    let pend = bit(ispr, b);
    wr(wtimer(n, T_IFC), 0xFFF);
    wr(icpr, 1 << b);
    let released = !bit(ispr, b);
    wr(wtimer(n, T_IEN), 0);
    // IFS sets flags by software; the line stays down with IEN clear.
    wr(wtimer(n, T_IFS), IF_OF);
    let ifs = rd(wtimer(n, T_IF)) & IF_OF != 0 && !bit(ispr, b);
    wr(wtimer(n, T_IFC), 0xFFF);
    puts("wtim");
    putc(b'0' + n as u8);
    puts(" 32bit ");
    flag("top=", top);
    flag("cc0=", cc0);
    flag("no_early_of=", no_early_of);
    flag("cnt_hi=", cnt_hi);
    flag("of=", of);
    flag("masked=", masked);
    flag("pend=", pend);
    flag("released=", released);
    flag("ifs=", ifs);
    verdict(top && cc0 && no_early_of && cnt_hi && of && masked && pend && released && ifs)
}

/// RTCC normal mode at LFECLK/1 from 0xFFFFFFFD: OF on the wrap to 0, then
/// CC1 (CCV 1); IEN pends the NVIC line. Then CNTPRESC DIV4 from zero: CNT
/// follows PRECNT >> 2, and COMBCNT joins the frozen pair.
fn check_rtcc(irq: u32) -> bool {
    wr(RTCC + R_LOCK, R_UNLOCK);
    wr(RTCC + R_RET0, 0xA5A5_5A5A);
    wr(RTCC + R_RET31, 0x0BAD_F00D);
    let ret = rd(RTCC + R_RET0) == 0xA5A5_5A5A && rd(RTCC + R_RET31) == 0x0BAD_F00D;
    wr(RTCC + R_IFC, 0x7FF);
    wr(RTCC + R_CNT, 0xFFFF_FFFD);
    wr(RTCC + R_CC1_CTRL, 2);
    wr(RTCC + R_CC1_CCV, 1);
    wr(RTCC + R_CTRL, R_ENABLE);
    let of = wait(|| rd(RTCC + R_IF) & R_IF_OF != 0);
    let of_first = rd(RTCC + R_IF) & R_IF_CC1 == 0;
    let cc1 = wait(|| rd(RTCC + R_IF) & R_IF_CC1 != 0);
    let tick = rd(RTCC + R_IF) & R_IF_CNTTICK != 0;
    let (ispr, b) = nvic(NVIC_ISPR0, irq);
    let (icpr, _) = nvic(NVIC_ICPR0, irq);
    let masked = !bit(ispr, b);
    wr(RTCC + R_IEN, R_IF_CC1);
    let pend = bit(ispr, b);
    wr(RTCC + R_IFC, 0x7FF);
    wr(icpr, 1 << b);
    let released = !bit(ispr, b);
    wr(RTCC + R_IEN, 0);
    // CNTPRESC DIV4 from zero.
    wr(RTCC + R_CTRL, 0);
    wr(RTCC + R_PRECNT, 0);
    wr(RTCC + R_CNT, 0);
    wr(RTCC + R_CTRL, R_ENABLE | (2 << 8));
    let counted = wait(|| rd(RTCC + R_CNT) >= 3);
    wr(RTCC + R_CTRL, 0);
    let pre = rd(RTCC + R_PRECNT);
    let cnt = rd(RTCC + R_CNT);
    let presc = counted && cnt == pre >> 2;
    let comb = rd(RTCC + R_COMBCNT) == pre | (cnt << 15);
    puts("rtcc ");
    flag("ret=", ret);
    flag("of=", of);
    flag("of_first=", of_first);
    flag("cc1=", cc1);
    flag("tick=", tick);
    flag("masked=", masked);
    flag("pend=", pend);
    flag("released=", released);
    flag("presc=", presc);
    flag("comb=", comb);
    verdict(ret && of && of_first && cc1 && tick && masked && pend && released && presc && comb)
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
    /// TIMER0's NVIC line (10 on PG1/JG1, 11 on PG12/JG12).
    pub timer0_irq: u32,
    /// NVIC lines of WTIMER0, WTIMER1, ... (empty where there is none).
    pub wtimer_irqs: &'static [u32],
    /// The RTCC's NVIC line where the descriptor maps one (MG12 30, MG13 31).
    pub rtcc_irq: Option<u32>,
}

/// Run every check, store the summary word (0xEF310000 | one bit per check
/// = 0xEF3101FF on a pass, plus bits 9.. for the WTIMERs: 0xEF3103FF with
/// one, 0xEF3107FF with two, and the next bit for the RTCC), then toggle LED0 forever on a busy-wait delay.
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
        check_timer0(board),
        check_timer1_oneshot(),
        check_timer1_updown_cc3(),
    ];
    let mut summary = 0xEF31_0000u32;
    for (i, ok) in checks.iter().enumerate() {
        if *ok {
            summary |= 1 << i;
        }
    }
    let mut expected = 0xEF31_01FFu32;
    for (n, &irq) in board.wtimer_irqs.iter().enumerate() {
        let bitn = checks.len() + n;
        expected |= 1 << bitn;
        if check_wtimer(n as u32, irq) {
            summary |= 1 << bitn;
        }
    }
    if let Some(irq) = board.rtcc_irq {
        let bitn = checks.len() + board.wtimer_irqs.len();
        expected |= 1 << bitn;
        if check_rtcc(irq) {
            summary |= 1 << bitn;
        }
    }
    wr(board.summary, summary);
    puts(board.family);
    if summary == expected {
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
