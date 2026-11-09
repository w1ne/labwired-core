#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

//! ESP-Mosaico V1.2 GPIO7 button -> GPIO8 haptic-control smoke.
//!
//! LabWired loads the ELF directly into internal HP SRAM. This does not test
//! the S31 ROM, flash boot path, or ESP-IDF startup.

use panic_halt as _;
use riscv_rt::entry;

const GPIO_BASE: usize = 0x2058_3000;
const GPIO_OUT_W1TS: *mut u32 = (GPIO_BASE + 0x08) as *mut u32;
const GPIO_OUT_W1TC: *mut u32 = (GPIO_BASE + 0x0C) as *mut u32;
const GPIO_ENABLE_W1TS: *mut u32 = (GPIO_BASE + 0x38) as *mut u32;
const GPIO_IN: *const u32 = (GPIO_BASE + 0x64) as *const u32;
const GPIO_FUNC8_OUT_SEL_CFG: *mut u32 = (GPIO_BASE + 0xB14) as *mut u32;
const IO_MUX_GPIO7: *mut u32 = 0x2058_201C as *mut u32;
const IO_MUX_GPIO8: *mut u32 = 0x2058_2020 as *mut u32;
const GPIO7_MASK: u32 = 1 << 7;
const GPIO8_MASK: u32 = 1 << 8;

#[entry]
fn main() -> ! {
    unsafe {
        // GPIO7: GPIO function, input buffer enabled, internal pull-up enabled.
        core::ptr::write_volatile(IO_MUX_GPIO7, 0x1B00);
        // GPIO8: GPIO function. Haptic control is active-high on the board.
        core::ptr::write_volatile(IO_MUX_GPIO8, 0x1800);
        core::ptr::write_volatile(GPIO_FUNC8_OUT_SEL_CFG, 256 | (1 << 10));
        core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO8_MASK);
        core::ptr::write_volatile(GPIO_ENABLE_W1TS, GPIO8_MASK);
    }

    loop {
        let pressed = unsafe { core::ptr::read_volatile(GPIO_IN) & GPIO7_MASK == 0 };
        unsafe {
            if pressed {
                core::ptr::write_volatile(GPIO_OUT_W1TS, GPIO8_MASK);
            } else {
                core::ptr::write_volatile(GPIO_OUT_W1TC, GPIO8_MASK);
            }
            core::arch::asm!("nop", options(nomem, nostack));
        }
    }
}
