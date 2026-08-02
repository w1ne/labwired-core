//! Bare-metal ESP32-C3 UART.
//!
//! Deliberately no HAL: this drives the registers the C3 TRM documents, so the
//! rally exercises the simulator's own `esp_uart` model rather than an
//! abstraction that could paper over a gap.

/// UART0 — the USB console each node prints commentary on.
pub const UART0: u32 = 0x6000_0000;
/// UART1 — the wire between the two chips. This is the instance Arduino's
/// `Serial1` opens, and what the diagram's TX/RX wires resolve to.
pub const UART1: u32 = 0x6001_0000;

const OFF_FIFO: u32 = 0x00;
const OFF_STATUS: u32 = 0x1C;

#[inline(always)]
fn read(base: u32, off: u32) -> u32 {
    unsafe { core::ptr::read_volatile((base + off) as *const u32) }
}

#[inline(always)]
fn write(base: u32, off: u32, val: u32) {
    unsafe { core::ptr::write_volatile((base + off) as *mut u32, val) }
}

/// STATUS[9:0] = rxfifo_cnt.
pub fn rx_count(base: u32) -> u32 {
    read(base, OFF_STATUS) & 0x3FF
}

/// STATUS[25:16] = txfifo_cnt.
pub fn tx_count(base: u32) -> u32 {
    (read(base, OFF_STATUS) >> 16) & 0x3FF
}

pub fn put(base: u32, byte: u8) {
    // Leave headroom below the 128-entry FIFO rather than filling it exactly.
    while tx_count(base) >= 120 {}
    write(base, OFF_FIFO, byte as u32);
}

pub fn puts(base: u32, s: &str) {
    for b in s.as_bytes() {
        put(base, *b);
    }
}

pub fn put_dec(base: u32, mut n: u32) {
    if n == 0 {
        put(base, b'0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = buf.len();
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    for b in &buf[i..] {
        put(base, *b);
    }
}

/// One byte if the RX FIFO has one, else `None`.
///
/// Never blocks: both roles must stay responsive while the peer's bytes are
/// still in flight on the wire.
pub fn try_get(base: u32) -> Option<u8> {
    if rx_count(base) == 0 {
        return None;
    }
    Some((read(base, OFF_FIFO) & 0xFF) as u8)
}
