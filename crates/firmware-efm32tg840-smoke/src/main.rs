//! EFM32TG840 (Tiny Gecko, Cortex-M3) Renode-parity smoke.
//!
//! USART0 is the console; the Series-0 GPIO/TIMER checks are shared with the
//! EFM32GG995 smoke (`firmware-efm32gg995-smoke/src/s0.rs`). Renode's
//! efm32tg840.repl maps two TIMERs, so the TIMER2/TIMER3 checks are skipped
//! where absent. LED0 is PD7 (STK3300). Summary word at 0x2000_0F00.
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
        banner: "EFM32TG840 RENODE SMOKE",
        family: "EFM32TG",
        summary: 0x2000_0F00,
        led: (3, 7),
        timers: 2,
    })
}
