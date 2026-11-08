//! EFM32LG942 (Leopard Gecko, Cortex-M3) Renode-parity smoke.
//!
//! USART0 is the console; the Series-0 GPIO/TIMER checks are shared with the
//! EFM32GG995 smoke (`firmware-efm32gg995-smoke/src/s0.rs`). Summary word
//! at 0x2001_FF00.
#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

#[path = "../../firmware-efm32gg995-smoke/src/s0.rs"]
mod s0;

#[entry]
fn main() -> ! {
    s0::run(&s0::Board {
        console: 0x4000_C000,
        banner: "EFM32LG942 RENODE SMOKE",
        family: "EFM32LG",
        summary: 0x2001_FF00,
    })
}
