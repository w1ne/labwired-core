// Arduino Nano R4 GPIO smoke: toggle the board's active-high LED_BUILTIN.
// LED_BUILTIN is Arduino pin 22, RA4M1 P204 (ArduinoCore-renesas NANOR4).

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// RA4M1 PORT2 @ 0x4004_0040; PCNTR1 has PDR[15:0], PCNTR3 has POSR/PORR.
const PORT2_PCNTR1: *mut u32 = 0x4004_0040 as *mut u32;
const PORT2_PCNTR3: *mut u32 = 0x4004_0048 as *mut u32;
const LED_BUILTIN: u32 = 1 << 4; // P204

#[inline(never)]
fn settle() {
    for _ in 0..64 {
        unsafe { core::arch::asm!("nop", options(nomem, nostack, preserves_flags)) };
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // Configure P204 as a GPIO output (PORT2.PDR[4]).
        write_volatile(PORT2_PCNTR1, read_volatile(PORT2_PCNTR1) | LED_BUILTIN);

        // The Nano R4 schematic wires P204 through R9 and DL2 to ground, so
        // driving it high lights the on-board LED. Exercise both output levels.
        for _ in 0..8 {
            write_volatile(PORT2_PCNTR3, LED_BUILTIN); // POSR: output high
            settle();
            write_volatile(PORT2_PCNTR3, LED_BUILTIN << 16); // PORR: output low
            settle();
        }
    }

    loop {
        core::hint::spin_loop();
    }
}
