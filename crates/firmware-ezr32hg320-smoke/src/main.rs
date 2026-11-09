//! EZR32HG320 (Happy Gecko + Si4460, Cortex-M0+) Renode-parity smoke.
//!
//! USART0 is the console; the Series-0 GPIO/TIMER checks are shared with the
//! EFM32GG995 smoke (`firmware-efm32gg995-smoke/src/s0.rs`). Renode's
//! ezr32hg320.repl maps three TIMERs, so the TIMER3 check is skipped. No
//! Renode board repl names HG320 LEDs, so PF4/PF5 (as on the EFM32HG350
//! smoke) carry the push-pull check; pad bonding is not modelled. Summary word at 0x2000_1F00.
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
        banner: "EZR32HG320 RENODE SMOKE",
        family: "EZR32HG",
        summary: 0x2000_1F00,
        led: (5, 4),
        timers: 3,
    })
}
