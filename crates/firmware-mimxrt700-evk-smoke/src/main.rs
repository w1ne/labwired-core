#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use core::sync::atomic::{AtomicU32, Ordering};
use cortex_m_rt::{entry, exception};
use panic_halt as _;

const RESULT: *mut u32 = 0x2000_0008 as *mut u32;
const PASS: *mut u32 = 0x2000_000C as *mut u32;

const FLEXCOMM0: usize = 0x4011_0000;
const NVIC_ISER0: usize = 0xE000_E100;
const NVIC_ICER0: usize = 0xE000_E180;
const NVIC_ISPR0: usize = 0xE000_E200;
const NVIC_ICPR0: usize = 0xE000_E280;
const FLEXCOMM0_IRQ_BIT: u32 = 1 << 7;
const FLEXCOMM0_IRQN: i16 = 7;
const LPUART_VERID: usize = 0x00;
const LPUART_PARAM: usize = 0x04;
const LPUART_BAUD: usize = 0x10;
const LPUART_STAT: usize = 0x14;
const LPUART_CTRL: usize = 0x18;
const LPUART_DATA: usize = 0x1C;
const LPUART_FIFO: usize = 0x28;
const LPUART_WATER: usize = 0x2C;
const LPFC_ISTAT: usize = 0xFF4;
const LPFC_PSELID: usize = 0xFF8;
const PSELID_ID_PRESENT: u32 = 0x0010_3070;
const PSELID_UART: u32 = 1;
const PSELID_LOCK: u32 = 1 << 3;
const STAT_TDRE: u32 = 1 << 23;
const STAT_TC: u32 = 1 << 22;
const STAT_RDRF: u32 = 1 << 21;
const CTRL_TIE_TE: u32 = (1 << 23) | (1 << 19);
const CTRL_TE: u32 = 1 << 19;
const CTRL_RE: u32 = 1 << 18;
const CTRL_RIE: u32 = 1 << 21;
const FIFO_RXFE: u32 = 1 << 3;
const WATER_RXCOUNT_SHIFT: u32 = 24;
const ISTAT_UARTRX: u32 = 1 << 1;

static IRQ_ENTRY: AtomicU32 = AtomicU32::new(0);

fn rd(off: usize) -> u32 {
    unsafe { read_volatile((FLEXCOMM0 + off) as *const u32) }
}

fn wr(off: usize, value: u32) {
    unsafe { write_volatile((FLEXCOMM0 + off) as *mut u32, value) }
}

fn nvic_irq_pending() -> bool {
    unsafe { read_volatile(NVIC_ISPR0 as *const u32) & FLEXCOMM0_IRQ_BIT != 0 }
}

fn clear_nvic_irq_pending() {
    unsafe { write_volatile(NVIC_ICPR0 as *mut u32, FLEXCOMM0_IRQ_BIT) }
}

fn enable_nvic_irq() {
    unsafe { write_volatile(NVIC_ISER0 as *mut u32, FLEXCOMM0_IRQ_BIT) }
}

fn wait_nvic_irq_pending(expected: bool) -> bool {
    for _ in 0..10_000 {
        if nvic_irq_pending() == expected {
            return true;
        }
    }
    false
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

/// Exercise the imported LPUART RX FIFO, LP_FLEXCOMM receive status, CPU0
/// NVIC pending line, and exception delivery (FLEXCOMM0 IRQ 7).
fn smoke_lpuart_rx() -> bool {
    // Leave TE on for the later console output, but disable TIE so the shared
    // FLEXCOMM line reflects the receive interrupt only. Clear the earlier TX
    // pending bit before checking the receive gates.
    wr(LPUART_CTRL, CTRL_TE);
    clear_nvic_irq_pending();
    let irq_isolated = wait_nvic_irq_pending(false);
    let receiver_off =
        rd(LPUART_STAT) & STAT_RDRF == 0 && rd(LPFC_ISTAT) & ISTAT_UARTRX == 0 && irq_isolated;

    // Two injected bytes need the modeled four-entry FIFO enabled. Keep RIE
    // clear while receiving so data-ready and interrupt status are distinct.
    wr(LPUART_FIFO, rd(LPUART_FIFO) | FIFO_RXFE);
    wr(LPUART_CTRL, CTRL_TE | CTRL_RE);
    let mut queued = false;
    for _ in 0..1_000_000 {
        if rd(LPUART_WATER) >> WATER_RXCOUNT_SHIFT >= 2 {
            queued = true;
            break;
        }
    }

    let rx_masked = queued
        && rd(LPUART_STAT) & STAT_RDRF != 0
        && rd(LPFC_ISTAT) & ISTAT_UARTRX == 0
        && wait_nvic_irq_pending(false);
    wr(LPUART_CTRL, CTRL_TE | CTRL_RE | CTRL_RIE);
    let rx_asserted = rd(LPFC_ISTAT) & ISTAT_UARTRX != 0 && wait_nvic_irq_pending(true);
    clear_nvic_irq_pending();
    let rx_repend = wait_nvic_irq_pending(true);

    // Enable the already-pending external IRQ and require the Cortex-M
    // exception path to call DefaultHandler. The handler disables IRQ 7
    // before returning so the still-asserted RX level cannot retrigger it.
    enable_nvic_irq();
    let irq_handler_entered =
        (0..10_000).any(|_| IRQ_ENTRY.load(Ordering::Relaxed) == FLEXCOMM0_IRQN as u32 + 1);

    let first = rd(LPUART_DATA) & 0xFF;
    let second_still_queued = rd(LPUART_STAT) & STAT_RDRF != 0;
    wr(LPUART_CTRL, CTRL_TE | CTRL_RE);
    let rx_masked_with_data = second_still_queued
        && rd(LPUART_STAT) & STAT_RDRF != 0
        && rd(LPFC_ISTAT) & ISTAT_UARTRX == 0;
    clear_nvic_irq_pending();
    let irq_masked_released = rx_masked_with_data && wait_nvic_irq_pending(false);
    wr(LPUART_CTRL, CTRL_TE | CTRL_RE | CTRL_RIE);
    let rx_unmasked = rd(LPFC_ISTAT) & ISTAT_UARTRX != 0 && wait_nvic_irq_pending(true);

    let second = rd(LPUART_DATA) & 0xFF;
    clear_nvic_irq_pending();
    let irq_released = wait_nvic_irq_pending(false);
    let released =
        rd(LPUART_STAT) & STAT_RDRF == 0 && rd(LPFC_ISTAT) & ISTAT_UARTRX == 0 && irq_released;
    let ok = receiver_off
        && queued
        && rx_masked
        && rx_asserted
        && rx_repend
        && irq_handler_entered
        && first == b'R' as u32
        && second_still_queued
        && rx_masked_with_data
        && irq_masked_released
        && rx_unmasked
        && second == b'S' as u32
        && released;

    puts("rx lpuart0 re_off=");
    putc(if receiver_off { b'1' } else { b'0' });
    puts(" queued=");
    putc(if queued { b'2' } else { b'0' });
    puts(" masked=");
    putc(if rx_masked { b'1' } else { b'0' });
    puts(" asserted=");
    putc(if rx_asserted { b'1' } else { b'0' });
    puts(" masked_data=");
    putc(if rx_masked_with_data { b'1' } else { b'0' });
    puts(" unmasked=");
    putc(if rx_unmasked { b'1' } else { b'0' });
    puts(" data=");
    hex(first);
    putc(b',');
    hex(second);
    puts(" released=");
    putc(if released { b'1' } else { b'0' });
    puts(" irq=");
    putc(if rx_asserted { b'1' } else { b'0' });
    putc(if rx_repend { b'1' } else { b'0' });
    putc(if irq_masked_released { b'1' } else { b'0' });
    putc(if rx_unmasked { b'1' } else { b'0' });
    putc(if irq_released { b'1' } else { b'0' });
    puts(" handler=");
    putc(if irq_handler_entered { b'1' } else { b'0' });
    puts(if ok { " ok\r\n" } else { " FAIL\r\n" });
    ok
}

#[entry]
fn main() -> ! {
    let product: u32;
    unsafe {
        core::arch::asm!(
            "mul {product}, {product}, {right}",
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

    // 80 MHz functional clock, OSR=16, SBR=26; the smoke checks TX/RX and
    // register behavior, not a board-level baud-rate claim.
    wr(LPUART_BAUD, (15 << 24) | 26);
    wr(LPUART_CTRL, CTRL_TIE_TE);
    let istat = rd(LPFC_ISTAT);
    puts("MIMXRT700 LP_FLEXCOMM0 LPUART SMOKE\r\n");

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
    if smoke_lpuart_rx() {
        pass |= 1 << 4;
    }
    if IRQ_ENTRY.load(Ordering::Relaxed) == FLEXCOMM0_IRQN as u32 + 1 {
        pass |= 1 << 5;
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
    puts("\r\npass=");
    hex(pass);
    puts("\r\n");
    while rd(LPUART_STAT) & STAT_TC == 0 {}
    unsafe { write_volatile(PASS, 0x7000_0000 | pass) };

    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
}

#[exception]
unsafe fn DefaultHandler(irqn: i16) {
    if irqn == FLEXCOMM0_IRQN {
        IRQ_ENTRY.store(irqn as u32 + 1, Ordering::Relaxed);
        unsafe { write_volatile(NVIC_ICER0 as *mut u32, FLEXCOMM0_IRQ_BIT) };
    }
}
