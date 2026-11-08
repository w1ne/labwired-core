#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m::asm::nop;
use cortex_m_rt::entry;
use panic_halt as _;

const RCC_IOPENR: *mut u32 = 0x4002_104C as *mut u32;
const GPIOA_MODER: *mut u32 = 0x5000_0000 as *mut u32;
const GPIOA_BSRR: *mut u32 = 0x5000_0018 as *mut u32;
const LD4_PA5: u32 = 1 << 5;

#[entry]
fn main() -> ! {
    unsafe {
        // RM0503 U0 IOPENR is at RCC+0x4C; enable GPIOA before touching PA5.
        write_volatile(RCC_IOPENR, read_volatile(RCC_IOPENR) | 1);
        let moder = read_volatile(GPIOA_MODER);
        write_volatile(GPIOA_MODER, (moder & !(0b11 << 10)) | (0b01 << 10));

        // LD4 is the default active-high PA5 connection on NUCLEO-U083RC.
        // Alternate set/reset writes make the board-level pin activity visible.
        for _ in 0..8 {
            write_volatile(GPIOA_BSRR, LD4_PA5);
            for _ in 0..16 {
                nop();
            }
            write_volatile(GPIOA_BSRR, LD4_PA5 << 16);
            for _ in 0..16 {
                nop();
            }
        }
    }

    loop {
        nop();
    }
}
