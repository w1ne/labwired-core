// SPDX-License-Identifier: MIT
// Portable SCD41 CO2 monitor: stop any running measurement, read the serial
// number, and start periodic measurement over I2C with the 16-bit commands of
// the Sensirion SCD4x datasheet. Once per measurement interval it polls the
// data-ready status, reads the three CRC-framed words, converts them, drives a
// high-CO2 indicator with hysteresis, and prints each reading to the console.
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
        self.text("0x").digits(value, digits)
    }

    /// `digits` more hexadecimal digits, with no prefix.
    fn digits(&mut self, value: u32, digits: u32) -> &mut Self {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
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

const CMD_STOP_PERIODIC: u16 = 0x3F86;
const CMD_GET_SERIAL: u16 = 0x3682;
const CMD_START_PERIODIC: u16 = 0x21B1;
const CMD_DATA_READY: u16 = 0xE4B8;
const CMD_READ_MEASUREMENT: u16 = 0xEC05;

/// `setup` results other than 0.
const ERR_NO_SENSOR: u32 = 1;
const ERR_CRC: u32 = 2;
/// `loop` results that are not a sample.
const LOOP_NOT_READY: u32 = 1;
const LOOP_NO_SAMPLE: u32 = 0;

/// Indicator on at 1000 ppm, off again below 800 ppm.
const ALARM_ON_PPM: u32 = 1_000;
const ALARM_OFF_PPM: u32 = 800;
/// Datasheet maximum execution time of `stop_periodic_measurement`.
const STOP_WAIT_NS: u32 = 500_000_000;
/// Datasheet maximum execution time of the read commands.
const READ_WAIT_NS: u32 = 1_000_000;

struct Monitor {
    bus: u32,
    led: u32,
    address: u32,
    interval_ms: u32,
    ready: bool,
    alarm: bool,
    centi_c: i32,
    centi_rh: i32,
}

static mut MONITOR: Monitor = Monitor {
    bus: 0,
    led: 0,
    address: 0,
    interval_ms: 0,
    ready: false,
    alarm: false,
    centi_c: 0,
    centi_rh: 0,
};

/// `advance_ns` takes a `u32`, which holds 4.29 s at most. A wait of
/// several seconds is given in whole seconds and a remainder.
fn wait_ms(ms: u32) {
    for _ in 0..ms / 1_000 {
        unsafe { advance_ns(1_000_000_000) };
    }
    unsafe { advance_ns(ms % 1_000 * 1_000_000) };
}

fn command(monitor: &Monitor, code: u16) -> bool {
    let frame = code.to_be_bytes();
    unsafe {
        i2c_transfer(monitor.bus, monitor.address, frame.as_ptr(), 2, core::ptr::null_mut(), 0) == 0
    }
}

/// Send `code`, wait, and read `N` words, each followed by its CRC.
/// `None`: no answer or a CRC error.
fn read_words<const N: usize>(monitor: &Monitor, code: u16) -> Option<[u32; N]> {
    if !command(monitor, code) {
        return None;
    }
    unsafe { advance_ns(READ_WAIT_NS) };
    let mut frame = [0u8; 9];
    let acked = unsafe {
        i2c_transfer(monitor.bus, monitor.address, core::ptr::null(), 0, frame.as_mut_ptr(), 3 * N as u32) == 0
    };
    if !acked {
        return None;
    }
    let mut words = [0u32; N];
    for (index, word) in words.iter_mut().enumerate() {
        let bytes = &frame[3 * index..3 * index + 3];
        if crc8(&bytes[0..2]) != bytes[2] {
            return None;
        }
        *word = u32::from(u16::from_be_bytes([bytes[0], bytes[1]]));
    }
    Some(words)
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

/// Stop any periodic measurement of the sensor at `address` on `bus`, read
/// its serial number and start periodic measurement. `interval_ms` is the wait
/// before each data-ready poll. 0: ready.
#[no_mangle]
pub extern "C" fn setup(bus: u32, led: u32, address: u32, interval_ms: u32) -> u32 {
    let monitor = unsafe { &mut *core::ptr::addr_of_mut!(MONITOR) };
    monitor.bus = bus;
    monitor.led = led;
    monitor.address = address;
    monitor.interval_ms = interval_ms;
    monitor.ready = false;
    monitor.alarm = false;
    unsafe { write(led, 0.0) };

    if !command(monitor, CMD_STOP_PERIODIC) {
        Line::new().text("no SCD41 at ").hex(address, 2).print();
        return ERR_NO_SENSOR;
    }
    unsafe { advance_ns(STOP_WAIT_NS) };
    let Some([high, middle, low]) = read_words::<3>(monitor, CMD_GET_SERIAL) else {
        Line::new().text("SCD41 serial CRC error").print();
        return ERR_CRC;
    };
    if !command(monitor, CMD_START_PERIODIC) {
        return ERR_NO_SENSOR;
    }
    monitor.ready = true;
    Line::new()
        .text("SCD41 serial ")
        .hex(high, 4)
        .digits(middle, 4)
        .digits(low, 4)
        .text(" at ")
        .hex(address, 2)
        .print();
    0
}

/// One measurement interval: wait, poll data-ready, read and update the
/// indicator. Returns CO2 in ppm times 10 plus the indicator state.
/// 1: data not ready. 0: no sample.
#[export_name = "loop"]
pub extern "C" fn period() -> u32 {
    let monitor = unsafe { &mut *core::ptr::addr_of_mut!(MONITOR) };
    if !monitor.ready {
        return LOOP_NO_SAMPLE;
    }
    wait_ms(monitor.interval_ms);
    let Some([status]) = read_words::<1>(monitor, CMD_DATA_READY) else {
        Line::new().text("SCD41 status error").print();
        return LOOP_NO_SAMPLE;
    };
    // Datasheet: data is not ready when the least significant 11 bits are 0.
    if status & 0x07FF == 0 {
        Line::new().text("SCD41 not ready").print();
        return LOOP_NOT_READY;
    }
    let Some([co2, raw_t, raw_rh]) = read_words::<3>(monitor, CMD_READ_MEASUREMENT) else {
        Line::new().text("SCD41 CRC error").print();
        return LOOP_NO_SAMPLE;
    };
    // T = -45 + 175 * word / 65535 and RH = 100 * word / 65535, in
    // hundredths, rounded to nearest.
    monitor.centi_c = -4_500 + ((17_500 * raw_t + 32_767) / 65_535) as i32;
    monitor.centi_rh = ((10_000 * raw_rh + 32_767) / 65_535) as i32;
    let alarm = if monitor.alarm { co2 >= ALARM_OFF_PPM } else { co2 >= ALARM_ON_PPM };
    if alarm != monitor.alarm {
        monitor.alarm = alarm;
        unsafe { write(monitor.led, if alarm { 1.0 } else { 0.0 }) };
    }
    let mut line = Line::new();
    line.text("CO2=")
        .number(co2)
        .text(" ppm T=")
        .centi(monitor.centi_c)
        .text(" C RH=")
        .centi(monitor.centi_rh)
        .text(" %");
    if alarm {
        line.text(" HIGH");
    }
    line.print();
    co2 * 10 + u32::from(alarm)
}

/// Temperature and humidity of the last good sample: 0.01 °C plus 10,000,
/// times 100,000, plus 0.01 %RH.
#[no_mangle]
pub extern "C" fn climate() -> u32 {
    let monitor = unsafe { &*core::ptr::addr_of!(MONITOR) };
    ((monitor.centi_c + 10_000) * 100_000 + monitor.centi_rh) as u32
}
