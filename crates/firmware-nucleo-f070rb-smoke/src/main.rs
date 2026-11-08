#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_AHBENR: *mut u32 = 0x4002_1014 as *mut u32;
const GPIOA_MODER: *mut u32 = 0x4800_0000 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x4800_0018 as *mut u32;
const LD2_PA5: u32 = 1 << 5;

#[inline(never)]
fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // GPIOA is clocked by AHBENR bit 17 on STM32F070.
        write_volatile(RCC_AHBENR, read_volatile(RCC_AHBENR) | (1 << 17));

        // Establish LD2 low, then configure PA5 as a general-purpose output.
        write_volatile(GPIOA_BSRR, LD2_PA5 << 16);
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(GPIOA_MODER, (moder & !(0b11 << 10)) | (0b01 << 10));
    }

    // Four on/off cycles exercise the physical board LED path.
    for _ in 0..4 {
        unsafe { write_volatile(GPIOA_BSRR, LD2_PA5) };
        spin(128);
        unsafe { write_volatile(GPIOA_BSRR, LD2_PA5 << 16) };
        spin(128);
    }

    loop {
        core::hint::spin_loop();
    }
}
