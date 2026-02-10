#![no_std]
#![no_main]

use panic_halt as _;

// Standard UART1 base for LabWired default configuration
const UART_DR: *mut u8 = 0x4000_C004 as *mut u8;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    let message = b"Hello from LabWired ARM Cortex-M0!\n";
    for &b in message {
        unsafe {
            core::ptr::write_volatile(UART_DR, b);
        }
    }

    loop {
        // Stay here so we can debug
        core::hint::spin_loop();
    }
}
