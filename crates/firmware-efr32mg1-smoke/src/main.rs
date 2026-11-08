//! EFR32MG1 (Cortex-M4F) Renode-parity smoke.
//!
//! USART0 is the console; the Series-1 GPIO and TIMER checks live in
//! firmware-efm32jg1-smoke/src/s1.rs (the EFR32 Series-1 GPIO_P and TIMER
//! register maps are the EFM32PG1B ones).
#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

#[path = "../../firmware-efm32jg1-smoke/src/s1.rs"]
mod s1;

#[entry]
fn main() -> ! {
    s1::run(&s1::Board {
        banner: "EFR32MG1 RENODE SMOKE",
        family: "EFR32MG1",
        summary: 0x2000_7C00,
        // LED0/LED1 = PF4/PF5 on the WSTK radio boards.
        led: (5, 4),
        timer0_irq: 10,
        wtimer_irqs: &[],
    })
}
