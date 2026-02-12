
#![no_std]
#![no_main]

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    // Intentionally access invalid memory to trigger a MemoryViolation
    let ptr = 0xFFFFFFFF as *mut u8;
    unsafe {
        *ptr = 42;
    }
    loop {}
}
