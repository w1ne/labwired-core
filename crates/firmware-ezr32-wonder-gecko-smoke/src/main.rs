//! SLWSTK6220A (EZR32WG330 868 MHz kit) Renode-parity smoke.
//!
//! Same checks as the EZR32WG330 chip smoke (`firmware-ezr32-330-smoke`):
//! UART1/USART1/USART2 lines, then UART0 as the console for the Series-0
//! GPIO/TIMER checks. LED0/LED1 are PE2/PE3 (Renode slwstk6220a.repl), so
//! the push-pull check drives the board LEDs and LED0 toggles on every
//! TIMER0 overflow afterwards. Summary word at 0x2000_7F00.
#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

#[path = "../../firmware-efm32gg995-smoke/src/s0.rs"]
mod s0;

#[path = "../../firmware-ezr32-330-smoke/src/ezr32_uarts.rs"]
mod ezr32_uarts;

#[entry]
fn main() -> ! {
    ezr32_uarts::announce(&[
        ("uart1", 0x4000_E400),
        ("usart1", 0x4000_C400),
        ("usart2", 0x4000_C800),
    ]);
    s0::run(&s0::Board {
        console: 0x4000_E000,
        banner: "SLWSTK6220A RENODE SMOKE",
        family: "SLWSTK6220A",
        summary: 0x2000_7F00,
        led: (4, 2),
        timers: 4,
    })
}
