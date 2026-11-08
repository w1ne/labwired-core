//! EFM32HG350 (Happy Gecko, Cortex-M0+) Renode-parity smoke.
//!
//! USART0 is the console; the Series-0 GPIO/TIMER checks are shared with the
//! EFM32GG995 smoke (`firmware-efm32gg995-smoke/src/s0.rs`). Renode's
//! efm32hg350.repl maps three TIMERs, so the TIMER2/TIMER3 checks are skipped
//! where absent. LED0 is PF4 (STK3400). Summary word at 0x2000_1F00.
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
        banner: "EFM32HG350 RENODE SMOKE",
        family: "EFM32HG",
        summary: 0x2000_1F00,
        led: (5, 4),
        timers: 3,
    })
}
