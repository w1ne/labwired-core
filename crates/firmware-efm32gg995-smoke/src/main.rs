//! EFM32GG995 (Giant Gecko, Cortex-M3) Renode-parity smoke.
//!
//! UART0, UART1, USART1 and USART2 each send one line first (`s0_uarts.rs`);
//! USART0 is then the console for the GPIO/TIMER checks in `s0.rs`, shared
//! with the other Series-0 smokes. The systems declare no debug_uart, so the
//! runner captures every UART. Summary word at 0x2001FF00.
#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

mod s0;
mod s0_uarts;

#[entry]
fn main() -> ! {
    s0_uarts::announce(&[
        ("uart0", 0x4000_E000),
        ("uart1", 0x4000_E400),
        ("usart1", 0x4000_C400),
        ("usart2", 0x4000_C800),
    ]);
    s0::run(&s0::Board {
        console: 0x4000_C000,
        banner: "EFM32GG995 RENODE SMOKE",
        family: "EFM32GG",
        summary: 0x2001_FF00,
        led: (4, 2),
        timers: 4,
    })
}
