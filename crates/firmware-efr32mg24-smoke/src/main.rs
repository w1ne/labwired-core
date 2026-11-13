// EFR32MG24 Renode-onboarding smoke. Keeps the USART0 banner, then exercises
// TIMER0 (0x4004_8000, NVIC line 4) and TIMER1 (0x4004_C000, NVIC line 5).
// Bases, IRQ numbers, CMU_CLKEN0 bits and TIMER_CNTWIDTH = 32 come from the
// Silicon Labs `efr32mg24b220f1536im48.h` / `efr32mg24_timer.h` headers;
// Renode's efr32xg24.repl maps the same blocks (it declares `width: 16`,
// the header says 32, and the profile follows the header).
//
// 1. TIMER0 holds a 32-bit TOP and CNT.
// 2. 19 MHz / (PRESC + 1) = 1 MHz: START sets STATUS.RUNNING and CNT advances.
// 3. TOP = 99 with IEN.OF: the overflow reaches the core through NVIC line 4
//    three times; the handler clears IF.OF through the +0x2000 CLR alias.
// 4. A direct IF store does not clear a flag (Series-2 has no IFC); the CLR
//    alias does.
// 5. TIMER1 with NVIC line 5 disabled: IF.OF with IEN clear does not pend the
//    line, setting IEN pends it, and clearing IF lets ICPR release it.
//
// Bare-register, no HAL.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, Ordering};
use cortex_m_rt::{entry, exception};
use panic_halt as _;

// EFR32 Series-2 CMU and USART0 register map, as used by the imported xG24 REPL.
const CMU_CLKEN0: *mut u32 = 0x4000_8064 as *mut u32;
const CMU_CLKEN0_TIMER0: u32 = 1 << 4;
const CMU_CLKEN0_TIMER1: u32 = 1 << 5;
const CMU_CLKEN0_USART0: u32 = 1 << 9;

const USART0_BASE: usize = 0x4005_C000;
const USART0_EN: *mut u32 = (USART0_BASE + 0x04) as *mut u32;
const USART0_CMD: *mut u32 = (USART0_BASE + 0x14) as *mut u32;
const USART0_STATUS: *const u32 = (USART0_BASE + 0x18) as *const u32;
const USART0_CLKDIV: *mut u32 = (USART0_BASE + 0x1C) as *mut u32;
const USART0_TXDATA: *mut u32 = (USART0_BASE + 0x38) as *mut u32;

const USART_CMD_TXEN: u32 = 1 << 2;
const USART_STATUS_TXBL: u32 = 1 << 6;
const USART0_CLKDIV_115200: u32 = 2384;

// TIMER_TypeDef offsets; +0x1000 is the SET alias, +0x2000 the CLR alias.
const TIMER0_BASE: usize = 0x4004_8000;
const TIMER1_BASE: usize = 0x4004_C000;
const T_CFG: usize = 0x04;
const T_CMD: usize = 0x0C;
const T_STATUS: usize = 0x10;
const T_IF: usize = 0x14;
const T_IEN: usize = 0x18;
const T_TOP: usize = 0x1C;
const T_CNT: usize = 0x24;
const T_EN: usize = 0x30;
const ALIAS_SET: usize = 0x1000;
const ALIAS_CLR: usize = 0x2000;

const CFG_PRESC_SHIFT: u32 = 18;
// HFRCODPLL_STARTUP_FREQ = 19 MHz; PRESC = 18 gives a 1 MHz count.
const PRESC_1MHZ: u32 = 18;
const CMD_START: u32 = 1 << 0;
const CMD_STOP: u32 = 1 << 1;
const STATUS_RUNNING: u32 = 1 << 0;
const IF_OF: u32 = 1 << 0;

const TIMER0_IRQN: i16 = 4;
const TIMER1_IRQN: i16 = 5;
const NVIC_ISER0: *mut u32 = 0xE000_E100 as *mut u32;
const NVIC_ICER0: *mut u32 = 0xE000_E180 as *mut u32;
const NVIC_ISPR0: *mut u32 = 0xE000_E200 as *mut u32;
const NVIC_ICPR0: *mut u32 = 0xE000_E280 as *mut u32;
const TIMER0_NVIC_BIT: u32 = 1 << TIMER0_IRQN;
const TIMER1_NVIC_BIT: u32 = 1 << TIMER1_IRQN;

static TIMER0_IRQS: AtomicU32 = AtomicU32::new(0);

fn reg(base: usize, offset: usize) -> *mut u32 {
    (base + offset) as *mut u32
}

fn rd(base: usize, offset: usize) -> u32 {
    unsafe { read_volatile(reg(base, offset)) }
}

fn wr(base: usize, offset: usize, value: u32) {
    unsafe { write_volatile(reg(base, offset), value) }
}

fn putc(byte: u8) {
    unsafe {
        while read_volatile(USART0_STATUS) & USART_STATUS_TXBL == 0 {}
        write_volatile(USART0_TXDATA, byte as u32);
    }
}

fn puts(s: &[u8]) {
    for byte in s {
        putc(*byte);
    }
}

fn put_kv(key: &[u8], value: u32) {
    putc(b' ');
    puts(key);
    putc(b'=');
    let mut buf = [0u8; 10];
    let mut n = value;
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    puts(&buf[i..]);
}

fn spin(n: u32) {
    for _ in 0..n {
        core::hint::spin_loop();
    }
}

#[exception]
unsafe fn DefaultHandler(irqn: i16) {
    if irqn == TIMER0_IRQN {
        wr(TIMER0_BASE, ALIAS_CLR + T_IF, IF_OF);
        TIMER0_IRQS.fetch_add(1, Ordering::SeqCst);
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(
            CMU_CLKEN0,
            CMU_CLKEN0_USART0 | CMU_CLKEN0_TIMER0 | CMU_CLKEN0_TIMER1,
        );
        write_volatile(USART0_EN, 1);
        write_volatile(USART0_CLKDIV, USART0_CLKDIV_115200);
        write_volatile(USART0_CMD, USART_CMD_TXEN);
    }

    puts(b"EFR32MG24 Cortex-M33 USART0 smoke\r\n");

    // 1. Counter width. CFG is written while EN is clear, as on silicon.
    let t0 = TIMER0_BASE;
    wr(t0, T_CFG, PRESC_1MHZ << CFG_PRESC_SHIFT);
    wr(t0, T_EN, 1);
    wr(t0, T_TOP, 0xFFFF_FFFF);
    wr(t0, T_CNT, 0x1234_5678);
    let w32 = (rd(t0, T_TOP) == 0xFFFF_FFFF && rd(t0, T_CNT) == 0x1234_5678) as u32;

    // 2. The counter runs.
    wr(t0, T_CNT, 0);
    wr(t0, T_CMD, CMD_START);
    let run = (rd(t0, T_STATUS) & STATUS_RUNNING != 0) as u32;
    spin(2000);
    let cnt = (rd(t0, T_CNT) > 0) as u32;
    wr(t0, T_CMD, CMD_STOP);
    let stop = (rd(t0, T_STATUS) & STATUS_RUNNING == 0) as u32;

    // 3. Three overflow interrupts through NVIC line 4.
    wr(t0, T_TOP, 99);
    wr(t0, T_CNT, 0);
    wr(t0, ALIAS_CLR + T_IF, 0xFFFF_FFFF);
    wr(t0, T_IEN, IF_OF);
    unsafe { write_volatile(NVIC_ISER0, TIMER0_NVIC_BIT) };
    wr(t0, T_CMD, CMD_START);
    let mut guard = 0u32;
    while TIMER0_IRQS.load(Ordering::SeqCst) < 3 && guard < 200_000 {
        guard += 1;
    }
    wr(t0, T_CMD, CMD_STOP);
    wr(t0, T_IEN, 0);
    unsafe { write_volatile(NVIC_ICER0, TIMER0_NVIC_BIT) };
    let irq = TIMER0_IRQS.load(Ordering::SeqCst);

    // 4. A direct IF store keeps the flag; the CLR alias clears it.
    wr(t0, ALIAS_SET + T_IF, IF_OF);
    wr(t0, T_IF, 0xFFFF_FFFF);
    let direct_kept = (rd(t0, T_IF) & IF_OF != 0) as u32;
    wr(t0, ALIAS_CLR + T_IF, IF_OF);
    let alias_clr = (rd(t0, T_IF) & IF_OF == 0) as u32;

    // 5. TIMER1 overflow level with NVIC line 5 disabled.
    let t1 = TIMER1_BASE;
    unsafe {
        write_volatile(NVIC_ICER0, TIMER1_NVIC_BIT);
        write_volatile(NVIC_ICPR0, TIMER1_NVIC_BIT);
    }
    wr(t1, T_CFG, PRESC_1MHZ << CFG_PRESC_SHIFT);
    wr(t1, T_EN, 1);
    wr(t1, T_TOP, 9);
    wr(t1, T_IEN, 0);
    wr(t1, T_CMD, CMD_START);
    let mut guard = 0u32;
    while rd(t1, T_IF) & IF_OF == 0 && guard < 100_000 {
        guard += 1;
    }
    wr(t1, T_CMD, CMD_STOP);
    let of = (rd(t1, T_IF) & IF_OF != 0) as u32;
    let masked = (unsafe { read_volatile(NVIC_ISPR0) } & TIMER1_NVIC_BIT == 0) as u32;
    wr(t1, T_IEN, IF_OF);
    let pend = (unsafe { read_volatile(NVIC_ISPR0) } & TIMER1_NVIC_BIT != 0) as u32;
    wr(t1, ALIAS_CLR + T_IF, IF_OF);
    unsafe { write_volatile(NVIC_ICPR0, TIMER1_NVIC_BIT) };
    let rel = (unsafe { read_volatile(NVIC_ISPR0) } & TIMER1_NVIC_BIT == 0) as u32;
    wr(t1, T_IEN, 0);

    puts(b"timers");
    put_kv(b"w32", w32);
    put_kv(b"run", run);
    put_kv(b"cnt", cnt);
    put_kv(b"stop", stop);
    put_kv(b"irq", irq);
    put_kv(b"direct_kept", direct_kept);
    put_kv(b"alias_clr", alias_clr);
    put_kv(b"t1of", of);
    put_kv(b"t1masked", masked);
    put_kv(b"t1pend", pend);
    put_kv(b"t1rel", rel);
    let ok = w32 == 1
        && run == 1
        && cnt == 1
        && stop == 1
        && irq == 3
        && direct_kept == 1
        && alias_clr == 1
        && of == 1
        && masked == 1
        && pend == 1
        && rel == 1;
    puts(if ok { b" ok\r\n" } else { b" FAIL\r\n" });

    loop {
        core::hint::spin_loop();
    }
}
