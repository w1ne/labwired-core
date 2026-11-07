#![no_std]
#![no_main]

use panic_halt as _;
use riscv_rt::entry;

const GPIO_OUT_W1TS: *mut u32 = 0x6000_4008 as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = 0x6000_400c as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = 0x6000_4024 as *mut u32;
const D10_GPIO_MASK: u32 = 1 << 10;

#[entry]
fn main() -> ! {
    unsafe {
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, D10_GPIO_MASK);
        loop {
            core::ptr::write_volatile(GPIO_OUT_W1TS, D10_GPIO_MASK);
            delay();
            core::ptr::write_volatile(GPIO_OUT_W1TC, D10_GPIO_MASK);
            delay();
        }
    }
}

#[inline(never)]
fn delay() {
    for _ in 0..16 {
        unsafe { core::arch::asm!("nop") };
    }
}
