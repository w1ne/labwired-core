#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// LabWired's NPCX UART model exposes UTBUF at +0x00 and the transmit-ready
// TBE flag at UICTRL +0x04. Clock and baud configuration are outside this
// smoke's modeled surface.
const UART1_BASE: usize = 0x400E_0000;
const UTBUF: *mut u8 = UART1_BASE as *mut u8;
const UICTRL: *const u8 = (UART1_BASE + 0x04) as *const u8;
const TBE: u8 = 1 << 0;

fn putc(byte: u8) {
    unsafe {
        while read_volatile(UICTRL) & TBE == 0 {}
        write_volatile(UTBUF, byte);
    }
}

#[entry]
fn main() -> ! {
    for byte in b"NPCX9 UART1 smoke\r\n" {
        putc(*byte);
    }

    loop {
        core::hint::spin_loop();
    }
}
