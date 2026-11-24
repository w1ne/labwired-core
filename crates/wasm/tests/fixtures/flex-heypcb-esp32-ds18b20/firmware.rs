// SPDX-License-Identifier: MIT
// Portable DS18B20 thermometer: bit-bang the 1-Wire line at standard speed
// (Maxim AN126 slot times), read and check the ROM code, write the alarm
// thresholds and the resolution to the scratchpad and read them back. Each
// pass starts a conversion, polls the busy status with read slots, reads the
// scratchpad, checks its CRC, drives an over-temperature indicator with the
// TH/TL hysteresis, and prints the reading to the console.
// Nothing here names an MCU: the line, the indicator and the console are host
// ports.
#![no_std]

#[link(wasm_import_module = "labwired")]
extern "C" {
    fn read(port: u32) -> f64;
    fn write(port: u32, value: f64);
    fn advance_ns(delta: u32);
    fn console_write(ptr: *const u8, len: u32);
}

/// One console line, built without an allocator.
struct Line {
    bytes: [u8; 64],
    len: usize,
}

impl Line {
    fn new() -> Self {
        Self { bytes: [0; 64], len: 0 }
    }

    fn text(&mut self, text: &str) -> &mut Self {
        for &byte in text.as_bytes() {
            self.bytes[self.len] = byte;
            self.len += 1;
        }
        self
    }

    fn number(&mut self, value: u32) -> &mut Self {
        let mut digits = [0u8; 10];
        let (mut value, mut count) = (value, 0);
        loop {
            digits[count] = b'0' + (value % 10) as u8;
            count += 1;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        for index in (0..count).rev() {
            self.bytes[self.len] = digits[index];
            self.len += 1;
        }
        self
    }

    /// A count of 1/16 °C, as `-10.1250`.
    fn sixteenths(&mut self, raw: i16) -> &mut Self {
        if raw < 0 {
            self.text("-");
        }
        let ten_thousandths = u32::from(raw.unsigned_abs()) * 625;
        self.number(ten_thousandths / 10_000).text(".");
        let fraction = ten_thousandths % 10_000;
        for scale in [1_000, 100, 10, 1] {
            self.number(fraction / scale % 10);
        }
        self
    }

    /// Two hexadecimal digits, with no prefix.
    fn byte(&mut self, value: u8) -> &mut Self {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        self.bytes[self.len] = DIGITS[usize::from(value >> 4)];
        self.bytes[self.len + 1] = DIGITS[usize::from(value & 0xf)];
        self.len += 2;
        self
    }

    fn print(&mut self) {
        self.text("\n");
        unsafe { console_write(self.bytes.as_ptr(), self.len as u32) };
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}

const READ_ROM: u8 = 0x33;
const SKIP_ROM: u8 = 0xCC;
const CONVERT_T: u8 = 0x44;
const WRITE_SCRATCHPAD: u8 = 0x4E;
const READ_SCRATCHPAD: u8 = 0xBE;

/// Alarm thresholds written to TH and TL, in whole °C.
const ALARM_HIGH_C: u8 = 30;
const ALARM_LOW_C: u8 = 28;
/// Busy polls, 1.07 ms apart, before a conversion counts as lost: longer
/// than the 750 ms of a 12-bit conversion.
const MAX_POLLS: u32 = 1_000;

/// `setup` results other than 0.
const ERR_NO_PRESENCE: u32 = 1;
const ERR_ROM_CRC: u32 = 2;
const ERR_FAMILY: u32 = 3;
const ERR_SCRATCHPAD: u32 = 4;
/// `loop` results that are not a sample.
const LOOP_NO_SENSOR: u32 = 0x8000;
const LOOP_TIMEOUT: u32 = 0x8001;
const LOOP_CRC: u32 = 0x8002;

struct Thermometer {
    drive: u32,
    line: u32,
    led: u32,
    ready: bool,
    alarm: bool,
    high: i16,
    low: i16,
}

static mut THERMOMETER: Thermometer = Thermometer {
    drive: 0,
    line: 0,
    led: 0,
    ready: false,
    alarm: false,
    high: 0,
    low: 0,
};

fn us(delay: u32) {
    unsafe { advance_ns(delay * 1_000) };
}

// ── 1-Wire, standard speed (Maxim AN126) ────────────────────────────────

impl Thermometer {
    fn release(&self, high: bool) {
        unsafe { write(self.drive, if high { 1.0 } else { 0.0 }) };
    }
    fn sample(&self) -> bool {
        unsafe { read(self.line) != 0.0 }
    }
    /// Reset pulse; true when a part answers with a presence pulse.
    fn reset(&self) -> bool {
        self.release(false);
        us(480);
        self.release(true);
        us(70);
        let presence = !self.sample();
        us(410);
        presence
    }
    fn write_bit(&self, bit: bool) {
        self.release(false);
        us(if bit { 6 } else { 60 });
        self.release(true);
        us(if bit { 64 } else { 10 });
    }
    fn read_bit(&self) -> bool {
        self.release(false);
        us(3);
        self.release(true);
        us(10);
        let bit = self.sample();
        us(57);
        bit
    }
    fn write_byte(&self, byte: u8) {
        for bit in 0..8 {
            self.write_bit(byte >> bit & 1 == 1);
        }
    }
    fn read_byte(&self) -> u8 {
        let mut byte = 0;
        for bit in 0..8 {
            byte |= u8::from(self.read_bit()) << bit;
        }
        byte
    }
    /// SKIP ROM, READ SCRATCHPAD; `None` without a part or with a bad CRC.
    fn read_scratchpad(&self) -> Option<[u8; 9]> {
        if !self.reset() {
            return None;
        }
        self.write_byte(SKIP_ROM);
        self.write_byte(READ_SCRATCHPAD);
        let mut scratchpad = [0u8; 9];
        for byte in scratchpad.iter_mut() {
            *byte = self.read_byte();
        }
        (crc8(&scratchpad[..8]) == scratchpad[8]).then_some(scratchpad)
    }
}

/// Dallas/Maxim CRC-8: polynomial x^8 + x^5 + x^4 + 1, LSB first.
fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &byte in bytes {
        let mut b = byte;
        for _ in 0..8 {
            let mix = (crc ^ b) & 1;
            crc >>= 1;
            if mix != 0 {
                crc ^= 0x8C;
            }
            b >>= 1;
        }
    }
    crc
}

/// Read the ROM code of the one part on the line, check its CRC and family,
/// write TH, TL and the resolution, and read them back. 0: ready.
#[no_mangle]
pub extern "C" fn setup(drive: u32, line: u32, led: u32, family: u32, resolution_bits: u32) -> u32 {
    let thermometer = unsafe { &mut *core::ptr::addr_of_mut!(THERMOMETER) };
    thermometer.drive = drive;
    thermometer.line = line;
    thermometer.led = led;
    thermometer.ready = false;
    thermometer.alarm = false;
    thermometer.release(true);

    if !thermometer.reset() {
        Line::new().text("no 1-Wire presence").print();
        return ERR_NO_PRESENCE;
    }
    thermometer.write_byte(READ_ROM);
    let mut rom = [0u8; 8];
    for byte in rom.iter_mut() {
        *byte = thermometer.read_byte();
    }
    if crc8(&rom[..7]) != rom[7] {
        Line::new().text("ROM CRC error").print();
        return ERR_ROM_CRC;
    }
    let mut line = Line::new();
    line.text("ROM");
    for byte in rom {
        line.text(" ").byte(byte);
    }
    line.print();
    if u32::from(rom[0]) != family {
        Line::new().text("family ").byte(rom[0]).text(", expected ").byte(family as u8).print();
        return ERR_FAMILY;
    }

    let config = ((resolution_bits.clamp(9, 12) - 9) << 5 | 0x1F) as u8;
    thermometer.reset();
    thermometer.write_byte(SKIP_ROM);
    thermometer.write_byte(WRITE_SCRATCHPAD);
    thermometer.write_byte(ALARM_HIGH_C);
    thermometer.write_byte(ALARM_LOW_C);
    thermometer.write_byte(config);
    let Some(scratchpad) = thermometer.read_scratchpad() else {
        Line::new().text("scratchpad CRC error").print();
        return ERR_SCRATCHPAD;
    };
    if scratchpad[2] != ALARM_HIGH_C || scratchpad[3] != ALARM_LOW_C || scratchpad[4] != config {
        Line::new().text("scratchpad write not kept").print();
        return ERR_SCRATCHPAD;
    }
    thermometer.high = i16::from(scratchpad[2] as i8) * 16;
    thermometer.low = i16::from(scratchpad[3] as i8) * 16;
    thermometer.ready = true;
    Line::new()
        .text("DS18B20 ")
        .number(u32::from(scratchpad[4] >> 5 & 3) + 9)
        .text("-bit, TH ")
        .number(u32::from(scratchpad[2]))
        .text(" C, TL ")
        .number(u32::from(scratchpad[3]))
        .text(" C")
        .print();
    0
}

/// Read the scratchpad without a conversion. Returns the temperature count
/// it holds, or `LOOP_CRC`.
#[no_mangle]
pub extern "C" fn peek() -> u32 {
    let thermometer = unsafe { &*core::ptr::addr_of!(THERMOMETER) };
    match thermometer.read_scratchpad() {
        Some(scratchpad) => {
            let raw = i16::from_le_bytes([scratchpad[0], scratchpad[1]]);
            Line::new().text("scratchpad T=").sixteenths(raw).text(" C").print();
            u32::from(raw as u16)
        }
        None => LOOP_CRC,
    }
}

/// One measurement: convert, poll busy, read, update the indicator. Returns
/// the temperature count in 1/16 °C as 16 bits, plus 0x10000 while the
/// indicator is on.
#[export_name = "loop"]
pub extern "C" fn measure() -> u32 {
    let thermometer = unsafe { &mut *core::ptr::addr_of_mut!(THERMOMETER) };
    if !thermometer.ready || !thermometer.reset() {
        return LOOP_NO_SENSOR;
    }
    thermometer.write_byte(SKIP_ROM);
    thermometer.write_byte(CONVERT_T);
    // An externally powered part answers read slots with 0 while it converts.
    let mut polls = 0;
    while !thermometer.read_bit() {
        polls += 1;
        if polls >= MAX_POLLS {
            Line::new().text("conversion timeout").print();
            return LOOP_TIMEOUT;
        }
        us(1_000);
    }
    let Some(scratchpad) = thermometer.read_scratchpad() else {
        Line::new().text("scratchpad CRC error").print();
        return LOOP_CRC;
    };
    let raw = i16::from_le_bytes([scratchpad[0], scratchpad[1]]);
    if raw >= thermometer.high {
        thermometer.alarm = true;
    } else if raw < thermometer.low {
        thermometer.alarm = false;
    }
    unsafe { write(thermometer.led, if thermometer.alarm { 1.0 } else { 0.0 }) };
    let mut line = Line::new();
    line.text("T=").sixteenths(raw).text(" C after ").number(polls).text(" polls");
    if thermometer.alarm {
        line.text(" HIGH");
    }
    line.print();
    u32::from(raw as u16) | if thermometer.alarm { 0x1_0000 } else { 0 }
}
