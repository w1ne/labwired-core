//! Allocation-free status view and nRF52840 TWIM0 SSD1306 driver.

use crate::{flags, live, ScannerState};
use core::{
    cell::UnsafeCell,
    ptr::{read_volatile, write_volatile},
    sync::atomic::{AtomicBool, Ordering},
};

pub const LINE_CAPACITY: usize = 21;
pub const TWIM_EVENTS_STOPPED: usize = 0x104;
pub const TWIM_EVENTS_ERROR: usize = 0x124;
pub const TWIM_DMA_STATIC: bool = true;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AsciiLine {
    bytes: [u8; LINE_CAPACITY],
    len: u8,
}

impl AsciiLine {
    const fn new() -> Self {
        Self {
            bytes: [0; LINE_CAPACITY],
            len: 0,
        }
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
    fn push(&mut self, byte: u8) {
        if (self.len as usize) < LINE_CAPACITY {
            self.bytes[self.len as usize] = byte;
            self.len += 1;
        }
    }
    fn text(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.push(b);
        }
    }
    fn number(&mut self, value: i32) {
        if value < 0 {
            self.push(b'-');
        }
        let mut n = value.unsigned_abs();
        let mut digits = [0u8; 10];
        let mut count = 0;
        loop {
            digits[count] = (n % 10) as u8 + b'0';
            count += 1;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        while count > 0 {
            count -= 1;
            self.push(digits[count]);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayView {
    pub lines: [AsciiLine; 4],
    pub status: AsciiLine,
}

impl DisplayView {
    pub fn from_state(state: &ScannerState) -> Self {
        let mut lines = [AsciiLine::new(); 4];
        lines[0].text(b"RPM ");
        if state.live_valid & live::RPM != 0 {
            lines[0].number(state.rpm as i32);
        } else {
            lines[0].text(b"--");
        }
        lines[1].text(b"SPD ");
        if state.live_valid & live::SPEED != 0 {
            lines[1].number(state.speed_kph as i32);
        } else {
            lines[1].text(b"--");
        }
        lines[1].text(b" km/h");
        lines[2].text(b"TEMP ");
        if state.live_valid & live::COOLANT != 0 {
            lines[2].number(state.coolant_c as i32);
        } else {
            lines[2].text(b"--");
        }
        lines[2].text(b" C");
        lines[3].text(b"DTC ");
        lines[3].number(state.dtc_count as i32);
        let mut status = AsciiLine::new();
        if state.status_flags & flags::DEVICE_ERROR != 0 {
            if state.status_flags & flags::TIMEOUT != 0
                && state.status_flags & flags::CAN_CONFIG_ERROR != 0
            {
                status.text(b"STALE TO CAN DEV ERR");
            } else {
                status.text(b"DEV ERR");
            }
        }
        // Error composition is ordered and its maximum is exactly 21 bytes.
        for (set, label) in if status.as_bytes().is_empty() {
            [
                (state.status_flags & flags::STALE != 0, b"STALE" as &[u8]),
                (state.status_flags & flags::TIMEOUT != 0, b"TIMEOUT"),
                (
                    state.status_flags & flags::CAN_CONFIG_ERROR != 0,
                    b"CAN ERR",
                ),
            ]
        } else {
            [(false, b"" as &[u8]); 3]
        } {
            if set {
                if !status.as_bytes().is_empty() {
                    status.push(b' ');
                }
                status.text(label);
            }
        }
        if status.as_bytes().is_empty() {
            status.text(
                if state.live_valid & state.required_live != state.required_live {
                    b"INIT"
                } else {
                    b"OK"
                },
            );
        }
        Self { lines, status }
    }
}

const TWIM: usize = 0x4000_3000;
const WAIT_LIMIT: u32 = 100_000;
const ADDRESS: u32 = 0x3c;

pub struct Ssd1306 {
    framebuffer: [u8; 1024],
    stuck: bool,
}

struct DmaCell(UnsafeCell<[u8; 17]>);
unsafe impl Sync for DmaCell {}
static TWIM_TX: DmaCell = DmaCell(UnsafeCell::new([0; 17]));
static TAKEN: AtomicBool = AtomicBool::new(false);

impl Ssd1306 {
    /// Claims the sole non-reentrant TWIM0/static-DMA-buffer instance.
    pub fn take() -> Option<Self> {
        TAKEN
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| Self {
                framebuffer: [0; 1024],
                stuck: false,
            })
    }
    pub fn init(&mut self) -> bool {
        unsafe {
            wr(0x5000_0000, 0x700 + 26 * 4, 0x0000_0600);
            wr(0x5000_0000, 0x700 + 27 * 4, 0x0000_0600);
            wr(TWIM, 0x508, 26);
            wr(TWIM, 0x50c, 27);
            wr(TWIM, 0x524, 0x0668_0000);
            wr(TWIM, 0x588, ADDRESS);
            wr(TWIM, 0x200, 1 << 9); // LASTTX -> STOP, yielding bounded STOPPED.
            wr(TWIM, 0x500, 6);
        }
        self.command(&[
            0xae, 0x20, 0x00, 0xa8, 0x3f, 0xd3, 0x00, 0x40, 0xa1, 0xc8, 0xda, 0x12, 0x81, 0x7f,
            0xaf,
        ])
    }
    pub fn render(&mut self, view: &DisplayView) {
        self.framebuffer.fill(0);
        for (row, line) in view.lines.iter().enumerate() {
            self.draw_line(row, line.as_bytes());
        }
        self.draw_line(5, view.status.as_bytes());
    }
    pub fn update(&mut self) -> bool {
        if !self.command(&[0x21, 0, 127, 0x22, 0, 7]) {
            return false;
        }
        for offset in (0..self.framebuffer.len()).step_by(16) {
            dma_tx_set(0, 0x40);
            for index in 0..16 {
                dma_tx_set(1 + index, self.framebuffer[offset + index]);
            }
            if !self.write_tx(17) {
                return false;
            }
        }
        true
    }
    fn command(&mut self, bytes: &[u8]) -> bool {
        dma_tx_set(0, 0);
        for (index, byte) in bytes.iter().enumerate() {
            dma_tx_set(1 + index, *byte);
        }
        self.write_tx(1 + bytes.len())
    }
    fn draw_line(&mut self, page: usize, bytes: &[u8]) {
        for (column, &ch) in bytes.iter().take(21).enumerate() {
            let glyph = glyph(ch);
            let start = page * 128 + column * 6;
            self.framebuffer[start..start + 5].copy_from_slice(&glyph);
        }
    }
    fn write_tx(&mut self, len: usize) -> bool {
        if self.stuck {
            return false;
        }
        unsafe {
            wr(TWIM, TWIM_EVENTS_STOPPED, 0);
            wr(TWIM, TWIM_EVENTS_ERROR, 0);
            wr(TWIM, 0x14c, 0);
            wr(TWIM, 0x544, TWIM_TX.0.get() as *mut u8 as u32);
            wr(TWIM, 0x548, len as u32);
            wr(TWIM, 0x008, 1);
            for _ in 0..WAIT_LIMIT {
                if rd(TWIM, TWIM_EVENTS_ERROR) != 0 {
                    return self.stop_after_failure();
                }
                if rd(TWIM, TWIM_EVENTS_STOPPED) != 0 {
                    return true;
                }
            }
            self.stop_after_failure()
        }
    }
    unsafe fn stop_after_failure(&mut self) -> bool {
        wr(TWIM, TWIM_EVENTS_STOPPED, 0);
        wr(TWIM, 0x014, 1);
        for _ in 0..WAIT_LIMIT {
            if rd(TWIM, TWIM_EVENTS_STOPPED) != 0 {
                return false;
            }
        }
        // DMA source is driver-owned and remains allocated; poison future use.
        self.stuck = true;
        false
    }
}
fn dma_tx_set(index: usize, value: u8) {
    unsafe {
        (*TWIM_TX.0.get())[index] = value;
    }
}

// Compact deterministic 5x7 glyphs sufficient for the scanner's ASCII UI.
fn glyph(c: u8) -> [u8; 5] {
    match c {
        b'0'..=b'9' => DIGITS[(c - b'0') as usize],
        b'-' => [8, 8, 8, 8, 8],
        b'/' => [32, 16, 8, 4, 2],
        b' ' => [0; 5],
        b'A' => [126, 17, 17, 17, 126],
        b'C' => [62, 65, 65, 65, 34],
        b'D' => [127, 65, 65, 34, 28],
        b'E' => [127, 73, 73, 73, 65],
        b'H' => [127, 8, 8, 8, 127],
        b'I' => [0, 65, 127, 65, 0],
        b'L' => [127, 64, 64, 64, 64],
        b'M' => [127, 2, 12, 2, 127],
        b'N' => [127, 4, 8, 16, 127],
        b'O' => [62, 65, 65, 65, 62],
        b'P' => [127, 9, 9, 9, 6],
        b'R' => [127, 9, 25, 41, 70],
        b'S' => [70, 73, 73, 73, 49],
        b'T' => [1, 1, 127, 1, 1],
        b'U' => [63, 64, 64, 64, 63],
        b'k' => [127, 8, 20, 34, 65],
        b'm' => [124, 4, 24, 4, 120],
        b'h' => [127, 8, 4, 4, 120],
        _ => [0; 5],
    }
}
const DIGITS: [[u8; 5]; 10] = [
    [62, 81, 73, 69, 62],
    [0, 66, 127, 64, 0],
    [66, 97, 81, 73, 70],
    [33, 65, 69, 75, 49],
    [24, 20, 18, 127, 16],
    [39, 69, 69, 69, 57],
    [60, 74, 73, 73, 48],
    [1, 113, 9, 5, 3],
    [54, 73, 73, 73, 54],
    [6, 73, 73, 41, 30],
];

unsafe fn wr(base: usize, offset: usize, value: u32) {
    write_volatile((base + offset) as *mut u32, value)
}
unsafe fn rd(base: usize, offset: usize) -> u32 {
    read_volatile((base + offset) as *const u32)
}
