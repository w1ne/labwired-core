#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// nRF52840 UART0 Base Address
const UART0_BASE: u32 = 0x40002000;
const UART0_ENABLE: *mut u32 = (UART0_BASE + 0x500) as *mut u32;
const UART0_TXD: *mut u32 = (UART0_BASE + 0x51C) as *mut u32;

// Mock LED for demonstration
const MOCK_LED_REG: *mut u32 = 0x50000000 as *mut u32;

#[entry]
fn main() -> ! {
    let mut led_state = 0u32;

    unsafe {
        // Enable UART (value 4 = ENABLE)
        core::ptr::write_volatile(UART0_ENABLE, 4);
    }

    loop {
        // "Blink" the mocked LED by toggling the register
        led_state ^= 1;
        unsafe {
            core::ptr::write_volatile(MOCK_LED_REG, led_state);
        }

        // Write a message to UART
        print_uart("NRF52840_SMOKE_OK\n");

        // Small delay
        for _ in 0..1000u32 {
            cortex_m::asm::nop();
        }
    }
}

fn print_uart(s: &str) {
    for b in s.bytes() {
        unsafe {
            core::ptr::write_volatile(UART0_TXD, b as u32);
        }
    }
}
