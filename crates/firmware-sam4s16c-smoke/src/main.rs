#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const PMC_PCER0: *mut u32 = 0x400E_0410 as *mut u32;
const PMC_SR: *const u32 = 0x400E_0468 as *const u32;
const PMC_SR_MCKRDY: u32 = 1 << 3;

const UART0_CR: *mut u32 = 0x400E_0600 as *mut u32;
const UART0_MR: *mut u32 = 0x400E_0604 as *mut u32;
const UART0_SR: *const u32 = 0x400E_0614 as *const u32;
const UART0_THR: *mut u32 = 0x400E_061C as *mut u32;
const UART0_BRGR: *mut u32 = 0x400E_0620 as *mut u32;

const UART_SR_TXRDY: u32 = 1 << 1;
const UART_CR_RXEN_TXEN: u32 = (1 << 4) | (1 << 6);
const UART_MR_8N1: u32 = 0x8C0;

fn putc(byte: u8) {
    unsafe {
        while read_volatile(UART0_SR) & UART_SR_TXRDY == 0 {}
        write_volatile(UART0_THR, byte as u32);
    }
}

fn puts(text: &str) {
    for byte in text.bytes() {
        putc(byte);
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(PMC_PCER0, 1 << 8);
        while read_volatile(PMC_SR) & PMC_SR_MCKRDY == 0 {}

        write_volatile(UART0_MR, UART_MR_8N1);
        write_volatile(UART0_BRGR, 65);
        write_volatile(UART0_CR, UART_CR_RXEN_TXEN);
    }

    puts("SAM4S16C UART0 smoke\n");

    loop {
        core::hint::spin_loop();
    }
}
