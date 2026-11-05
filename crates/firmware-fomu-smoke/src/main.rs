#![no_std]
#![no_main]

use core::panic::PanicInfo;

const RESULT: *mut u32 = 0x1000_0000 as *mut u32;

#[link_section = ".text.entry"]
#[no_mangle]
pub extern "C" fn _start() -> ! {
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
