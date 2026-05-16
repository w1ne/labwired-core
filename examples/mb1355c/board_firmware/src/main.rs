#![no_std]
#![no_main]
#![allow(clippy::empty_loop)]

const USART1_TDR_PTR: *mut u8 = (0x40013800 + 0x28) as *mut u8;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        core::ptr::write_volatile(USART1_TDR_PTR, b'O');
        core::ptr::write_volatile(USART1_TDR_PTR, b'K');
        core::ptr::write_volatile(USART1_TDR_PTR, b'\n');
    }

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
