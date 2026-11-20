//! EFM32G210 (Gecko, Cortex-M3) Renode-parity smoke.
//!
//! USART0 is the console; the Series-0 GPIO/TIMER checks are shared with the
//! EFM32GG995 smoke (`firmware-efm32gg995-smoke/src/s0.rs`). Renode's efm32g210.repl maps two TIMERs, so the TIMER2/TIMER3
//! checks are skipped.
//! LED0 is PC0 (the EFM32-G8xx-STK LED0 pin). Summary word at 0x2000_3F00.
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
        banner: "EFM32G210 RENODE SMOKE",
        family: "EFM32G",
        summary: 0x2000_3F00,
        led: (2, 0),
        timers: 2,
    })
}
