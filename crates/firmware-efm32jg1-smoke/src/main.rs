//! EFM32JG1 (Jade Gecko, Cortex-M3) Renode-parity smoke.
//!
//! USART0 is the console; the Series-1 GPIO checks live in
//! firmware-efm32jg1-smoke/src/s1.rs.
#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

mod s1;

#[entry]
fn main() -> ! {
    s1::run(&s1::Board {
        banner: "EFM32JG1 RENODE SMOKE",
        family: "EFM32JG1",
        summary: 0x2000_7C00,
        // LED0/LED1 = PF4/PF5, as on the PG1/PG12 starter kits
        // (SLSTK3401A/SLSTK3402A).
        led: (5, 4),
    })
}
