#![no_std]
#![no_main]

use panic_halt as _;
use riscv_rt::entry;

const PCR_UART0_CONF: *mut u32 = 0x6009_6000 as *mut u32;
const PCR_UART0_SCLK_CONF: *mut u32 = 0x6009_6004 as *mut u32;
const UART0_BASE: *mut u32 = 0x6000_0000 as *mut u32;
const UART0_CLKDIV: *mut u32 = 0x6000_0014 as *mut u32;

#[entry]
fn main() -> ! {
    unsafe {
        // Enable UART0's APB and function clocks using the C61 PCR registers.
        let conf = core::ptr::read_volatile(PCR_UART0_CONF);
        core::ptr::write_volatile(PCR_UART0_CONF, conf | 1);
        let sclk = core::ptr::read_volatile(PCR_UART0_SCLK_CONF);
        core::ptr::write_volatile(PCR_UART0_SCLK_CONF, sclk | (1 << 22));

        // Set the documented 115200 baud divider, then send through UART0 FIFO.
        core::ptr::write_volatile(UART0_CLKDIV, 694);
        for &byte in b"ESP32C61_DEVKITC1_UART0_OK\n" {
            core::ptr::write_volatile(UART0_BASE, u32::from(byte));
        }
    }

    loop {}
}
