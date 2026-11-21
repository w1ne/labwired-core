// SPDX-License-Identifier: MIT
// Portable TMP117 over-temperature monitor: check the device ID over I2C,
// store the alarm limits in T_HIGH_LIMIT and T_LOW_LIMIT and read them back,
// then once per period wait for DATA_READY, read TEMP_RESULT (7.8125 m°C per
// LSB, TMP117 datasheet SNOSCY7 §7.6.1), drive an over-temperature indicator
// with hysteresis between the two limits, and print each reading.
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

    /// A value in thousandths, as `-12.345`.
    fn milli(&mut self, value: i32) -> &mut Self {
        if value < 0 {
            self.text("-");
        }
        let value = value.unsigned_abs();
        self.number(value / 1000).text(".");
        let fraction = value % 1000;
        if fraction < 100 {
            self.text("0");
        }
        if fraction < 10 {
            self.text("0");
        }
        self.number(fraction)
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

const REG_TEMP_RESULT: u8 = 0x00;
const REG_CONFIGURATION: u8 = 0x01;
const REG_T_HIGH_LIMIT: u8 = 0x02;
const REG_T_LOW_LIMIT: u8 = 0x03;
const REG_DEVICE_ID: u8 = 0x0F;
/// DEVICE_ID D11:D0; D15:D12 is the revision.
const DEVICE_NUMBER: u32 = 0x117;
/// CONFIGURATION D13.
const DATA_READY: u32 = 1 << 13;

/// `setup` results other than 0.
const ERR_NO_SENSOR: u32 = 1;
const ERR_WRONG_DEVICE: u32 = 2;
const ERR_LIMIT_READBACK: u32 = 3;

/// Indicator on at 30.00 °C, off again below 29.50 °C, in 1/128 °C.
const HIGH_LIMIT_RAW: i16 = 30 * 128;
const LOW_LIMIT_RAW: i16 = 29 * 128 + 64;
/// Time from the end of one sample to the start of the next: the 1 s
/// conversion cycle of the power-on CONFIGURATION.
const PERIOD_NS: u32 = 1_000_000_000;

struct Monitor {
    bus: u32,
    led: u32,
    address: u32,
    ready: bool,
    alarm: bool,
    high_raw: i16,
    low_raw: i16,
}

static mut MONITOR: Monitor = Monitor {
    bus: 0,
    led: 0,
    address: 0,
    ready: false,
    alarm: false,
    high_raw: 0,
    low_raw: 0,
};

/// Write the pointer, then read the 16-bit register MSB first.
/// `None`: no device answered.
fn read_register(monitor: &Monitor, register: u8) -> Option<u16> {
    let pointer = [register];
    let mut word = [0u8; 2];
    let acked = unsafe {
        i2c_transfer(monitor.bus, monitor.address, pointer.as_ptr(), 1, word.as_mut_ptr(), 2) == 0
    };
    acked.then(|| u16::from_be_bytes(word))
}

fn write_register(monitor: &Monitor, register: u8, value: u16) -> bool {
    let [high, low] = value.to_be_bytes();
    let frame = [register, high, low];
    unsafe {
        i2c_transfer(monitor.bus, monitor.address, frame.as_ptr(), 3, core::ptr::null_mut(), 0) == 0
    }
}

/// 1/128 °C to m°C, rounded half away from zero.
fn raw_to_milli_c(raw: i16) -> i32 {
    let scaled = i32::from(raw) * 125;
    if scaled < 0 {
        (scaled - 8) / 16
    } else {
        (scaled + 8) / 16
    }
}

/// Check the sensor at `address` on `bus` and store the alarm limits in it.
/// 0: ready.
#[no_mangle]
pub extern "C" fn setup(bus: u32, led: u32, address: u32) -> u32 {
    let monitor = unsafe { &mut *core::ptr::addr_of_mut!(MONITOR) };
    monitor.bus = bus;
    monitor.led = led;
    monitor.address = address;
    monitor.ready = false;
    monitor.alarm = false;
    unsafe { write(led, 0.0) };

    let Some(id) = read_register(monitor, REG_DEVICE_ID) else {
        Line::new().text("no TMP117 at ").hex(address, 2).print();
        return ERR_NO_SENSOR;
    };
    if u32::from(id) & 0x0FFF != DEVICE_NUMBER {
        Line::new().text("unexpected device ID ").hex(u32::from(id), 4).print();
        return ERR_WRONG_DEVICE;
    }
    if !write_register(monitor, REG_T_HIGH_LIMIT, HIGH_LIMIT_RAW as u16)
        || !write_register(monitor, REG_T_LOW_LIMIT, LOW_LIMIT_RAW as u16)
    {
        return ERR_NO_SENSOR;
    }
    let Some(high) = read_register(monitor, REG_T_HIGH_LIMIT) else {
        return ERR_NO_SENSOR;
    };
    if high as i16 != HIGH_LIMIT_RAW {
        return ERR_LIMIT_READBACK;
    }
    monitor.high_raw = high as i16;
    monitor.low_raw = LOW_LIMIT_RAW;
    monitor.ready = true;
    Line::new()
        .text("TMP117 ID ")
        .hex(u32::from(id), 4)
        .text(" at ")
        .hex(address, 2)
        .text(" high ")
        .milli(raw_to_milli_c(monitor.high_raw))
        .text(" C")
        .print();
    0
}

/// One period: read a sample and update the indicator. Returns the
/// temperature in m°C plus 100,000. 0: no sample.
#[export_name = "loop"]
pub extern "C" fn period() -> u32 {
    let monitor = unsafe { &mut *core::ptr::addr_of_mut!(MONITOR) };
    if !monitor.ready {
        return 0;
    }
    let ready = read_register(monitor, REG_CONFIGURATION)
        .is_some_and(|config| u32::from(config) & DATA_READY != 0);
    let result = match read_register(monitor, REG_TEMP_RESULT) {
        Some(raw) if ready => {
            let raw = raw as i16;
            let alarm = if monitor.alarm {
                raw >= monitor.low_raw
            } else {
                raw >= monitor.high_raw
            };
            if alarm != monitor.alarm {
                monitor.alarm = alarm;
                unsafe { write(monitor.led, if alarm { 1.0 } else { 0.0 }) };
            }
            let milli_c = raw_to_milli_c(raw);
            Line::new().text("T=").milli(milli_c).text(" C").print();
            (milli_c + 100_000) as u32
        }
        _ => 0,
    };
    unsafe { advance_ns(PERIOD_NS) };
    result
}
