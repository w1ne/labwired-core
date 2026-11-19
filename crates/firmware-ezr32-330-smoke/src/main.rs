//! EZR32LG330 / EZR32WG330 (Leopard / Wonder Gecko + Si4460) Renode-parity
//! smoke. Both rows map the same blocks (Renode ezr32wg330.repl pulls in
//! ezr32lg330.repl), so one image serves both.
//!
//! UART1, USART1 and USART2 each send one line first; UART0 is then the
//! console for the Series-0 GPIO/TIMER checks shared with the EFM32GG995
//! smoke (`firmware-efm32gg995-smoke/src/s0.rs`). The systems declare no
//! debug_uart, so the runner captures every UART. LED0 is PE2 (Renode
//! slwstk6220a.repl). Summary word at 0x2000_7F00.
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
        ("uart1", 0x4000_E400),
        ("usart1", 0x4000_C400),
        ("usart2", 0x4000_C800),
    ]);
    s0::run(&s0::Board {
        console: 0x4000_E000,
        banner: "EZR32 330 RENODE SMOKE",
        family: "EZR32 330",
        summary: 0x2000_7F00,
        led: (4, 2),
        timers: 4,
    })
}
