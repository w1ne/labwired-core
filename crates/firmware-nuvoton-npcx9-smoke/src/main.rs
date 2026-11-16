// NPCX9 Renode-onboarding smoke. Keeps the UART1 banner, then exercises the
// internal timers: ITIM32_1..6 at 0x400B0000 + n * 0x2000 (NVIC 28, 27, 17,
// 43, 44, 45) and ITIM64 at 0x400BE000 (NVIC 58), as in Renode's
// nuvoton_npcx9.repl. Register layout from Zephyr reg_def.h (itim32_reg /
// itim64_reg): ITPRE byte +0x01, ITCTS byte +0x04, ITCNT +0x08, ITCNT64H +0x0C.
//
// 1. ITPRE and ITCNT hold their values while ITEN is clear.
// 2. 15 MHz APB2 / (ITPRE + 1) = 1 MHz: ITEN starts the down-count; clearing
//    ITEN stops it, restores the load value and clears TO_STS.
// 3. ITCNT = 99 with TO_IE: three timeouts reach the core on NVIC 28; the
//    handler clears TO_STS (write 1), as Zephyr's npcx_itim_evt_isr does.
// 4. ITIM32_2 with NVIC 27 disabled: TO_STS without TO_IE leaves the line
//    idle, setting TO_IE pends it, clearing TO_STS lets ICPR release it.
// 5. ITIM32_2..6 and ITIM64 each take one timeout IRQ on their own line.
// 6. ITIM64 holds a 64-bit load and borrows across the word boundary.
// 7. CKSEL selects the 32768 Hz LFCLK: the count moves ~460x slower.
//
// Then the GPIO ports: GPIO0..GPIOF at 0x40081000 + n * 0x2000 (Renode
// GPIOPort.NPCX_GPIO), byte registers from Zephyr reg_def.h gpio_reg.
// 8. Each of the 16 ports holds PDIR / PTYPE / PPULL / PPUD / PENVDD /
//    PLOCK_CTL values.
// 9. PDOUT written before PDIR (the gpio_npcx.c order) holds while the pin is
//    an input and reaches PDIN once PDIR makes it an output.
// 10. Pull-up / pull-down inputs read 1 / 0 on PDIN.
// 11. An open-drain output with pull-up reads 1 when PDOUT releases the pad
//     and 0 when PDOUT drives it low.
// 12. A set PLOCK_CTL bit keeps its PDOUT bit.
//
// Then UART2..4 at 0x400E2000 / 0x400E4000 / 0x400E6000 (Renode
// UART.NPCX_UART). The smoke script injects 'b', 'c', 'd' into cr_uart2..4.
// 13. Each port reads TBE set; RBF is set only on its own port (UART1 stays
//     empty), URBUF returns its own byte and the read clears RBF.
// 14. Each port transmits its own "cr_uartN tx" line through its UTBUF.
//
// Bare-register, no HAL.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, Ordering};
use cortex_m_rt::{entry, exception};
use panic_halt as _;

// LabWired's NPCX UART model exposes UTBUF at +0x00 and the transmit-ready
// TBE flag at UICTRL +0x04. Clock and baud configuration are outside this
// smoke's modeled surface.
const UART1_BASE: usize = 0x400E_0000;
const UTBUF: *mut u8 = UART1_BASE as *mut u8;
const UICTRL: *const u8 = (UART1_BASE + 0x04) as *const u8;
const TBE: u8 = 1 << 0;
const RBF: u8 = 1 << 1;
const URBUF: usize = 0x02;
const UICTRL_OFF: usize = 0x04;
// (base, injected byte, name): UART2..4.
const UARTS: [(usize, u8, &[u8]); 3] = [
    (0x400E_2000, b'b', b"cr_uart2"),
    (0x400E_4000, b'c', b"cr_uart3"),
    (0x400E_6000, b'd', b"cr_uart4"),
];

const ITPRE: usize = 0x01;
const ITCTS: usize = 0x04;
const ITCNT: usize = 0x08;
const ITCNT64H: usize = 0x0C;
const TO_STS: u8 = 1 << 0;
const TO_IE: u8 = 1 << 2;
const CKSEL: u8 = 1 << 4;
const ITEN: u8 = 1 << 7;
// 15 MHz APB2 / 15 = 1 MHz.
const PRE_1MHZ: u8 = 14;

// (base, NVIC line): ITIM32_1..6, then ITIM64.
const TIMERS: [(usize, u32); 7] = [
    (0x400B_0000, 28),
    (0x400B_2000, 27),
    (0x400B_4000, 17),
    (0x400B_6000, 43),
    (0x400B_8000, 44),
    (0x400B_A000, 45),
    (0x400B_E000, 58),
];
const ITIM32_1: usize = TIMERS[0].0;
const ITIM32_2: usize = TIMERS[1].0;
const ITIM64: usize = TIMERS[6].0;

const GPIO0: usize = 0x4008_1000;
const GPIO_STRIDE: usize = 0x2000;
const PDOUT: usize = 0x00;
const PDIN: usize = 0x01;
const PDIR: usize = 0x02;
const PPULL: usize = 0x03;
const PPUD: usize = 0x04;
const PENVDD: usize = 0x05;
const PTYPE: usize = 0x06;
const PLOCK_CTL: usize = 0x07;

const NVIC_ISER: usize = 0xE000_E100;
const NVIC_ICER: usize = 0xE000_E180;
const NVIC_ISPR: usize = 0xE000_E200;
const NVIC_ICPR: usize = 0xE000_E280;

static ITIM1_IRQS: AtomicU32 = AtomicU32::new(0);
/// Bit n set: TIMERS[n] took its IRQ.
static SEEN: AtomicU32 = AtomicU32::new(0);

fn rd8(base: usize, offset: usize) -> u8 {
    unsafe { read_volatile((base + offset) as *const u8) }
}

fn wr8(base: usize, offset: usize, value: u8) {
    unsafe { write_volatile((base + offset) as *mut u8, value) }
}

fn rd32(base: usize, offset: usize) -> u32 {
    unsafe { read_volatile((base + offset) as *const u32) }
}

fn wr32(base: usize, offset: usize, value: u32) {
    unsafe { write_volatile((base + offset) as *mut u32, value) }
}

fn nvic(reg: usize, line: u32) -> (usize, u32) {
    (reg + 4 * (line as usize / 32), 1 << (line % 32))
}

fn nvic_set(reg: usize, line: u32) {
    let (addr, bit) = nvic(reg, line);
    unsafe { write_volatile(addr as *mut u32, bit) }
}

fn nvic_pending(line: u32) -> bool {
    let (addr, bit) = nvic(NVIC_ISPR, line);
    unsafe { read_volatile(addr as *const u32) & bit != 0 }
}

fn putc(byte: u8) {
    unsafe {
        while read_volatile(UICTRL) & TBE == 0 {}
        write_volatile(UTBUF, byte);
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

fn uart_puts(base: usize, s: &[u8]) {
    for byte in s {
        while rd8(base, UICTRL_OFF) & TBE == 0 {}
        wr8(base, 0x00, *byte);
    }
}

fn spin(n: u32) {
    for _ in 0..n {
        core::hint::spin_loop();
    }
}

#[exception]
unsafe fn DefaultHandler(irqn: i16) {
    for (n, &(base, line)) in TIMERS.iter().enumerate() {
        if irqn as u32 == line {
            wr8(base, ITCTS, rd8(base, ITCTS) | TO_STS);
            if n == 0 {
                ITIM1_IRQS.fetch_add(1, Ordering::SeqCst);
            }
            SEEN.fetch_or(1 << n, Ordering::SeqCst);
        }
    }
}

#[entry]
fn main() -> ! {
    puts(b"NPCX9 UART1 smoke\r\n");

    // 1. Registers hold while disabled.
    let t1 = ITIM32_1;
    wr8(t1, ITPRE, PRE_1MHZ);
    wr32(t1, ITCNT, 0x1234_5678);
    let regs = (rd8(t1, ITPRE) == PRE_1MHZ && rd32(t1, ITCNT) == 0x1234_5678) as u32;

    // 2. Down-count, stop.
    wr32(t1, ITCNT, 0xFFFF_FFFF);
    wr8(t1, ITCTS, ITEN);
    let en = (rd8(t1, ITCTS) & ITEN != 0) as u32;
    spin(2000);
    let run = (rd32(t1, ITCNT) < 0xFFFF_FFFF) as u32;
    wr8(t1, ITCTS, 0);
    let stop = (rd32(t1, ITCNT) == 0xFFFF_FFFF && rd8(t1, ITCTS) == 0) as u32;

    // 3. Three timeout IRQs on NVIC 28.
    wr32(t1, ITCNT, 99);
    nvic_set(NVIC_ISER, TIMERS[0].1);
    wr8(t1, ITCTS, ITEN | TO_IE);
    let mut guard = 0u32;
    while ITIM1_IRQS.load(Ordering::SeqCst) < 3 && guard < 200_000 {
        guard += 1;
    }
    wr8(t1, ITCTS, 0);
    nvic_set(NVIC_ICER, TIMERS[0].1);
    let irq = ITIM1_IRQS.load(Ordering::SeqCst);

    // 4. Level on ITIM32_2 with NVIC 27 disabled.
    let t2 = ITIM32_2;
    let l2 = TIMERS[1].1;
    nvic_set(NVIC_ICER, l2);
    nvic_set(NVIC_ICPR, l2);
    wr8(t2, ITPRE, PRE_1MHZ);
    wr32(t2, ITCNT, 9);
    wr8(t2, ITCTS, ITEN);
    let mut guard = 0u32;
    while rd8(t2, ITCTS) & TO_STS == 0 && guard < 100_000 {
        guard += 1;
    }
    wr8(t2, ITCTS, 0);
    // Disabling clears TO_STS; re-run once to leave it set while stopped.
    let sts_cleared = (rd8(t2, ITCTS) & TO_STS == 0) as u32;
    wr8(t2, ITCTS, ITEN);
    let mut guard = 0u32;
    while rd8(t2, ITCTS) & TO_STS == 0 && guard < 100_000 {
        guard += 1;
    }
    let sts = (rd8(t2, ITCTS) & TO_STS != 0) as u32;
    let masked = (!nvic_pending(l2)) as u32;
    wr8(t2, ITCTS, ITEN | TO_IE);
    let pend = nvic_pending(l2) as u32;
    wr8(t2, ITCTS, TO_IE | TO_STS);
    nvic_set(NVIC_ICPR, l2);
    let rel = (!nvic_pending(l2)) as u32;
    wr8(t2, ITCTS, 0);

    // 5. One IRQ on each remaining line.
    SEEN.store(0, Ordering::SeqCst);
    for (n, &(base, line)) in TIMERS.iter().enumerate().skip(1) {
        wr8(base, ITPRE, PRE_1MHZ);
        wr32(base, ITCNT, 49);
        if n == 6 {
            wr32(base, ITCNT64H, 0);
        }
        nvic_set(NVIC_ICPR, line);
        nvic_set(NVIC_ISER, line);
        wr8(base, ITCTS, ITEN | TO_IE);
        let mut guard = 0u32;
        while SEEN.load(Ordering::SeqCst) & (1 << n) == 0 && guard < 100_000 {
            guard += 1;
        }
        wr8(base, ITCTS, 0);
        nvic_set(NVIC_ICER, line);
    }
    let each = SEEN.load(Ordering::SeqCst).count_ones();

    // 6. ITIM64: 0x1_0000_0005 borrows into the high word after six ticks.
    let t64 = ITIM64;
    wr32(t64, ITCNT, 5);
    wr32(t64, ITCNT64H, 1);
    let w64 = (rd32(t64, ITCNT64H) == 1 && rd32(t64, ITCNT) == 5) as u32;
    wr8(t64, ITCTS, ITEN);
    spin(200);
    let hi = rd32(t64, ITCNT64H);
    let lo = rd32(t64, ITCNT);
    wr8(t64, ITCTS, 0);
    let borrow = (hi == 0 && lo > 0xFF00_0000) as u32;

    // 7. LFCLK: 15 MHz / 32768 Hz is ~458 cycles per tick.
    wr8(t1, ITPRE, 0);
    wr32(t1, ITCNT, 1000);
    wr8(t1, ITCTS, ITEN | CKSEL);
    spin(2000);
    let gone = 1000 - rd32(t1, ITCNT);
    wr8(t1, ITCTS, 0);
    let lfclk = (gone >= 1 && gone <= 50) as u32;

    puts(b"itim");
    put_kv(b"regs", regs);
    put_kv(b"en", en);
    put_kv(b"run", run);
    put_kv(b"stop", stop);
    put_kv(b"irq", irq);
    put_kv(b"sts_cleared", sts_cleared);
    put_kv(b"sts", sts);
    put_kv(b"masked", masked);
    put_kv(b"pend", pend);
    put_kv(b"rel", rel);
    put_kv(b"each", each);
    put_kv(b"w64", w64);
    put_kv(b"borrow", borrow);
    put_kv(b"lfclk", lfclk);
    let ok = regs == 1
        && en == 1
        && run == 1
        && stop == 1
        && irq == 3
        && sts_cleared == 1
        && sts == 1
        && masked == 1
        && pend == 1
        && rel == 1
        && each == 6
        && w64 == 1
        && borrow == 1
        && lfclk == 1;
    puts(if ok { b" ok\r\n" } else { b" FAIL\r\n" });

    // 8. All 16 ports exist and hold their configuration.
    let mut ports = 0u32;
    for n in 0..16 {
        let base = GPIO0 + n * GPIO_STRIDE;
        let tag = 0xA0 | n as u8;
        let cfg = [PDIR, PPULL, PPUD, PENVDD, PTYPE, PLOCK_CTL];
        for (k, &reg) in cfg.iter().enumerate() {
            wr8(base, reg, tag ^ k as u8);
        }
        if cfg
            .iter()
            .enumerate()
            .all(|(k, &reg)| rd8(base, reg) == tag ^ k as u8)
        {
            ports += 1;
        }
        for &reg in cfg.iter().rev() {
            wr8(base, reg, 0);
        }
    }

    // 9-12 on GPIO5.
    let g = GPIO0 + 5 * GPIO_STRIDE;
    wr8(g, PDOUT, 0x05);
    let latch = (rd8(g, PDOUT) == 0x05 && rd8(g, PDIN) & 0x0F == 0) as u32;
    wr8(g, PDIR, 0x0F);
    let drive = (rd8(g, PDIN) & 0x0F == 0x05) as u32;
    wr8(g, PDIR, 0);
    wr8(g, PDOUT, 0);
    wr8(g, PPULL, 0x30);
    wr8(g, PPUD, 0x20);
    let pull = (rd8(g, PDIN) & 0x30 == 0x10) as u32;
    wr8(g, PPULL, 0x40);
    wr8(g, PPUD, 0);
    wr8(g, PTYPE, 0x40);
    wr8(g, PDOUT, 0x40);
    wr8(g, PDIR, 0x40);
    let od_high = rd8(g, PDIN) & 0x40 != 0;
    wr8(g, PDOUT, 0);
    let od_low = rd8(g, PDIN) & 0x40 == 0;
    let od = (od_high && od_low) as u32;
    wr8(g, PDOUT, 0x80);
    wr8(g, PLOCK_CTL, 0x80);
    wr8(g, PDOUT, 0x01);
    let lock = (rd8(g, PDOUT) == 0x81) as u32;
    puts(b"gpio");
    put_kv(b"ports", ports);
    put_kv(b"latch", latch);
    put_kv(b"drive", drive);
    put_kv(b"pull", pull);
    put_kv(b"od", od);
    put_kv(b"lock", lock);
    let gpio_ok = ports == 16 && latch == 1 && drive == 1 && pull == 1 && od == 1 && lock == 1;
    puts(if gpio_ok { b" ok\r\n" } else { b" FAIL\r\n" });

    // 13. Per-port TBE / RBF / URBUF.
    let mut tbe = 0;
    let mut rx = 0;
    let mut drained = 0;
    for &(base, byte, _) in UARTS.iter() {
        let ctrl = rd8(base, UICTRL_OFF);
        tbe += (ctrl & TBE != 0) as u32;
        if ctrl & RBF != 0 && rd8(base, URBUF) == byte {
            rx += 1;
        }
        drained += (rd8(base, UICTRL_OFF) & RBF == 0) as u32;
    }
    let isolated = (unsafe { read_volatile(UICTRL) } & RBF == 0) as u32;
    // 14. Per-port transmit.
    for &(base, _, name) in UARTS.iter() {
        uart_puts(base, name);
        uart_puts(base, b" tx\r\n");
    }
    puts(b"uarts");
    put_kv(b"tbe", tbe);
    put_kv(b"rx", rx);
    put_kv(b"drained", drained);
    put_kv(b"isolated", isolated);
    let uart_ok = tbe == 3 && rx == 3 && drained == 3 && isolated == 1;
    puts(if uart_ok { b" ok\r\n" } else { b" FAIL\r\n" });

    loop {
        core::hint::spin_loop();
    }
}
