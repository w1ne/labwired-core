#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

use core::arch::asm;
use panic_halt as _;
use riscv_rt::entry;

const GPIO_BASE: usize = 0x6000_4000;
const GPIO_OUT_W1TS: *mut u32 = (GPIO_BASE + 0x08) as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = (GPIO_BASE + 0x0C) as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = (GPIO_BASE + 0x24) as *mut u32;
const GPIO0_MASK: u32 = 1;
const UART0_FIFO: *mut u32 = 0x6000_0000 as *mut u32;
const UART0_CLKDIV: *mut u32 = 0x6000_0014 as *mut u32;

macro_rules! nops {
    ($count:literal) => {
        unsafe {
            asm!(
                concat!(".rept ", stringify!($count), "\n nop\n .endr"),
                options(nomem, nostack)
            )
        }
    };
}

#[entry]
fn main() -> ! {
    unsafe {
        // 40 MHz APB / 115200 baud. CLKDIV is integer on this shared UART head.
        core::ptr::write_volatile(UART0_CLKDIV, 347);
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO0_MASK);
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO0_MASK);

        for &byte in b"C2_H4X_GPIO0_UART_OK\n" {
            core::ptr::write_volatile(UART0_FIFO, u32::from(byte));
        }
    }

    // GPIO0 drives the board's red discrete RGB channel. Four transitions are
    // enough to prove the output latch reaches the GPIO observer.
    for _ in 0..2 {
        unsafe {
            core::ptr::write_volatile(GPIO_OUT_W1TS, GPIO0_MASK);
        }
        nops!(12000);
        unsafe {
            core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO0_MASK);
        }
        nops!(12000);
    }

    loop {}
}
