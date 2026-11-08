#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

const IO_BANK0_BASE: u32 = 0x4001_4000;
const SIO_BASE: u32 = 0xD000_0000;
const SIO_GPIO_OUT_SET: *mut u32 = (SIO_BASE + 0x14) as *mut u32;
const SIO_GPIO_OUT_XOR: *mut u32 = (SIO_BASE + 0x1C) as *mut u32;
const SIO_GPIO_OE_SET: *mut u32 = (SIO_BASE + 0x24) as *mut u32;
const LED_MASK: u32 = (1 << 16) | (1 << 17) | (1 << 25);
const GPIO_FUNC_SIO: u32 = 5;

#[entry]
fn main() -> ! {
    unsafe {
        for pin in [16u32, 17, 25] {
            let gpio_ctrl = (IO_BANK0_BASE + 0x04 + 8 * pin) as *mut u32;
            let control = core::ptr::read_volatile(gpio_ctrl);
            core::ptr::write_volatile(gpio_ctrl, (control & !0x1F) | GPIO_FUNC_SIO);
        }

        // XIAO's three indicator LEDs are active-low, so high is off.
        core::ptr::write_volatile(SIO_GPIO_OUT_SET, LED_MASK);
        core::ptr::write_volatile(SIO_GPIO_OE_SET, LED_MASK);

        loop {
            core::ptr::write_volatile(SIO_GPIO_OUT_XOR, LED_MASK);
            delay();
        }
    }
}

#[inline(never)]
fn delay() {
    for _ in 0..5_000 {
        cortex_m::asm::nop();
    }
}
