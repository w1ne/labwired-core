#![no_std]
#![no_main]

//! STM32W108 SC1 UART + TIM1/TIM2 counter smoke.
//!
//! Puts SC1 in UART mode, reads SC1_UARTSTAT at idle, and prints a banner
//! polled on SC_UARTTXFREE, then waits for SC_UARTTXIDLE. Then exercises the
//! STM32-layout counter registers (CR1, EGR, CNT, PSC, ARR) that the imported
//! profile maps at 0x4000_E000 / 0x4000_F000. Interrupt flags live in the
//! separate INT_SC1FLAG / INT_TIMxFLAG blocks on this part and are not
//! exercised. Results land in fixed RAM words the smoke script asserts.

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const SC1: u32 = 0x4000_C800;
const SC1_DATA: u32 = 0x3C;
const SC1_UARTSTAT: u32 = 0x48;
const SC1_MODE: u32 = 0x54;
const SC1_UARTCFG: u32 = 0x5C;
const SC1_UARTPER: u32 = 0x68;
const SC_UARTRXVAL: u32 = 1 << 1;
const SC_UARTTXFREE: u32 = 1 << 2;
const SC_UARTTXIDLE: u32 = 1 << 6;
const SC_MODE_UART: u32 = 1;

const TIM1: u32 = 0x4000_E000;
const TIM2: u32 = 0x4000_F000;
const CR1: u32 = 0x00;
const EGR: u32 = 0x14;
const CNT: u32 = 0x24;
const PSC: u32 = 0x28;
const ARR: u32 = 0x2C;

// Middle of RAM: clear of .data/.bss at the bottom and the stack at the top.
const RESULT: u32 = 0x2008_0000;
const MAGIC: u32 = 0x5710_0000;

fn rd(base: u32, off: u32) -> u32 {
    unsafe { read_volatile((base + off) as *const u32) }
}

fn wr(base: u32, off: u32, value: u32) {
    unsafe { write_volatile((base + off) as *mut u32, value) }
}

fn spin(n: u32) {
    for _ in 0..n {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}

fn sc1_putc(byte: u8) {
    while rd(SC1, SC1_UARTSTAT) & SC_UARTTXFREE == 0 {}
    wr(SC1, SC1_DATA, u32::from(byte));
}

#[entry]
fn main() -> ! {
    let mut bits = 0u32;

    // SC1 in UART mode, 8N1 (UARTCFG: SC_UART8BIT bit 1 | SC_UARTRTS bit 0)
    // and UARTPER for 115200 baud from the 24 MHz peripheral clock.
    wr(SC1, SC1_MODE, SC_MODE_UART);
    wr(SC1, SC1_UARTCFG, (1 << 1) | (1 << 0));
    wr(SC1, SC1_UARTPER, 104);
    let stat = rd(SC1, SC1_UARTSTAT);
    // Idle: transmitter free and idle, nothing received.
    if stat & (SC_UARTTXFREE | SC_UARTTXIDLE) == (SC_UARTTXFREE | SC_UARTTXIDLE)
        && stat & SC_UARTRXVAL == 0
    {
        bits |= 1 << 7;
    }
    for &b in b"STM32W108 SC1 SMOKE\r\n" {
        sc1_putc(b);
    }
    while rd(SC1, SC1_UARTSTAT) & SC_UARTTXIDLE == 0 {}
    bits |= 1 << 8;

    // Reset values: stopped, CNT 0, 16-bit ARR all ones.
    if rd(TIM1, CR1) == 0 && rd(TIM1, CNT) == 0 && rd(TIM1, ARR) == 0xFFFF {
        bits |= 1 << 0;
    }

    // A stopped counter holds its value.
    spin(200);
    if rd(TIM1, CNT) == 0 {
        bits |= 1 << 1;
    }

    // CEN starts it counting.
    wr(TIM1, CR1, 1);
    spin(200);
    if rd(TIM1, CNT) > 0 {
        bits |= 1 << 2;
    }

    // TIM2 with PSC=7 (loaded by UG) runs about 8x slower than TIM1.
    wr(TIM1, CR1, 0);
    wr(TIM1, EGR, 1);
    wr(TIM2, PSC, 7);
    wr(TIM2, EGR, 1);
    wr(TIM1, CR1, 1);
    wr(TIM2, CR1, 1);
    spin(2000);
    let c2 = rd(TIM2, CNT);
    let c1 = rd(TIM1, CNT);
    if c2 > 0 && c1 > c2 {
        bits |= 1 << 3;
    }
    if c2 * 6 <= c1 && c1 <= c2 * 10 {
        bits |= 1 << 4;
    }

    // ARR=99 bounds the count and wraps it back down.
    wr(TIM1, ARR, 99);
    wr(TIM1, EGR, 1);
    let mut prev = rd(TIM1, CNT);
    let mut bounded = true;
    let mut wrapped = false;
    for _ in 0..64 {
        spin(13);
        let now = rd(TIM1, CNT);
        bounded &= now <= 99;
        wrapped |= now < prev;
        prev = now;
    }
    if bounded && wrapped {
        bits |= 1 << 5;
    }

    // Clearing CEN freezes the counter.
    wr(TIM1, CR1, 0);
    let frozen = rd(TIM1, CNT);
    spin(200);
    if rd(TIM1, CNT) == frozen {
        bits |= 1 << 6;
    }

    wr(RESULT, 4, c1);
    wr(RESULT, 8, c2);
    wr(RESULT, 12, stat);
    wr(RESULT, 0, MAGIC | bits);

    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}
