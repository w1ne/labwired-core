#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// RP2350 Peripheral Base Addresses (rp2350 addressmap.h — not the RP2040 map).
const UART0_BASE: u32 = 0x40070000;
// Scratch write into PIO0 window — smoke does not require a real LED model.
const MOCK_LED_REG: *mut u32 = 0x50200000 as *mut u32;

// UART data register (PL011 UARTDR at offset 0x00).
const UART0_DR: *mut u32 = UART0_BASE as *mut u32;

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
        print_uart("RP2350_SMOKE_OK\n");

        // Small delay
        for _ in 0..1000u32 {
            cortex_m::asm::nop();
        }
    }
}

fn print_uart(s: &str) {
    for b in s.bytes() {
        unsafe {
            core::ptr::write_volatile(UART0_DR, b as u32);
        }
    }
}
