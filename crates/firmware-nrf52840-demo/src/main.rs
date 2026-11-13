#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// nRF52840 UARTE0. TX uses EasyDMA rather than a byte-at-a-time register.
const UART0_BASE: u32 = 0x4000_2000;
const UART0_TASKS_STARTTX: *mut u32 = (UART0_BASE + 0x008) as *mut u32;
const UART0_EVENTS_ENDTX: *mut u32 = (UART0_BASE + 0x120) as *mut u32;
const UART0_ENABLE: *mut u32 = (UART0_BASE + 0x500) as *mut u32;
const UART0_TXD_PTR: *mut u32 = (UART0_BASE + 0x544) as *mut u32;
const UART0_TXD_MAXCNT: *mut u32 = (UART0_BASE + 0x548) as *mut u32;
const UARTE_ENABLE: u32 = 8; // 8 = UARTE (EasyDMA); 4 is the legacy UART.

const GPIO0_OUTSET: *mut u32 = 0x5000_0508 as *mut u32;
const GPIO0_OUTCLR: *mut u32 = 0x5000_050C as *mut u32;
const GPIO0_DIRSET: *mut u32 = 0x5000_0518 as *mut u32;
const LED0: u32 = 1 << 13;

#[entry]
fn main() -> ! {
    // EasyDMA reads the banner from RAM, as required by the UARTE data path.
    let msg = *b"NRF52840_SMOKE_OK\n";

    unsafe {
        write_volatile(UART0_ENABLE, UARTE_ENABLE);
        write_volatile(GPIO0_DIRSET, LED0);
        write_volatile(GPIO0_OUTSET, LED0);
    }

    loop {
        unsafe {
            write_volatile(GPIO0_OUTCLR, LED0);

            // EasyDMA console TX.
            write_volatile(UART0_EVENTS_ENDTX, 0);
            write_volatile(UART0_TXD_PTR, msg.as_ptr() as u32);
            write_volatile(UART0_TXD_MAXCNT, msg.len() as u32);
            write_volatile(UART0_TASKS_STARTTX, 1);
            while read_volatile(UART0_EVENTS_ENDTX) == 0 {}

            write_volatile(GPIO0_OUTSET, LED0);

            // Keep a short interval between GPIO transitions without relying
            // on a core intrinsic unavailable in this bare-metal build.
            for _ in 0..1000u32 {
                let _ = read_volatile(UART0_ENABLE);
            }
        }
    }
}
