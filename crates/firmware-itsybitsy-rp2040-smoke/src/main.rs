#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

const IO_BANK0_BASE: u32 = 0x4001_4000;
const SIO_BASE: u32 = 0xD000_0000;
const GPIO11_CTRL: *mut u32 = (IO_BANK0_BASE + 0x04 + 11 * 8) as *mut u32;
const SIO_GPIO_OUT: *mut u32 = (SIO_BASE + 0x10) as *mut u32;
const SIO_GPIO_OE: *mut u32 = (SIO_BASE + 0x20) as *mut u32;
const LED_BIT: u32 = 1 << 11;
const GPIO_FUNC_SIO: u32 = 5;

#[entry]
fn main() -> ! {
    unsafe {
        let control = core::ptr::read_volatile(GPIO11_CTRL);
        core::ptr::write_volatile(GPIO11_CTRL, (control & !0x1F) | GPIO_FUNC_SIO);
        core::ptr::write_volatile(SIO_GPIO_OE, LED_BIT);

        loop {
            core::ptr::write_volatile(SIO_GPIO_OUT, LED_BIT);
            delay();
            core::ptr::write_volatile(SIO_GPIO_OUT, 0);
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
