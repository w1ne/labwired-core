//! EFM32JG12 (Jade Gecko 12, Cortex-M3) Renode-parity smoke.
//!
//! USART0 is the console; the Series-1 GPIO checks live in
//! firmware-efm32jg1-smoke/src/s1.rs.
#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

#[path = "../../firmware-efm32jg1-smoke/src/s1.rs"]
mod s1;

#[entry]
fn main() -> ! {
    s1::run(&s1::Board {
        banner: "EFM32JG12 RENODE SMOKE",
        family: "EFM32JG12",
        summary: 0x2003_FC00,
        // LED0/LED1 = PF4/PF5, as on the PG1/PG12 starter kits
        // (SLSTK3401A/SLSTK3402A).
        led: (5, 4),
        timer0_irq: 11,
    })
}
