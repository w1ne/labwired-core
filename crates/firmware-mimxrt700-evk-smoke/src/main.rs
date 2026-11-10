#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
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
const CTRL_RE: u32 = 1 << 18;
const CTRL_RIE: u32 = 1 << 21;
const FIFO_RXFE: u32 = 1 << 3;
const WATER_RXCOUNT_SHIFT: u32 = 24;
const ISTAT_UARTRX: u32 = 1 << 1;

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

/// Exercise the imported LPUART RX FIFO and LP_FLEXCOMM receive status.
/// This checks ISTAT only; the imported profile does not route FLEXCOMM0 to
/// an NVIC input.
fn smoke_lpuart_rx() -> bool {
    let receiver_off = rd(LPUART_STAT) & STAT_RDRF == 0 && rd(LPFC_ISTAT) & ISTAT_UARTRX == 0;

    // Two injected bytes need the modeled four-entry FIFO enabled. Keep RIE
    // clear while receiving so data-ready and interrupt status are distinct.
    wr(LPUART_FIFO, rd(LPUART_FIFO) | FIFO_RXFE);
    wr(LPUART_CTRL, CTRL_TIE_TE | CTRL_RE);
    let mut queued = false;
    for _ in 0..1_000_000 {
        if rd(LPUART_WATER) >> WATER_RXCOUNT_SHIFT >= 2 {
            queued = true;
            break;
        }
    }

    let rx_masked =
        queued && rd(LPUART_STAT) & STAT_RDRF != 0 && rd(LPFC_ISTAT) & ISTAT_UARTRX == 0;
    wr(LPUART_CTRL, CTRL_TIE_TE | CTRL_RE | CTRL_RIE);
    let rx_asserted = rd(LPFC_ISTAT) & ISTAT_UARTRX != 0;

    let first = rd(LPUART_DATA) & 0xFF;
    let second_still_queued = rd(LPUART_STAT) & STAT_RDRF != 0;
    wr(LPUART_CTRL, CTRL_TIE_TE | CTRL_RE);
    let rx_masked_with_data = second_still_queued
        && rd(LPUART_STAT) & STAT_RDRF != 0
        && rd(LPFC_ISTAT) & ISTAT_UARTRX == 0;
    wr(LPUART_CTRL, CTRL_TIE_TE | CTRL_RE | CTRL_RIE);
    let rx_unmasked = rd(LPFC_ISTAT) & ISTAT_UARTRX != 0;

    let second = rd(LPUART_DATA) & 0xFF;
    let released = rd(LPUART_STAT) & STAT_RDRF == 0 && rd(LPFC_ISTAT) & ISTAT_UARTRX == 0;
    let ok = receiver_off
        && queued
        && rx_masked
        && rx_asserted
        && first == b'R' as u32
        && second_still_queued
        && rx_masked_with_data
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
    puts(if ok { " ok\r\n" } else { " FAIL\r\n" });
    ok
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
