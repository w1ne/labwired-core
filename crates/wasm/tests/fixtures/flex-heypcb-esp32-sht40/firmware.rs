// SPDX-License-Identifier: MIT
// Portable SHT40 humidity monitor: reset the sensor and read its serial number
// over I2C, then once per period start a high-precision measurement, wait,
// read the two CRC-framed words, convert them with the formulas of the
// Sensirion SHT4x datasheet, drive a high-humidity indicator with hysteresis,
// and print each reading to the console.
// Nothing here names an MCU: the bus, the indicator and the console are host
// ports.
#![no_std]

#[link(wasm_import_module = "labwired")]
extern "C" {
    fn write(port: u32, value: f64);
    fn advance_ns(delta: u32);
    /// 0: a device acknowledged. 1: no device answered the address.
    fn i2c_transfer(
        bus: u32,
        address: u32,
        write_ptr: *const u8,
        write_len: u32,
        read_ptr: *mut u8,
        read_len: u32,
    ) -> u32;
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

    /// A value in hundredths, as `-12.34`.
    fn centi(&mut self, value: i32) -> &mut Self {
        if value < 0 {
            self.text("-");
        }
        let value = value.unsigned_abs();
        self.number(value / 100).text(".");
        if value % 100 < 10 {
            self.text("0");
        }
        self.number(value % 100)
    }

    /// `digits` hexadecimal digits, as `0x0f4e`.
    fn hex(&mut self, value: u32, digits: u32) -> &mut Self {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        self.text("0x");
        for index in (0..digits).rev() {
            self.bytes[self.len] = DIGITS[((value >> (4 * index)) & 0xf) as usize];
            self.len += 1;
        }
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

const CMD_MEASURE_HIGH_PRECISION: u8 = 0xFD;
const CMD_READ_SERIAL: u8 = 0x89;
const CMD_SOFT_RESET: u8 = 0x94;

/// `setup` results other than 0.
const ERR_NO_SENSOR: u32 = 1;
const ERR_CRC: u32 = 2;

/// Indicator on at 70.00 %RH, off again below 65.00 %RH.
const ALARM_ON_CENTI_RH: i32 = 7_000;
const ALARM_OFF_CENTI_RH: i32 = 6_500;
/// Wait after a soft reset and after the serial-number command.
const COMMAND_WAIT_NS: u32 = 1_000_000;
/// Time from the end of one sample to the start of the next.
const PERIOD_NS: u32 = 1_000_000;

struct Monitor {
    bus: u32,
    led: u32,
    address: u32,
    measure_wait_ns: u32,
    ready: bool,
    alarm: bool,
    centi_c: i32,
}

static mut MONITOR: Monitor = Monitor {
    bus: 0,
    led: 0,
    address: 0,
    measure_wait_ns: 0,
    ready: false,
    alarm: false,
    centi_c: 0,
};

fn command(monitor: &Monitor, code: u8) -> bool {
    let frame = [code];
    unsafe {
        i2c_transfer(monitor.bus, monitor.address, frame.as_ptr(), 1, core::ptr::null_mut(), 0) == 0
    }
}

/// Read two words, each followed by its CRC. `None`: no answer or a CRC error.
fn read_words(monitor: &Monitor) -> Option<[u32; 2]> {
    let mut frame = [0u8; 6];
    let acked = unsafe {
        i2c_transfer(monitor.bus, monitor.address, core::ptr::null(), 0, frame.as_mut_ptr(), 6) == 0
    };
    if !acked || crc8(&frame[0..2]) != frame[2] || crc8(&frame[3..5]) != frame[5] {
        return None;
    }
    Some([
        u32::from(u16::from_be_bytes([frame[0], frame[1]])),
        u32::from(u16::from_be_bytes([frame[3], frame[4]])),
    ])
}

/// Sensirion CRC-8: polynomial 0x31, initial value 0xFF, no final XOR.
fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0xFFu8;
    for &byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x31 } else { crc << 1 };
        }
    }
    crc
}

/// Reset the sensor at `address` on `bus` and read its serial number.
/// `measure_wait_us` is the wait between a measurement command and the read.
/// 0: ready.
#[no_mangle]
pub extern "C" fn setup(bus: u32, led: u32, address: u32, measure_wait_us: u32) -> u32 {
    let monitor = unsafe { &mut *core::ptr::addr_of_mut!(MONITOR) };
    monitor.bus = bus;
    monitor.led = led;
    monitor.address = address;
    monitor.measure_wait_ns = measure_wait_us * 1_000;
    monitor.ready = false;
    monitor.alarm = false;
    unsafe { write(led, 0.0) };

    if !command(monitor, CMD_SOFT_RESET) {
        Line::new().text("no SHT40 at ").hex(address, 2).print();
        return ERR_NO_SENSOR;
    }
    unsafe { advance_ns(COMMAND_WAIT_NS) };
    if !command(monitor, CMD_READ_SERIAL) {
        return ERR_NO_SENSOR;
    }
    unsafe { advance_ns(COMMAND_WAIT_NS) };
    let Some([high, low]) = read_words(monitor) else {
        Line::new().text("SHT40 serial CRC error").print();
        return ERR_CRC;
    };
    monitor.ready = true;
    Line::new()
        .text("SHT40 serial ")
        .hex((high << 16) | low, 8)
        .text(" at ")
        .hex(address, 2)
        .print();
    0
}

/// One period: measure and update the indicator. Returns the relative
/// humidity in 0.01 %RH plus 10,000. 0: no sample.
#[export_name = "loop"]
pub extern "C" fn period() -> u32 {
    let monitor = unsafe { &mut *core::ptr::addr_of_mut!(MONITOR) };
    if !monitor.ready || !command(monitor, CMD_MEASURE_HIGH_PRECISION) {
        return 0;
    }
    unsafe { advance_ns(monitor.measure_wait_ns) };
    let sample = read_words(monitor);
    let result = match sample {
        Some([raw_t, raw_rh]) => {
            // T = -45 + 175 * S_T / 65535 and RH = -6 + 125 * S_RH / 65535,
            // in hundredths, rounded to nearest. RH is cropped to 0..100 %.
            let centi_c = -4_500 + ((17_500 * raw_t + 32_767) / 65_535) as i32;
            let centi_rh = (-600 + ((12_500 * raw_rh + 32_767) / 65_535) as i32).clamp(0, 10_000);
            monitor.centi_c = centi_c;
            let alarm = if monitor.alarm {
                centi_rh >= ALARM_OFF_CENTI_RH
            } else {
                centi_rh >= ALARM_ON_CENTI_RH
            };
            if alarm != monitor.alarm {
                monitor.alarm = alarm;
                unsafe { write(monitor.led, if alarm { 1.0 } else { 0.0 }) };
            }
            Line::new()
                .text("T=")
                .centi(centi_c)
                .text(" C RH=")
                .centi(centi_rh)
                .text(" %")
                .print();
            (centi_rh + 10_000) as u32
        }
        None => {
            Line::new().text("SHT40 CRC error").print();
            0
        }
    };
    unsafe { advance_ns(PERIOD_NS) };
    result
}

/// Temperature of the last good sample, in 0.01 °C plus 10,000.
#[no_mangle]
pub extern "C" fn temperature_centi_c() -> u32 {
    unsafe { ((*core::ptr::addr_of!(MONITOR)).centi_c + 10_000) as u32 }
}
