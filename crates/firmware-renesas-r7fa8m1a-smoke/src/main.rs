#![no_std]
#![no_main]

use core::ptr::write_volatile;
use cortex_m_rt::entry;
use panic_halt as _;

const RESULT: *mut u32 = 0x2200_0000 as *mut u32;

#[entry]
fn main() -> ! {
    let product: u32;
    unsafe {
        core::arch::asm!(
            "mul {product}, {left}, {right}",
            product = out(reg) product,
            left = in(reg) 17u32,
            right = in(reg) 19u32,
            options(nomem, nostack, pure),
        );
        write_volatile(RESULT, product);
    }

    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
}
