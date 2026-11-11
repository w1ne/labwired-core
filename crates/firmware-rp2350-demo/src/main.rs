#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// RP2350 Peripheral Base Addresses (rp2350 addressmap.h — not the RP2040 map).
const UART0_BASE: u32 = 0x40070000;
// RP2350 SIO at 0xD000_0000; every GPIO register has an HI twin at +4.
const SIO_GPIO_OUT_XOR: *mut u32 = 0xD0000028 as *mut u32;
const SIO_GPIO_OE_SET: *mut u32 = 0xD0000038 as *mut u32;
const GPIO25: u32 = 1 << 25;

// UART data register (PL011 UARTDR at offset 0x00).
const UART0_DR: *mut u32 = UART0_BASE as *mut u32;

#[entry]
fn main() -> ! {
    unsafe {
        core::ptr::write_volatile(SIO_GPIO_OE_SET, GPIO25);
    }

    loop {
        // Toggle the Pico 2 onboard LED through the SIO GPIO output latch.
        unsafe {
            core::ptr::write_volatile(SIO_GPIO_OUT_XOR, GPIO25);
        }

        // Write a message to UART
        print_uart("RP2350_SMOKE_OK\n");
        print_uart("PICO2_GPIO25_UART_OK\n");

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
