#![no_std]
#![no_main]

use core::ptr::write_volatile;
use cortex_m_rt::entry;
use panic_halt as _;

// Renode platforms/cpus/eos-s3.repl: PL011 at 0x4001_0000.
const UART_DATA: *mut u32 = 0x4001_0000 as *mut u32;

#[entry]
fn main() -> ! {
    for byte in b"EOS S3 Cortex-M4F PL011 smoke" {
        unsafe { write_volatile(UART_DATA, u32::from(*byte)) };
    }
    loop {
        core::hint::spin_loop();
    }
}
