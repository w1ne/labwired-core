#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// RP2350 register addresses, from its address map (not the RP2040 map).
const UART0_DR: *mut u32 = 0x4007_0000 as *mut u32;
const GPIO25_CTRL: *mut u32 = 0x4002_80CC as *mut u32;
const SIO_GPIO_OUT_SET: *mut u32 = 0xD000_0014 as *mut u32;
const SIO_GPIO_OUT_CLR: *mut u32 = 0xD000_0018 as *mut u32;
const SIO_GPIO_OE_SET: *mut u32 = 0xD000_0024 as *mut u32;

const GPIO_FUNC_SIO: u32 = 5;
const LED_PIN: u32 = 25;

#[entry]
fn main() -> ! {
    unsafe {
        // Route GP25 through SIO, then enable it as an output. Pico 2's user
        // LED is attached to this pad in the board system profile.
        core::ptr::write_volatile(GPIO25_CTRL, GPIO_FUNC_SIO);
        core::ptr::write_volatile(SIO_GPIO_OE_SET, 1 << LED_PIN);
    }

    // The UART assertion observes the modeled PL011 data-register path. USB
    // CDC and the physical UART waveform are not part of this smoke.
    print_uart("PICO2_SMOKE_OK\n");

    loop {
        for _ in 0..4 {
            unsafe { core::ptr::write_volatile(SIO_GPIO_OUT_SET, 1 << LED_PIN) };
            delay(1_000);
            unsafe { core::ptr::write_volatile(SIO_GPIO_OUT_CLR, 1 << LED_PIN) };
            delay(1_000);
        }
    }
}

fn print_uart(s: &str) {
    for byte in s.bytes() {
        unsafe { core::ptr::write_volatile(UART0_DR, byte as u32) };
    }
}

fn delay(n: u32) {
    for _ in 0..n {
        cortex_m::asm::nop();
    }
}
