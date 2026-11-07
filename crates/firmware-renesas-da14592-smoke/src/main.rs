#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// Renode's DA14592 platform maps UART0 as a DesignWare APB UART with a
// 4-byte register stride: THR@0x00 and LSR@0x14.
const UART0_THR: *mut u32 = 0x5002_0000 as *mut u32;
const UART0_LSR: *const u32 = 0x5002_0014 as *const u32;
const RESULT: *mut u32 = 0x2000_0000 as *mut u32;
const LSR_THRE: u32 = 1 << 5;

fn putc(byte: u8) {
    unsafe {
        let mut remaining = 10_000u32;
        while read_volatile(UART0_LSR) & LSR_THRE == 0 && remaining > 0 {
            remaining -= 1;
        }
        write_volatile(UART0_THR, byte as u32);
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

    for byte in b"DA14592 M33 UART0 smoke; mul=323\r\n" {
        putc(*byte);
    }

    loop {
        core::hint::spin_loop();
    }
}
