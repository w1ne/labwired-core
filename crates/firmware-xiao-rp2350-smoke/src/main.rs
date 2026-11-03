#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// Seeed's XIAO RP2350 user guide maps its yellow USER LED to GPIO25/D19.
const GPIO25_CTRL: *mut u32 = 0x4002_80CC as *mut u32;
const SIO_GPIO_OUT_SET: *mut u32 = 0xD000_0014 as *mut u32;
const SIO_GPIO_OUT_XOR: *mut u32 = 0xD000_001C as *mut u32;
const SIO_GPIO_OE_SET: *mut u32 = 0xD000_0024 as *mut u32;
const GPIO_FUNC_SIO: u32 = 5;
const USER_LED_MASK: u32 = 1 << 25;

#[entry]
fn main() -> ! {
    unsafe {
        let control = core::ptr::read_volatile(GPIO25_CTRL);
        core::ptr::write_volatile(GPIO25_CTRL, (control & !0x1F) | GPIO_FUNC_SIO);

        // The USER LED is active-low, so high is the off state.
        core::ptr::write_volatile(SIO_GPIO_OUT_SET, USER_LED_MASK);
        core::ptr::write_volatile(SIO_GPIO_OE_SET, USER_LED_MASK);

        loop {
            core::ptr::write_volatile(SIO_GPIO_OUT_XOR, USER_LED_MASK);
            delay();
        }
    }
}

#[inline(never)]
fn delay() {
    for _ in 0..4_000 {
        cortex_m::asm::nop();
    }
}
