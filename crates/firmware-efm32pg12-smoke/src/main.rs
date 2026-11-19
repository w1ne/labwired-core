//! EFM32PG12 (Pearl Gecko 12, Cortex-M4F) Renode-parity smoke.
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
        banner: "EFM32PG12 RENODE SMOKE",
        family: "EFM32PG12",
        summary: 0x2003_FC00,
        // LED0/LED1 = PF4/PF5 on the SLSTK3402A starter kit.
        led: (5, 4),
        timer0_irq: 11,
        // WTIMER0_IRQn 36, WTIMER1_IRQn 37 (Gecko SDK v4.4.0).
        wtimer_irqs: &[36, 37],
        rtcc_irq: None,
    })
}
