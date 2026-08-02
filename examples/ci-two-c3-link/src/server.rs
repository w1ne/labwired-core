//! Fixture node A: serves PING on UART1, reports each completed rally on UART0.

#![no_std]
#![no_main]

#[macro_use]
mod rt;
mod uart;

use uart::{put_dec, puts, try_get, UART0, UART1};

bare_metal_entry!(main);

fn main() -> ! {
    puts(UART0, "server up\r\n");
    const PONG: &[u8] = b"PONG\n";
    let mut rounds: u32 = 0;

    loop {
        puts(UART1, "PING\n");

        // Bounded, so a broken link fails loudly instead of running out the
        // step budget and looking like a hang.
        let mut budget = 2_000_000u32;
        let mut matched = 0usize;
        while matched < PONG.len() && budget > 0 {
            budget -= 1;
            if let Some(b) = try_get(UART1) {
                matched = if b == PONG[matched] { matched + 1 } else { 0 };
            }
        }
        if matched < PONG.len() {
            puts(UART0, "server: no PONG\r\n");
            loop {}
        }

        rounds += 1;
        puts(UART0, "rally ");
        put_dec(UART0, rounds);
        puts(UART0, "\r\n");
        if rounds == 3 {
            puts(UART0, "server done\r\n");
            loop {}
        }
    }
}
