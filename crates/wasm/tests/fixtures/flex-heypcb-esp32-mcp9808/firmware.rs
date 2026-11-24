// SPDX-License-Identifier: MIT
// Portable MCP9808 frost monitor: check the manufacturer and device IDs over
// I2C, store the frost limits in T_LOWER and T_UPPER and read T_LOWER back,
// select the 0.0625 °C resolution and read it back, then once per conversion
// read T_A (13-bit two's complement, 0.0625 °C per LSB, MCP9808 datasheet
// DS20005095B Register 5-4), drive a frost indicator with hysteresis between
// the two limits, and print each reading.
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
    bytes: [u8; 80],
    len: usize,
}

impl Line {
    fn new() -> Self {
        Self { bytes: [0; 80], len: 0 }
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

    /// A T_A count as °C with four decimals, as `-10.1250`. One count is
    /// exactly 625 ten-thousandths of a degree, so no rounding is needed.
    fn celsius(&mut self, count: i32) -> &mut Self {
        let tenths_milli = count * 625;
        if tenths_milli < 0 {
            self.text("-");
        }
        let value = tenths_milli.unsigned_abs();
        self.number(value / 10_000).text(".");
        let fraction = value % 10_000;
        for limit in [1000, 100, 10] {
            if fraction < limit {
                self.text("0");
            }
        }
        self.number(fraction)
    }

    /// `digits` hexadecimal digits, as `0x0054`.
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

/// Register pointers, DS20005095B Table 5-1.
const REG_T_UPPER: u8 = 0x02;
const REG_T_LOWER: u8 = 0x03;
const REG_T_A: u8 = 0x05;
const REG_MANUFACTURER_ID: u8 = 0x06;
const REG_DEVICE_ID: u8 = 0x07;
const REG_RESOLUTION: u8 = 0x08;
const MANUFACTURER_ID: u16 = 0x0054;
/// Device ID in the high byte; the low byte is the revision.
const DEVICE_ID: u16 = 0x04;
/// Resolution 11: 0.0625 °C, 250 ms conversion (DS20005095B Register 5-7).
const RESOLUTION_0_0625: u8 = 0x03;
const CONVERSION_NS: u32 = 250_000_000;

/// `setup` results other than 0.
const ERR_NO_SENSOR: u32 = 1;
const ERR_WRONG_DEVICE: u32 = 2;
const ERR_LIMIT_READBACK: u32 = 3;
const ERR_RESOLUTION_READBACK: u32 = 4;

/// Indicator on below -5.00 °C, off again above -4.50 °C, in T_A counts
/// (0.0625 °C). Both are multiples of 4: the limit registers hold 0.25 °C.
const LOW_LIMIT_COUNT: i32 = -5 * 16;
const HIGH_LIMIT_COUNT: i32 = -4 * 16 - 8;

/// `loop` result offset, so a reading down to -100 °C stays positive.
const RESULT_OFFSET: i32 = 1_000_000;

struct Monitor {
    bus: u32,
    led: u32,
    address: u32,
    ready: bool,
    frost: bool,
    low_count: i32,
    high_count: i32,
}

static mut MONITOR: Monitor = Monitor {
    bus: 0,
    led: 0,
    address: 0,
    ready: false,
    frost: false,
    low_count: 0,
    high_count: 0,
};

/// Write the pointer, then read `N` bytes MSB first. `None`: no device
/// answered.
fn read<const N: usize>(monitor: &Monitor, register: u8) -> Option<[u8; N]> {
    let pointer = [register];
    let mut bytes = [0u8; N];
    let acked = unsafe {
        i2c_transfer(
            monitor.bus,
            monitor.address,
            pointer.as_ptr(),
            1,
            bytes.as_mut_ptr(),
            N as u32,
        ) == 0
    };
    acked.then_some(bytes)
}

fn read_word(monitor: &Monitor, register: u8) -> Option<u16> {
    read::<2>(monitor, register).map(u16::from_be_bytes)
}

fn write_bytes(monitor: &Monitor, frame: &[u8]) -> bool {
    unsafe {
        i2c_transfer(
            monitor.bus,
            monitor.address,
            frame.as_ptr(),
            frame.len() as u32,
            core::ptr::null_mut(),
            0,
        ) == 0
    }
}

/// Bits 12:0 of a temperature register as a signed count of 0.0625 °C.
/// Bits 15:13 of T_A are the comparator flags; they are not temperature.
fn signed_count(word: u16) -> i32 {
    let count = i32::from(word & 0x1FFF);
    if count & 0x1000 != 0 {
        count - 0x2000
    } else {
        count
    }
}

/// A count as a limit-register word: sign in bit 12, bits 1:0 zero.
fn limit_word(count: i32) -> u16 {
    (count & 0x1FFC) as u16
}

/// Check the sensor at `address` on `bus` and store the frost limits in it.
/// 0: ready.
#[no_mangle]
pub extern "C" fn setup(bus: u32, led: u32, address: u32) -> u32 {
    let monitor = unsafe { &mut *core::ptr::addr_of_mut!(MONITOR) };
    monitor.bus = bus;
    monitor.led = led;
    monitor.address = address;
    monitor.ready = false;
    monitor.frost = false;
    unsafe { write(led, 0.0) };

    let Some(manufacturer) = read_word(monitor, REG_MANUFACTURER_ID) else {
        Line::new().text("no MCP9808 at ").hex(address, 2).print();
        return ERR_NO_SENSOR;
    };
    let Some(device) = read_word(monitor, REG_DEVICE_ID) else {
        return ERR_NO_SENSOR;
    };
    if manufacturer != MANUFACTURER_ID || device >> 8 != DEVICE_ID {
        Line::new()
            .text("unexpected IDs ")
            .hex(u32::from(manufacturer), 4)
            .text("/")
            .hex(u32::from(device), 4)
            .print();
        return ERR_WRONG_DEVICE;
    }
    let [low_high, low_low] = limit_word(LOW_LIMIT_COUNT).to_be_bytes();
    let [high_high, high_low] = limit_word(HIGH_LIMIT_COUNT).to_be_bytes();
    if !write_bytes(monitor, &[REG_T_LOWER, low_high, low_low])
        || !write_bytes(monitor, &[REG_T_UPPER, high_high, high_low])
    {
        return ERR_NO_SENSOR;
    }
    let Some(low) = read_word(monitor, REG_T_LOWER) else {
        return ERR_NO_SENSOR;
    };
    if signed_count(low) != LOW_LIMIT_COUNT {
        return ERR_LIMIT_READBACK;
    }
    if !write_bytes(monitor, &[REG_RESOLUTION, RESOLUTION_0_0625]) {
        return ERR_NO_SENSOR;
    }
    let Some([resolution]) = read::<1>(monitor, REG_RESOLUTION) else {
        return ERR_NO_SENSOR;
    };
    if resolution & 0x03 != RESOLUTION_0_0625 {
        Line::new().text("resolution reads ").hex(u32::from(resolution), 2).print();
        return ERR_RESOLUTION_READBACK;
    }
    monitor.low_count = signed_count(low);
    monitor.high_count = HIGH_LIMIT_COUNT;
    monitor.ready = true;
    Line::new()
        .text("MCP9808 ")
        .hex(u32::from(manufacturer), 4)
        .text("/")
        .hex(u32::from(device), 4)
        .text(" at ")
        .hex(address, 2)
        .text(" frost below ")
        .celsius(monitor.low_count)
        .text(" C")
        .print();
    0
}

/// One conversion: read T_A and update the indicator. Returns the temperature
/// in 1/10,000 °C plus 1,000,000. 0: no sample.
#[export_name = "loop"]
pub extern "C" fn period() -> u32 {
    let monitor = unsafe { &mut *core::ptr::addr_of_mut!(MONITOR) };
    if !monitor.ready {
        return 0;
    }
    let result = match read_word(monitor, REG_T_A) {
        Some(word) => {
            let count = signed_count(word);
            let frost = if monitor.frost {
                count <= monitor.high_count
            } else {
                count < monitor.low_count
            };
            if frost != monitor.frost {
                monitor.frost = frost;
                unsafe { write(monitor.led, if frost { 1.0 } else { 0.0 }) };
            }
            Line::new().text("T=").celsius(count).text(" C").print();
            (count * 625 + RESULT_OFFSET) as u32
        }
        None => 0,
    };
    unsafe { advance_ns(CONVERSION_NS) };
    result
}
