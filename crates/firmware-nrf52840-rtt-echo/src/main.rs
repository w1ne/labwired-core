// LabWired - Firmware Simulation Platform
// Echoes each SEGGER_RTT_GetKey byte back on up-channel 0. Links the stock
// vendor RTT library unmodified.
#![no_std]
#![no_main]

use core::ffi::c_char;
use cortex_m_rt::entry;
use panic_halt as _;

extern "C" {
    fn SEGGER_RTT_Init();
    fn SEGGER_RTT_GetKey() -> i32;
    fn SEGGER_RTT_PutChar(buffer_index: u32, c: c_char) -> u32;
}

#[entry]
fn main() -> ! {
    unsafe {
        SEGGER_RTT_Init();
    }
    loop {
        let key = unsafe { SEGGER_RTT_GetKey() };
        if key >= 0 {
            unsafe {
                SEGGER_RTT_PutChar(0, key as u8 as c_char);
            }
        }
    }
}
