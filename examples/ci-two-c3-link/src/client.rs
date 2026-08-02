//! Fixture node B: answers every PING on UART1 with a PONG.

#![no_std]
#![no_main]

#[macro_use]
mod rt;
mod uart;

use uart::{puts, try_get, UART0, UART1};

bare_metal_entry!(main);

fn main() -> ! {
    puts(UART0, "client up\r\n");
    const PING: &[u8] = b"PING\n";
    let mut matched = 0usize;

    loop {
        if let Some(b) = try_get(UART1) {
            matched = if b == PING[matched] { matched + 1 } else { 0 };
            if matched == PING.len() {
                matched = 0;
                puts(UART1, "PONG\n");
                puts(UART0, "client: returned\r\n");
            }
        }
    }
}
