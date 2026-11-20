#![no_std]
#![no_main]

use core::ptr::write_volatile;
use cortex_m_rt::entry;
use panic_halt as _;

const UART0_BASE: u32 = 0x4010_8000;

const UART_PWREN: *mut u32 = (UART0_BASE + 0x800) as *mut u32;
const UART_CTL0: *mut u32 = (UART0_BASE + 0x1100) as *mut u32;
const UART_LCRH: *mut u32 = (UART0_BASE + 0x1104) as *mut u32;
const UART_STAT: *const u32 = (UART0_BASE + 0x1108) as *const u32;
const UART_IBRD: *mut u32 = (UART0_BASE + 0x1110) as *mut u32;
const UART_FBRD: *mut u32 = (UART0_BASE + 0x1114) as *mut u32;
const UART_TXDATA: *mut u32 = (UART0_BASE + 0x1120) as *mut u32;

const UART_ENABLE: u32 = 1 << 0;
const UART_TX_ENABLE: u32 = 1 << 4;
const UART_TXFE: u32 = 1 << 6;
const UART_PWREN_KEY_ENABLE: u32 = 0x2600_0001;

fn putc(byte: u8) {
    // Bound the poll so a missing status model cannot hang the smoke forever.
    for _ in 0..100_000 {
        if unsafe { core::ptr::read_volatile(UART_STAT) } & UART_TXFE != 0 {
            unsafe { write_volatile(UART_TXDATA, u32::from(byte)) };
            return;
        }
    }
}

fn puts(message: &[u8]) {
    for &byte in message {
        putc(byte);
    }
}

#[entry]
fn main() -> ! {
    unsafe {
        // UART GPRCM PWREN key, followed by 115200 8N1 from the reset 32 MHz
        // BUSCLK. Integer/fractional divisor = 17 + 23/64.
        write_volatile(UART_PWREN, UART_PWREN_KEY_ENABLE);
        write_volatile(UART_IBRD, 17);
        write_volatile(UART_FBRD, 23);
        write_volatile(UART_LCRH, 3 << 4); // 8-bit words, FIFO disabled.
        write_volatile(UART_CTL0, UART_ENABLE | UART_TX_ENABLE);
    }

    puts(b"LP_MSPM0G3507_UART0_OK\r\n");
    loop {
        cortex_m::asm::nop();
    }
}
