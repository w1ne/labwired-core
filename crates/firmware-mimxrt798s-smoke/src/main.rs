#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, Ordering};
use cortex_m_rt::{entry, exception};
use panic_halt as _;

const RESULT: *mut u32 = 0x2000_0000 as *mut u32;
const PASS: *mut u32 = 0x2000_0004 as *mut u32;

const FLEXCOMM0: usize = 0x4011_0000;
const LPUART_VERID: usize = 0x00;
const LPUART_PARAM: usize = 0x04;
const LPUART_BAUD: usize = 0x10;
const LPUART_STAT: usize = 0x14;
const LPUART_CTRL: usize = 0x18;
const LPUART_DATA: usize = 0x1C;
const LPFC_ISTAT: usize = 0xFF4;
const LPFC_PSELID: usize = 0xFF8;
const PSELID_ID_PRESENT: u32 = 0x0010_3070;
const PSELID_UART: u32 = 1;
const PSELID_LOCK: u32 = 1 << 3;
const STAT_TDRE: u32 = 1 << 23;
const STAT_TC: u32 = 1 << 22;
const CTRL_TIE_TE: u32 = (1 << 23) | (1 << 19);

// OS event timer, CPU0 non-secure view (MIMXRT798S_cm33_core0_COMMON.h:
// OSTIMER_CPU0_BASE_NS 0x40207000, OS_EVENT_IRQn 34; PERI_OSTIMER.h register
// layout). Count and match are Gray code (fsl_ostimer.c). Renode clocks it at
// 1 MHz; the core runs at 237.5 MHz.
const OSTIMER: usize = 0x4020_7000;
const OS_EVTIMERL: usize = 0x00;
const OS_EVTIMERH: usize = 0x04;
const OS_MATCH_L: usize = 0x10;
const OS_MATCH_H: usize = 0x14;
const OS_EVENT_CTRL: usize = 0x1C;
const OS_INTRFLAG: u32 = 1 << 0;
const OS_INTENA: u32 = 1 << 1;
const OS_EVENT_IRQN: i16 = 34;
const NVIC_ISER1: *mut u32 = 0xE000_E104 as *mut u32;
const NVIC_ICER1: *mut u32 = 0xE000_E184 as *mut u32;
const NVIC_ISPR1: *mut u32 = 0xE000_E204 as *mut u32;
const NVIC_ICPR1: *mut u32 = 0xE000_E284 as *mut u32;
const OS_NVIC_BIT: u32 = 1 << (OS_EVENT_IRQN - 32);

static OS_IRQS: AtomicU32 = AtomicU32::new(0);
static OS_IRQ_CTRL: AtomicU32 = AtomicU32::new(0);

#[exception]
unsafe fn DefaultHandler(irqn: i16) {
    if irqn == OS_EVENT_IRQN {
        OS_IRQ_CTRL.store(os_rd(OS_EVENT_CTRL), Ordering::SeqCst);
        // W1C the flag and drop INTENA, as OSTIMER_HandleIRQ does.
        os_wr(OS_EVENT_CTRL, OS_INTRFLAG);
        OS_IRQS.fetch_add(1, Ordering::SeqCst);
    }
}

fn os_rd(off: usize) -> u32 {
    unsafe { read_volatile((OSTIMER + off) as *const u32) }
}

fn os_wr(off: usize, value: u32) {
    unsafe { write_volatile((OSTIMER + off) as *mut u32, value) }
}

fn gray_to_bin(mut g: u64) -> u64 {
    let mut v = 0;
    while g != 0 {
        v ^= g;
        g >>= 1;
    }
    v
}

/// EVTIMERL then EVTIMERH (the L read latches H), decoded to binary.
fn os_now() -> u64 {
    let lo = os_rd(OS_EVTIMERL) as u64;
    let hi = os_rd(OS_EVTIMERH) as u64;
    gray_to_bin((hi << 32) | lo)
}

/// Arm a match `ticks` after now; MATCH_H is written last, as the SDK does.
fn os_arm(ticks: u64) -> u64 {
    let target = os_now() + ticks;
    let gray = target ^ (target >> 1);
    os_wr(OS_MATCH_L, gray as u32);
    os_wr(OS_MATCH_H, (gray >> 32) as u32);
    target
}

fn os_wait_flag() -> bool {
    let mut guard = 0u32;
    while os_rd(OS_EVENT_CTRL) & OS_INTRFLAG == 0 {
        guard += 1;
        if guard > 2_000_000 {
            return false;
        }
    }
    true
}

fn rd(off: usize) -> u32 {
    unsafe { read_volatile((FLEXCOMM0 + off) as *const u32) }
}

fn wr(off: usize, value: u32) {
    unsafe { write_volatile((FLEXCOMM0 + off) as *mut u32, value) }
}

fn putc(b: u8) {
    while rd(LPUART_STAT) & STAT_TDRE == 0 {}
    wr(LPUART_DATA, b as u32);
}

fn puts(s: &str) {
    for b in s.bytes() {
        putc(b);
    }
}

fn hex(v: u32) {
    for i in (0..8).rev() {
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

    let pselid_reset = rd(LPFC_PSELID);
    wr(LPFC_PSELID, PSELID_UART);
    let pselid_uart = rd(LPFC_PSELID);
    wr(LPFC_PSELID, PSELID_UART | PSELID_LOCK);
    wr(LPFC_PSELID, 0);
    let pselid_locked = rd(LPFC_PSELID);

    // 80 MHz functional clock, OSR=16, SBR=26; the smoke checks TX and
    // register behavior, not a board-level baud-rate claim.
    wr(LPUART_BAUD, (15 << 24) | 26);
    wr(LPUART_CTRL, CTRL_TIE_TE);
    let istat = rd(LPFC_ISTAT);
    puts("MIMXRT798S LP_FLEXCOMM0 LPUART SMOKE\r\n");

    let verid = rd(LPUART_VERID);
    let param = rd(LPUART_PARAM);
    let baud = rd(LPUART_BAUD);
    let ctrl = rd(LPUART_CTRL);
    let mut pass = 0u32;
    if pselid_reset == PSELID_ID_PRESENT
        && pselid_uart == (PSELID_ID_PRESENT | PSELID_UART)
        && pselid_locked == (PSELID_ID_PRESENT | PSELID_UART | PSELID_LOCK)
    {
        pass |= 1;
    }
    if verid == 0x0401_0003 && param == 0x0202 {
        pass |= 1 << 1;
    }
    if baud == ((15 << 24) | 26) && ctrl == CTRL_TIE_TE && istat & 1 != 0 {
        pass |= 1 << 2;
    }
    if product == 17 * 19 {
        pass |= 1 << 3;
    }

    puts("flexcomm0 pselid=");
    hex(pselid_reset);
    puts("->");
    hex(pselid_uart);
    puts("->");
    hex(pselid_locked);
    puts("\r\nlpuart0 verid=");
    hex(verid);
    puts(" param=");
    hex(param);
    puts(" baud=");
    hex(baud);
    puts(" ctrl=");
    hex(ctrl);
    puts(" istat=");
    hex(istat);

    // OSTIMER 1: the count advances in Gray code at 1 MHz.
    let reset_ctrl = os_rd(OS_EVENT_CTRL);
    let t0 = os_now();
    for _ in 0..20_000 {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
    let t1 = os_now();
    let run = (t1 > t0) as u32;
    // OSTIMER 2: polled match sets INTRFLAG at or after the target; W1C clears.
    let target = os_arm(25);
    let poll_ok = os_wait_flag();
    let poll = (poll_ok && os_now() >= target) as u32;
    os_wr(OS_EVENT_CTRL, OS_INTRFLAG);
    let w1c = (os_rd(OS_EVENT_CTRL) & OS_INTRFLAG == 0) as u32;
    // OSTIMER 3: INTENA + NVIC line 34 deliver one OS_EVENT interrupt.
    unsafe {
        write_volatile(NVIC_ICPR1, OS_NVIC_BIT);
        write_volatile(NVIC_ISER1, OS_NVIC_BIT);
    }
    os_wr(OS_EVENT_CTRL, OS_INTENA);
    os_arm(25);
    let mut guard = 0u32;
    while OS_IRQS.load(Ordering::SeqCst) < 1 && guard < 2_000_000 {
        guard += 1;
    }
    let irq = OS_IRQS.load(Ordering::SeqCst);
    let isr_ctrl = OS_IRQ_CTRL.load(Ordering::SeqCst);
    // OSTIMER 4: with INTENA clear the flag sets but NVIC line 34 stays idle.
    os_arm(25);
    let flag4 = os_wait_flag();
    let masked = (flag4
        && unsafe { read_volatile(NVIC_ISPR1) } & OS_NVIC_BIT == 0
        && OS_IRQS.load(Ordering::SeqCst) == 1) as u32;
    // OSTIMER 5: setting INTENA over the set flag raises the IRQ at once.
    os_wr(OS_EVENT_CTRL, OS_INTENA);
    let mut guard = 0u32;
    while OS_IRQS.load(Ordering::SeqCst) < 2 && guard < 200_000 {
        guard += 1;
    }
    let late = (OS_IRQS.load(Ordering::SeqCst) == 2
        && os_rd(OS_EVENT_CTRL) & (OS_INTRFLAG | OS_INTENA) == 0) as u32;
    unsafe { write_volatile(NVIC_ICER1, OS_NVIC_BIT) };
    if reset_ctrl == 0
        && run == 1
        && poll == 1
        && w1c == 1
        && irq == 1
        && isr_ctrl == (OS_INTRFLAG | OS_INTENA)
        && masked == 1
        && late == 1
    {
        pass |= 1 << 4;
    }
    puts("\r\nostimer ctrl=");
    hex(reset_ctrl);
    puts(" run=");
    hex(run);
    puts(" poll=");
    hex(poll);
    puts(" w1c=");
    hex(w1c);
    puts(" irq=");
    hex(irq);
    puts(" isr_ctrl=");
    hex(isr_ctrl);
    puts(" masked=");
    hex(masked);
    puts(" late=");
    hex(late);
    puts("\r\npass=");
    hex(pass);
    puts("\r\n");
    while rd(LPUART_STAT) & STAT_TC == 0 {}
    unsafe { write_volatile(PASS, 0x7000_0000 | pass) };

    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
}
