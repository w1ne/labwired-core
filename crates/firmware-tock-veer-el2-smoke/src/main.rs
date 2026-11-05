#![no_std]
#![no_main]

use core::panic::PanicInfo;

const RESULT: *mut u32 = 0x2060_0000 as *mut u32;

core::arch::global_asm!(
    ".section .text.entry,\"ax\",@progbits",
    ".globl _start",
    ".type _start,@function",
    "_start:",
    "la sp, __stack_top",
    "call main",
    "1: j 1b",
);

#[no_mangle]
pub extern "C" fn main() -> ! {
    let product: u32;
    unsafe {
        core::arch::asm!(
            "mul {product}, {left}, {right}",
            product = lateout(reg) product,
            left = in(reg) 17u32,
            right = in(reg) 19u32,
            options(nomem, nostack),
        );
        core::ptr::write_volatile(RESULT, product);
    }

    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
}

#[panic_handler]
fn panic(_: &PanicInfo<'_>) -> ! {
    loop {
        unsafe { core::arch::asm!("nop", options(nomem, nostack)) };
    }
}
