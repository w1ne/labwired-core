//! EFM32LG995 (Leopard Gecko, Cortex-M3) Renode-parity smoke.
//!
//! UART0, UART1, USART1 and USART2 each send one line first (`s0_uarts.rs`);
//! USART0 is then the console for the Series-0 GPIO/TIMER checks shared with
//! the EFM32GG995 smoke (`firmware-efm32gg995-smoke/src/s0.rs`). The systems
//! declare no debug_uart, so the runner captures every UART. Summary word
//! at 0x2000_7F00 (32 KiB SRAM).
#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

#[path = "../../firmware-efm32gg995-smoke/src/s0.rs"]
mod s0;

#[path = "../../firmware-efm32gg995-smoke/src/s0_uarts.rs"]
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
        banner: "EFM32LG995 RENODE SMOKE",
        family: "EFM32LG",
        summary: 0x2000_7F00,
        led: (4, 2),
        timers: 4,
    })
}
