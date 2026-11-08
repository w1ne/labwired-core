//! EFM32GG995 (Giant Gecko, Cortex-M3) Renode-parity smoke.
//!
//! USART0 is the console; the GPIO/TIMER checks live in `s0.rs`, shared
//! with the other Series-0 smokes. Summary word at 0x2001FF00.
#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

mod s0;

#[entry]
fn main() -> ! {
    s0::run(&s0::Board {
        console: 0x4000_C000,
        banner: "EFM32GG995 RENODE SMOKE",
        family: "EFM32GG",
        summary: 0x2001_FF00,
    })
}
