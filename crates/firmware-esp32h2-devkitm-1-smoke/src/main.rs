#![no_std]
#![no_main]

use panic_halt as _;
use riscv_rt::entry;

const UART0_BASE: *mut u32 = 0x6000_0000 as *mut u32;
const UART0_CLKDIV: *mut u32 = 0x6000_0014 as *mut u32;
const GPIO_BASE: usize = 0x6009_1000;
const GPIO_OUT_W1TS: *mut u32 = (GPIO_BASE + 0x08) as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = (GPIO_BASE + 0x0C) as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = (GPIO_BASE + 0x24) as *mut u32;
const GPIO10_OUT_SEL: *mut u32 = (GPIO_BASE + 0x554 + 10 * 4) as *mut u32;
const GPIO10_MASK: u32 = 1 << 10;

#[entry]
fn main() -> ! {
    unsafe {
        // GPIO10 is a header pin; H2 selects the GPIO_OUT latch with signal 128.
        core::ptr::write_volatile(GPIO10_OUT_SEL, 128);
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO10_MASK);
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO10_MASK);
        for _ in 0..4 {
            core::ptr::write_volatile(GPIO_OUT_W1TS, GPIO10_MASK);
            core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO10_MASK);
        }

        // H2 UART0's reset divider is 694; emit through the USB-UART console.
        core::ptr::write_volatile(UART0_CLKDIV, 694);
        for &byte in b"ESP32H2_DEVKITM1_UART0_OK\n" {
            core::ptr::write_volatile(UART0_BASE, u32::from(byte));
        }
    }

    loop {}
}
