#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// USART1 on STM32F103: base 0x4001_3800 (same as uart1 in chip config)
// SR offset 0x00, DR offset 0x04, CR1 offset 0x0C
const UART1_BASE: u32 = 0x4001_3800;
const UART1_SR: *const u32 = (UART1_BASE + 0x00) as *const u32;
const UART1_DR: *mut u32 = (UART1_BASE + 0x04) as *mut u32;

// USART2: base 0x4000_4400 — used as debug output
const UART2_BASE: u32 = 0x4000_4400;
const UART2_SR: *const u32 = (UART2_BASE + 0x00) as *const u32;
const UART2_DR: *mut u32 = (UART2_BASE + 0x04) as *mut u32;

// SR bits
const SR_RXNE: u32 = 1 << 5; // RX Not Empty
const SR_TXE: u32 = 1 << 7; // TX Empty (ready)

fn uart2_byte(byte: u8) {
    unsafe {
        // Wait until TX is ready
        for _ in 0..256 {
            if core::ptr::read_volatile(UART2_SR) & SR_TXE != 0 {
                break;
            }
        }
        core::ptr::write_volatile(UART2_DR, byte as u32);
    }
}

fn uart2_str(s: &str) {
    for b in s.bytes() {
        uart2_byte(b);
    }
}

fn uart1_has_data() -> bool {
    unsafe { core::ptr::read_volatile(UART1_SR) & SR_RXNE != 0 }
}

fn uart1_read_byte() -> u8 {
    unsafe { (core::ptr::read_volatile(UART1_DR) & 0xFF) as u8 }
}

/// Read one complete NMEA sentence from UART1 into `buf`.
/// Returns the length of the sentence (including the trailing \n), or 0 on overflow.
fn read_nmea_sentence(buf: &mut [u8]) -> usize {
    let mut len = 0;
    loop {
        // Poll for next byte (busy-wait with iteration limit to avoid infinite hang)
        let byte = loop {
            let mut attempts = 0u32;
            if uart1_has_data() {
                break uart1_read_byte();
            }
            attempts += 1;
            if attempts > 2_000_000 {
                return 0; // timeout
            }
        };

        if len >= buf.len() {
            return 0; // buffer overflow — discard
        }
        buf[len] = byte;
        len += 1;

        // NMEA sentences end with \n (preceded by \r)
        if byte == b'\n' {
            return len;
        }
    }
}

#[entry]
fn main() -> ! {
    uart2_str("NEO-6M GPS Lab\r\n");
    uart2_str("Reading NMEA stream from UART1...\r\n");

    let mut sentence_buf = [0u8; 128];
    let mut count = 0u32;

    loop {
        let len = read_nmea_sentence(&mut sentence_buf);
        if len == 0 {
            continue;
        }

        count += 1;

        // Echo the raw NMEA sentence to UART2 with a prefix
        uart2_str("[GPS] ");
        for &b in &sentence_buf[..len] {
            uart2_byte(b);
        }
    }
}
