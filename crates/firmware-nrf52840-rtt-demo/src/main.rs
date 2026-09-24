// LabWired - Firmware Simulation Platform
// SEGGER RTT tutorial demo: the sample from SEGGER's RTT knowledge-base page
// (hello + printf counter + virtual-terminal color beat), ported to bare-metal
// nRF52840. Links the stock vendor library unmodified.
#![no_std]
#![no_main]

use core::ffi::{c_char, CStr};
use cortex_m_rt::entry;
use panic_halt as _;

extern "C" {
    fn SEGGER_RTT_Init();
    fn SEGGER_RTT_WriteString(buffer_index: u32, s: *const c_char) -> u32;
    fn SEGGER_RTT_printf(buffer_index: u32, s_format: *const c_char, ...) -> i32;
    fn SEGGER_RTT_TerminalOut(terminal_id: u8, s: *const c_char) -> i32;
    /// SEGGER's host-to-target example: one character from down-channel 0,
    /// or -1 when the host has not stored one.
    fn SEGGER_RTT_GetKey() -> i32;
}

// The same control sequences as SEGGER_RTT.h's RTT_CTRL_TEXT_* macros. Rust
// cannot see C macros, so the escapes are repeated here verbatim. The red one
// is inlined in the TerminalOut call below, exactly as in SEGGER's C sample.
const BRIGHT_WHITE: &CStr = c"\x1B[1;37m";
const BRIGHT_GREEN: &CStr = c"\x1B[1;32m";

#[entry]
fn main() -> ! {
    unsafe {
        SEGGER_RTT_Init();
        SEGGER_RTT_WriteString(0, c"Hello World from SEGGER!\n".as_ptr());
    }
    let mut cnt: i32 = 0;
    loop {
        unsafe {
            SEGGER_RTT_printf(
                0,
                c"%sCounter: %s%d\n".as_ptr(),
                BRIGHT_WHITE.as_ptr(),
                BRIGHT_GREEN.as_ptr(),
                cnt,
            );
            if cnt > 100 {
                SEGGER_RTT_TerminalOut(1, c"\x1B[1;31mCounter overflow!".as_ptr());
                cnt = 0;
            }
            // SEGGER's GetKey example: the host stored the byte in down-channel 0.
            let key = SEGGER_RTT_GetKey();
            if key >= 0 {
                SEGGER_RTT_printf(0, c"Got key: %c\n".as_ptr(), key);
                if key == i32::from(b'q') {
                    SEGGER_RTT_WriteString(0, c"quit\n".as_ptr());
                    loop {
                        core::hint::spin_loop();
                    }
                }
            }
        }
        // Pace the loop so a browser run shows the counter climbing instead of
        // a wall of text. Deterministic in simulation (no wall clock involved).
        for _ in 0..50_000u32 {
            core::hint::spin_loop();
        }
        cnt += 1;
    }
}
