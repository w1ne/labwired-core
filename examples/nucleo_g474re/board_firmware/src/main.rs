#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

const USART2_TDR_PTR: *mut u8 = (0x40004400 + 0x28) as *mut u8;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        core::ptr::write_volatile(USART2_TDR_PTR, b'O');
        core::ptr::write_volatile(USART2_TDR_PTR, b'K');
        core::ptr::write_volatile(USART2_TDR_PTR, b'\n');
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
