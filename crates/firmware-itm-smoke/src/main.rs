// LabWired - Firmware Simulation Platform
// ITM smoke: enables stimulus port 0 and writes the bytes the CLI test reads.
#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

const ITM_PORT0: *mut u8 = 0xE000_0000 as *mut u8;
const ITM_TER: *mut u32 = 0xE000_0E00 as *mut u32;
const ITM_TCR: *mut u32 = 0xE000_0E80 as *mut u32;

#[entry]
fn main() -> ! {
    unsafe {
        // ITMENA is clear. This byte must not appear in the stream.
        core::ptr::write_volatile(ITM_PORT0, b'X');
        core::ptr::write_volatile(ITM_TCR, 1);
        core::ptr::write_volatile(ITM_TER, 1);
        for &byte in b"ITM hello" {
            core::ptr::write_volatile(ITM_PORT0, byte);
        }
    }
    loop {
        core::hint::spin_loop();
    }
}
