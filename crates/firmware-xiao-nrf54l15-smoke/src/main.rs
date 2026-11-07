#![no_std]
#![no_main]

use core::ptr::write_volatile;
use cortex_m_rt::entry;
use panic_halt as _;

const GPIO2: usize = 0x5005_0400;
const OUTSET: *mut u32 = (GPIO2 + 0x004) as *mut u32;
const OUTCLR: *mut u32 = (GPIO2 + 0x008) as *mut u32;
const DIRSET: *mut u32 = (GPIO2 + 0x014) as *mut u32;
const USER_LED: u32 = 1;

#[entry]
fn main() -> ! {
    unsafe {
        // The Seeed board guide and schematic map the active-low user LED to P2.00.
        write_volatile(DIRSET, USER_LED);
        write_volatile(OUTCLR, USER_LED);

        // Keep the LED on at the end while producing 32 observable pin edges.
        for step in 0..=32 {
            if step % 2 == 0 {
                write_volatile(OUTCLR, USER_LED);
            } else {
                write_volatile(OUTSET, USER_LED);
            }
            core::hint::spin_loop();
        }
    }

    loop {
        core::hint::spin_loop();
    }
}
