//! UT32M0R500 (Frontgrade/CAES, Cortex-M0+) smoke on the Renode
//! platform surface: UART0 + UART1 (Gaisler APBUART) and the three Arm
//! CMSDK AHB GPIO banks with the UT32 pull registers, plus both ARM SP804
//! dual-timer windows.
//!
//! GPIO register map: Frontgrade application note "UT32M0R500 Enable the
//! GPIO" (12/21/2017) and Renode `ARM_AHB_GPIO`. GPIO number = 16 * bank + n.
//!
//! Checks, one letter each on the summary line (P pass, F fail):
//!   1 reset    GPIO0 OUTEN/ALTFUNC/INTEN read 0 after reset
//!   2 altfunc  GPIO2 ALTFUNC resets to 0xFFFF (Renode
//!              alternateFunctionResetValue); ALTFUNCCLR frees pin 13
//!   3 pullup   GPIO0 pin 3 PULL_ENABLE + PULL_UP_DOWN = 1 reads 1
//!   4 pulldn   PULL_UP_DOWN = 0 reads 0
//!   5 output   GPIO1 pins 0..3 OUTENSET, DATAOUT, DATA reads the pads
//!   6 masklow  MASKLOWBYTE[0x0C] = 0x04 changes pins 2/3 only
//!   7 maskhigh GPIO1 pin 9 through MASKHIGHBYTE[0x02]
//!   8 outenclr OUTENCLR releases pin 0; DATA follows its pull-down
//!   9 intedge  GPIO1 pin 1 rising-edge INTSTATUS latches; write 1 clears
//!  10 sram     a pattern at the top of the 96 KiB SRAM reads back
//!  11 timer    SP804 timer0 channel 1 and timer1 channel 2 count down, set
//!              RIS/MIS, and clear their interrupt latches
//! The summary word 0x5E32_0000 | pass mask (all pass: 0x5E32_07FF) goes to
//! 0x2001_7F00. GPIO2 pin 13 (GPIO 45, the app note's output) then pulses
//! three times, and UART1 prints one line last.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ── Gaisler APBUART (Renode ut32m0r500.repl uart0/uart1) ────────────────────
const UART0: u32 = 0x4000_4000;
const UART1: u32 = 0x4000_5000;
const UART_DATA: u32 = 0x00;
const UART_STATUS: u32 = 0x04;
const UART_TX_READY: u32 = 0x06; // TS | TE

// ── CMSDK AHB GPIO banks 0..2 ───────────────────────────────────────────────
const GPIO0: u32 = 0x4002_0000;
const GPIO1: u32 = 0x4002_1000;
const GPIO2: u32 = 0x4002_2000;
const TIMER0: u32 = 0x4000_1000;
const TIMER1: u32 = 0x4000_2000;
const TIMER_LOAD: u32 = 0x00;
const TIMER_CONTROL: u32 = 0x08;
const TIMER_INTCLR: u32 = 0x0C;
const TIMER_RIS: u32 = 0x10;
const TIMER_MIS: u32 = 0x14;
const DATA: u32 = 0x00;
const DATAOUT: u32 = 0x04;
const OUTENSET: u32 = 0x10;
const OUTENCLR: u32 = 0x14;
const ALTFUNCSET: u32 = 0x18;
const ALTFUNCCLR: u32 = 0x1C;
const INTENSET: u32 = 0x20;
const INTTYPESET: u32 = 0x28;
const INTPOLSET: u32 = 0x30;
const INTSTATUS: u32 = 0x38;
const PULL_ENABLE: u32 = 0x40;
const PULL_UP_DOWN: u32 = 0x44;
const MASKLOWBYTE: u32 = 0x400;
const MASKHIGHBYTE: u32 = 0x800;

const GPIO45: u32 = 13; // GPIO2 pin 13

const SRAM_TOP_WORD: u32 = 0x2001_7FFC;
const SUMMARY: u32 = 0x2001_7F00;
const SUMMARY_TAG: u32 = 0x5E32_0000;

fn rd(addr: u32) -> u32 {
    unsafe { read_volatile(addr as *const u32) }
}

fn wr(addr: u32, value: u32) {
    unsafe { write_volatile(addr as *mut u32, value) }
}

fn putc(uart: u32, byte: u8) {
    while rd(uart + UART_STATUS) & UART_TX_READY != UART_TX_READY {}
    wr(uart + UART_DATA, u32::from(byte));
}

fn puts(uart: u32, text: &[u8]) {
    for byte in text {
        putc(uart, *byte);
    }
}

fn spin(iterations: u32) {
    for _ in 0..iterations {
        core::hint::spin_loop();
    }
}

fn check_reset() -> bool {
    rd(GPIO0 + OUTENSET) == 0 && rd(GPIO0 + ALTFUNCSET) == 0 && rd(GPIO0 + INTENSET) == 0
}

fn check_altfunc() -> bool {
    let reset = rd(GPIO2 + ALTFUNCSET) == 0xFFFF;
    wr(GPIO2 + ALTFUNCCLR, 1 << GPIO45);
    reset && rd(GPIO2 + ALTFUNCSET) == 0xFFFF & !(1 << GPIO45)
}

fn check_pull(up: bool) -> bool {
    wr(GPIO0 + PULL_ENABLE, 1 << 3);
    wr(GPIO0 + PULL_UP_DOWN, if up { 1 << 3 } else { 0 });
    (rd(GPIO0 + DATA) >> 3) & 1 == u32::from(up)
}

fn check_output() -> bool {
    // Pull-downs under the outputs so a released pad reads a known 0.
    wr(GPIO1 + PULL_ENABLE, 0x020F);
    wr(GPIO1 + PULL_UP_DOWN, 0);
    wr(GPIO1 + DATAOUT, 0x0005);
    wr(GPIO1 + OUTENSET, 0x000F);
    rd(GPIO1 + OUTENCLR) == 0x000F && rd(GPIO1 + DATA) & 0x000F == 0x0005
}

fn check_mask_low() -> bool {
    // Mask 0x0C: pin 2 -> 0, pin 3 -> 1; pins 0/1 keep 1/0.
    wr(GPIO1 + MASKLOWBYTE + 4 * 0x0C, 0x08);
    rd(GPIO1 + DATA) & 0x000F == 0x0009 && rd(GPIO1 + MASKLOWBYTE + 4 * 0x0C) == 0x08
}

fn check_mask_high() -> bool {
    wr(GPIO1 + OUTENSET, 1 << 9);
    wr(GPIO1 + MASKHIGHBYTE + 4 * 0x02, 0xFFFF);
    let on = rd(GPIO1 + DATA) & (1 << 9) != 0;
    wr(GPIO1 + MASKHIGHBYTE + 4 * 0x02, 0);
    on && rd(GPIO1 + DATA) & (1 << 9) == 0 && rd(GPIO1 + DATAOUT) & 0x000F == 0x0009
}

fn check_outen_clear() -> bool {
    wr(GPIO1 + OUTENCLR, 1 << 0);
    rd(GPIO1 + OUTENSET) & 0x000F == 0x000E && rd(GPIO1 + DATA) & 1 == 0
}

fn check_int_edge() -> bool {
    // Pin 1 is an output at 0. Rising edge, enabled; drive it high.
    wr(GPIO1 + INTSTATUS, 0xFFFF);
    wr(GPIO1 + INTTYPESET, 1 << 1);
    wr(GPIO1 + INTPOLSET, 1 << 1);
    wr(GPIO1 + INTENSET, 1 << 1);
    let idle = rd(GPIO1 + INTSTATUS) & (1 << 1) == 0;
    wr(GPIO1 + MASKLOWBYTE + 4 * 0x02, 0x02);
    let latched = rd(GPIO1 + INTSTATUS) & (1 << 1) != 0;
    wr(GPIO1 + INTSTATUS, 1 << 1);
    idle && latched && rd(GPIO1 + INTSTATUS) & (1 << 1) == 0
}

fn check_sram_top() -> bool {
    wr(SRAM_TOP_WORD, 0x5A5A_C3C3);
    let ok = rd(SRAM_TOP_WORD) == 0x5A5A_C3C3;
    wr(SRAM_TOP_WORD, !0x5A5A_C3C3);
    ok && rd(SRAM_TOP_WORD) == !0x5A5A_C3C3
}

fn check_sp804() -> bool {
    // Exercise one channel in each SP804 block. The control value selects
    // enabled, periodic, 32-bit operation with its interrupt output enabled.
    for (base, channel) in [(TIMER0, 0), (TIMER1, 1)] {
        let load = base + channel * 0x20 + TIMER_LOAD;
        let control = base + channel * 0x20 + TIMER_CONTROL;
        let intclr = base + channel * 0x20 + TIMER_INTCLR;
        let ris = base + channel * 0x20 + TIMER_RIS;
        let mis = base + channel * 0x20 + TIMER_MIS;
        wr(intclr, 0);
        wr(load, 32);
        wr(control, (1 << 7) | (1 << 6) | (1 << 5) | (1 << 1));
        let mut fired = false;
        for _ in 0..256 {
            if rd(ris) & 1 != 0 {
                fired = true;
                break;
            }
        }
        let masked = rd(mis) & 1 != 0;
        wr(intclr, 0);
        let cleared = rd(ris) & 1 == 0 && rd(mis) & 1 == 0;
        wr(control, 0x20);
        if !(fired && masked && cleared) {
            return false;
        }
    }
    true
}

#[entry]
fn main() -> ! {
    puts(UART0, b"UT32M0R500 Cortex-M0+ UART0 smoke\n");

    let checks = [
        check_reset(),
        check_altfunc(),
        check_pull(true),
        check_pull(false),
        check_output(),
        check_mask_low(),
        check_mask_high(),
        check_outen_clear(),
        check_int_edge(),
        check_sram_top(),
        check_sp804(),
    ];

    let mut mask = 0u32;
    puts(UART0, b"UT32 checks ");
    for (i, ok) in checks.iter().enumerate() {
        if *ok {
            mask |= 1 << i;
        }
        putc(UART0, if *ok { b'P' } else { b'F' });
    }
    puts(UART0, b" DONE\n");
    wr(SUMMARY, SUMMARY_TAG | mask);

    // GPIO 45 (GPIO2 pin 13) as an output, three pulses.
    wr(GPIO2 + OUTENSET, 1 << GPIO45);
    for _ in 0..3 {
        wr(GPIO2 + MASKHIGHBYTE + 4 * 0x20, 0x2000);
        spin(2_000);
        wr(GPIO2 + MASKHIGHBYTE + 4 * 0x20, 0);
        spin(2_000);
    }

    puts(UART1, b"UT32 UART1 line\n");

    loop {
        core::hint::spin_loop();
    }
}
