#![no_std]
#![no_main]

//! Feather nRF52840 Express: send a marker over UARTE0 on D0/TX (P0.25).
//! The USB CDC path and all other board peripherals are intentionally omitted.

use core::ptr::{addr_of, read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const GPIO0_PIN_CNF_25: *mut u32 = (0x5000_0000 + 0x700 + 25 * 4) as *mut u32;

const UARTE0: u32 = 0x4000_2000;
const TASKS_STARTTX: *mut u32 = (UARTE0 + 0x008) as *mut u32;
const EVENTS_ENDTX: *mut u32 = (UARTE0 + 0x120) as *mut u32;
const ENABLE: *mut u32 = (UARTE0 + 0x500) as *mut u32;
const PSEL_TXD: *mut u32 = (UARTE0 + 0x50c) as *mut u32;
const BAUDRATE: *mut u32 = (UARTE0 + 0x524) as *mut u32;
const TXD_PTR: *mut u32 = (UARTE0 + 0x544) as *mut u32;
const TXD_MAXCNT: *mut u32 = (UARTE0 + 0x548) as *mut u32;

const TXD_PIN_P0_25: u32 = 25;
const GPIO_OUTPUT_INPUT_DISCONNECTED: u32 = 0x3;
const BAUD_115200: u32 = 0x01d6_0000;
const UART_MESSAGE_LEN: u32 = 26;

// UARTE EasyDMA reads from RAM. A mutable static is placed in the data region,
// unlike a string literal which can live in flash.
static mut UART_MESSAGE: [u8; UART_MESSAGE_LEN as usize] = *b"FEATHER_NRF52840_UART_OK\r\n";

#[entry]
fn main() -> ! {
    unsafe {
        // D0/TX = P0.25. Configure the pad before assigning it to UARTE0.
        write_volatile(GPIO0_PIN_CNF_25, GPIO_OUTPUT_INPUT_DISCONNECTED);
        write_volatile(PSEL_TXD, TXD_PIN_P0_25);
        write_volatile(BAUDRATE, BAUD_115200);
        write_volatile(ENABLE, 8); // UARTE mode

        write_volatile(EVENTS_ENDTX, 0);
        write_volatile(TXD_PTR, addr_of!(UART_MESSAGE) as u32);
        write_volatile(TXD_MAXCNT, UART_MESSAGE_LEN);
        write_volatile(TASKS_STARTTX, 1);
        while read_volatile(EVENTS_ENDTX) == 0 {
            core::hint::spin_loop();
        }
    }

    loop {
        core::hint::spin_loop();
    }
}
