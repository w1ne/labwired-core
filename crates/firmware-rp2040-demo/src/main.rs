#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// RP2040 Peripheral Base Addresses
const UART0_BASE: u32 = 0x40034000;
// We'll use a mocked "LED" mapped to a scratch register or standard PIO base for demonstration
const MOCK_LED_REG: *mut u32 = 0x50200000 as *mut u32;

// UART Registers (stm32v2 layout: TDR at offset 0x28)
const UART0_TDR: *mut u32 = (UART0_BASE + 0x28) as *mut u32;

#[entry]
fn main() -> ! {
    let mut led_state = 0u32;

    loop {
        // "Blink" the mocked LED by toggling the register
        led_state ^= 1;
        unsafe {
            core::ptr::write_volatile(MOCK_LED_REG, led_state);
        }

        // Write a message to UART
        print_uart("RP2040_SMOKE_OK\n");

        // Small delay
        for _ in 0..1000u32 {
            cortex_m::asm::nop();
        }
    }
}

fn print_uart(s: &str) {
    for b in s.bytes() {
        unsafe {
            core::ptr::write_volatile(UART0_TDR, b as u32);
        }
    }
}
