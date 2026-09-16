// LabWired - Firmware Simulation Platform
// SEGGER RTT smoke firmware: links the stock vendor RTT library unmodified and
// prints the string the `rtt-smoke.yaml` script asserts on.
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
            SEGGER_RTT_WriteString(0, c"RTT hello from labwired\n".as_ptr());
        }
        // Pace the loop so the ring buffer does not wrap every few cycles.
        for _ in 0..100_000u32 {
            core::hint::spin_loop();
        }
    }
}
