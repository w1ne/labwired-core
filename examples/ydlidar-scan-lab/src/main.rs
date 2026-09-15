#![no_std]
#![no_main]
#![allow(clippy::identity_op)]

//! 360° scanning lidar lab.
//!
//! USART1 receives the scanner's frame stream; USART2 is the console. The
//! firmware reassembles frames, validates the XOR-16 checksum itself, and
//! prints one line per revolution naming the nearest return and its bearing.
//!
//! The point of printing the *nearest bearing* rather than a byte count is
//! that it exercises the whole chain: framing, the quarter-millimetre distance
//! word, angle interpolation between FSA and LSA, and the angle correction. A
//! model that got any of those wrong still produces a plausible byte count.

use cortex_m_rt::entry;
use panic_halt as _;

const RCC_BASE: u32 = 0x4002_1000;
const RCC_APB2ENR: *mut u32 = (RCC_BASE + 0x18) as *mut u32;
const RCC_APB1ENR: *mut u32 = (RCC_BASE + 0x1C) as *mut u32;
const GPIOA_CRL: *mut u32 = (0x4001_0800 + 0x00) as *mut u32;
const GPIOA_CRH: *mut u32 = (0x4001_0800 + 0x04) as *mut u32;

const UART1_BASE: u32 = 0x4001_3800;
const UART1_SR: *const u32 = (UART1_BASE + 0x00) as *const u32;
const UART1_DR: *mut u32 = (UART1_BASE + 0x04) as *mut u32;
const UART1_BRR: *mut u32 = (UART1_BASE + 0x08) as *mut u32;
const UART1_CR1: *mut u32 = (UART1_BASE + 0x0C) as *mut u32;

const UART2_BASE: u32 = 0x4000_4400;
const UART2_SR: *const u32 = (UART2_BASE + 0x00) as *const u32;
const UART2_DR: *mut u32 = (UART2_BASE + 0x04) as *mut u32;
const UART2_BRR: *mut u32 = (UART2_BASE + 0x08) as *mut u32;
const UART2_CR1: *mut u32 = (UART2_BASE + 0x0C) as *mut u32;

const SR_RXNE: u32 = 1 << 5;
const SR_TXE: u32 = 1 << 7;

/// Enable AFIO, GPIOA, USART1 (APB2) and USART2 (APB1). Unclocked MMIO is
/// dropped by the model and the port stays in reset, so the pad mux below
/// would be swallowed without this.
fn enable_peripheral_clocks() {
    unsafe {
        let apb2 = core::ptr::read_volatile(RCC_APB2ENR);
        core::ptr::write_volatile(RCC_APB2ENR, apb2 | (1 << 0) | (1 << 2) | (1 << 14));
        let apb1 = core::ptr::read_volatile(RCC_APB1ENR);
        core::ptr::write_volatile(RCC_APB1ENR, apb1 | (1 << 17));
    }
}

/// Mux the serial pads and set both baud divisors.
///
/// Pads are the DS5319 Rev 20 Table 5 defaults, no AFIO remap: PA9 =
/// USART1_TX (`0xB` = AF push-pull), PA10 = USART1_RX (`0x4` = floating
/// input), PA2 = USART2_TX (`0xB`).
///
/// `BRR` = f_PCLK / baud. Nothing here touches the PLL, so the part runs on
/// the 8 MHz HSI it selects at reset and both APB prescalers are 1.
///
/// * USART1 at **230400** — the scanner's link rate: 8_000_000 / 230_400 =
///   34.72 → 35 = 0x23. That is 0.8% fast, inside the ~2% an 8N1 receiver
///   tolerates over a 10-bit character.
/// * USART2 at 115200 (console): 8_000_000 / 115_200 = 69.44 → 69 = 0x45.
fn serial_init() {
    unsafe {
        let crl = core::ptr::read_volatile(GPIOA_CRL);
        core::ptr::write_volatile(GPIOA_CRL, (crl & !(0xF << 8)) | (0xB << 8));
        let crh = core::ptr::read_volatile(GPIOA_CRH);
        let crh = (crh & !(0xFF << 4)) | (0xB << 4) | (0x4 << 8);
        core::ptr::write_volatile(GPIOA_CRH, crh);

        core::ptr::write_volatile(UART1_BRR, 0x23);
        // UE | TE | RE
        core::ptr::write_volatile(UART1_CR1, (1 << 13) | (1 << 3) | (1 << 2));

        core::ptr::write_volatile(UART2_BRR, 0x45);
        // UE | TE — console is transmit only.
        core::ptr::write_volatile(UART2_CR1, (1 << 13) | (1 << 3));
    }
}

fn uart2_byte(byte: u8) {
    unsafe {
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

/// Print an unsigned value without `core::fmt` — this is a `no_std` binary and
/// pulling in the formatting machinery would dominate its size.
fn uart2_u32(mut v: u32) {
    let mut buf = [0u8; 10];
    let mut n = 0;
    if v == 0 {
        uart2_byte(b'0');
        return;
    }
    while v > 0 {
        buf[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    while n > 0 {
        n -= 1;
        uart2_byte(buf[n]);
    }
}

/// Blocking read of one byte, with a bounded spin so a stalled link cannot
/// hang the lab forever.
fn uart1_read_byte() -> Option<u8> {
    for _ in 0..2_000_000u32 {
        unsafe {
            if core::ptr::read_volatile(UART1_SR) & SR_RXNE != 0 {
                return Some((core::ptr::read_volatile(UART1_DR) & 0xFF) as u8);
            }
        }
    }
    None
}

const HEADER_LO: u8 = 0xAA;
const HEADER_HI: u8 = 0x55;
const MAX_SAMPLES: usize = 40;

struct Frame {
    ct: u8,
    fsa: u16,
    lsa: u16,
    count: usize,
    /// `(intensity, distance_word)` in wire order.
    samples: [(u8, u16); MAX_SAMPLES],
}

/// Read bytes until a checksum-clean frame is assembled.
///
/// On a checksum failure the whole frame is dropped and the search restarts at
/// the next header — a corrupt LSN must never be trusted to size the read.
fn read_frame() -> Option<Frame> {
    loop {
        // Hunt for the header.
        if uart1_read_byte()? != HEADER_LO {
            continue;
        }
        if uart1_read_byte()? != HEADER_HI {
            continue;
        }
        let ct = uart1_read_byte()?;
        let lsn = uart1_read_byte()? as usize;
        if lsn == 0 || lsn > MAX_SAMPLES {
            continue;
        }
        let fsa = u16::from(uart1_read_byte()?) | (u16::from(uart1_read_byte()?) << 8);
        let lsa = u16::from(uart1_read_byte()?) | (u16::from(uart1_read_byte()?) << 8);
        let cs = u16::from(uart1_read_byte()?) | (u16::from(uart1_read_byte()?) << 8);

        let mut samples = [(0u8, 0u16); MAX_SAMPLES];
        let mut sum: u16 = 0x55AA ^ (ct as u16 | ((lsn as u16) << 8)) ^ fsa;
        for slot in samples.iter_mut().take(lsn) {
            let intensity = uart1_read_byte()?;
            let distance = u16::from(uart1_read_byte()?) | (u16::from(uart1_read_byte()?) << 8);
            *slot = (intensity, distance);
            sum ^= distance;
            sum ^= intensity as u16;
        }
        sum ^= lsa;
        if sum != cs {
            continue;
        }
        return Some(Frame {
            ct,
            fsa,
            lsa,
            count: lsn,
            samples,
        });
    }
}

/// Angle field to hundredths of a degree. The field is `deg * 64` shifted left
/// one, the low bit being a check bit: `(raw >> 1) * 100 / 64`.
fn angle_centideg(raw: u16) -> u32 {
    (raw as u32 >> 1) * 100 / 64
}

/// The angle correction, in centidegrees, as a first-order approximation.
///
/// A frame carries the MECHANICAL angle of the head. The bearing the beam
/// actually travelled is `mech + atan(21.8 * (155.3 - d) / (155.3 * d))`,
/// because the emitter sits off the rotation axis. It is not a small term: it
/// runs from about -1.8 deg at 200 mm to -7.8 deg far away.
///
/// Over this scanner's whole range the argument stays under 0.14, and there
/// `atan(x) ~ x` is good to 0.05 deg, so the transcendental collapses to one
/// rational expression:
///
///   correction_cdeg ~ 804 * (155.3 - d) / d,   d in mm
///
/// That matters here: this is a Cortex-M3 with no FPU, and pulling in a
/// software `atan` would dwarf the rest of the firmware.
fn angle_correction_cdeg(mm: u32) -> i32 {
    if mm == 0 {
        return 0;
    }
    let d = mm as i64;
    ((804 * (1553 - 10 * d)) / (10 * d)) as i32
}

#[entry]
fn main() -> ! {
    enable_peripheral_clocks();
    serial_init();
    uart2_str("360 Scanning Lidar Lab\r\n");
    uart2_str("Reading scan frames from UART1 at 230400...\r\n");

    let mut revolution: u32 = 0;
    let mut points: u32 = 0;
    let mut near_mm: u32 = u32::MAX;
    let mut near_centideg: u32 = 0;

    loop {
        let Some(frame) = read_frame() else {
            uart2_str("[SCAN] link idle\r\n");
            continue;
        };

        // Bit 0 of CT marks the start of a revolution; the rest is the spin
        // rate in tenths of a hertz.
        if frame.ct & 1 == 1 {
            if points > 0 {
                uart2_str("[SCAN] rev=");
                uart2_u32(revolution);
                uart2_str(" pts=");
                uart2_u32(points);
                uart2_str(" near=");
                uart2_u32(near_mm);
                uart2_str("mm@");
                uart2_u32(near_centideg / 100);
                uart2_str("deg spin=");
                uart2_u32((frame.ct >> 1) as u32);
                uart2_str("dHz\r\n");
            }
            revolution += 1;
            points = 0;
            near_mm = u32::MAX;
            near_centideg = 0;
            continue;
        }

        // Interpolate bearings linearly between FSA and LSA, in centidegrees
        // so this stays integer-only.
        let first = angle_centideg(frame.fsa);
        let last = angle_centideg(frame.lsa);
        let span = (last + 36_000 - first) % 36_000;
        let divisor = if frame.count > 1 {
            (frame.count - 1) as u32
        } else {
            1
        };

        for (i, &(_, distance)) in frame.samples.iter().take(frame.count).enumerate() {
            // Raw 0 and 1 are the invalid and no-return markers.
            if distance <= 1 {
                continue;
            }
            let mm = distance as u32 / 4;
            points += 1;
            if mm < near_mm {
                near_mm = mm;
                let mech = (first + span * i as u32 / divisor) % 36_000;
                let corrected = mech as i32 + angle_correction_cdeg(mm);
                near_centideg = corrected.rem_euclid(36_000) as u32;
            }
        }
    }
}
