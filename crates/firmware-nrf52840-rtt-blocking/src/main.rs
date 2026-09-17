// LabWired - Firmware Simulation Platform
// SEGGER RTT blocking-mode fixture: same stock vendor library, but with a
// 16-byte up-buffer and BLOCK_IF_FIFO_FULL. The 25-byte line is longer than
// the 15 usable bytes, so the firmware blocks until the model drains.
#![no_std]
#![no_main]

use core::ffi::c_char;
use cortex_m_rt::entry;
use panic_halt as _;

extern "C" {
    fn SEGGER_RTT_Init();
    fn SEGGER_RTT_WriteString(buffer_index: u32, s: *const c_char) -> u32;
}

#[entry]
fn main() -> ! {
    unsafe {
        SEGGER_RTT_Init();
    }
    loop {
        unsafe {
            SEGGER_RTT_WriteString(0, c"BLOCK16:0123456789ABCDEF\n".as_ptr());
        }
        // Shorter pace than the default fixture: a blocking write already
        // yields until a drain, so a run makes progress quickly.
        for _ in 0..1000u32 {
            core::hint::spin_loop();
        }
    }
}
