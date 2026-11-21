// SPDX-License-Identifier: MIT
// Portable functional firmware for the HeyPCB ESP32-S3 NTC source slice.
// This is compiled to WebAssembly; it is not ESP32 firmware or CPU emulation.
//
// Ten times a second the firmware reads the SNS_AN1 divider in millivolts, as
// Arduino-ESP32 `analogReadMilliVolts()` returns it, converts it to the NTC
// resistance and then to a temperature with the beta equation, prints the
// temperature and drives the IO48 LED as an over-temperature indicator.
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

    /// A temperature in tenths of a degree, as `-10.0` or `25.3`.
    fn tenths(&mut self, value: i32) -> &mut Self {
        if value < 0 {
            self.text("-");
        }
        let magnitude = value.unsigned_abs();
        self.number(magnitude / 10).text(".").number(magnitude % 10)
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

/// Natural logarithm of a positive finite number, without libm:
/// x = m * 2^e with m in [1, 2), ln(m) = 2 * atanh((m - 1) / (m + 1)).
fn ln(x: f64) -> f64 {
    let bits = x.to_bits();
    let exponent = ((bits >> 52) & 0x7ff) as i64 - 1023;
    let mantissa = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    let s = (mantissa - 1.0) / (mantissa + 1.0);
    let s2 = s * s;
    let (mut term, mut sum) = (s, 0.0);
    let mut k = 1.0;
    while k < 60.0 {
        sum += term / k;
        term *= s2;
        k += 2.0;
    }
    exponent as f64 * core::f64::consts::LN_2 + 2.0 * sum
}

/// `loop` before `setup`.
const ERR_NOT_READY: u32 = 0;
/// The divider reads 0 mV or 3300 mV: U4 or R6 is open or shorted.
const ERR_SENSOR_FAULT: u32 = 1;
/// A reading is returned as `(READING + T in tenths of °C) * 10 + LED`.
const READING: i32 = 10_000;

/// Board divider: R6 = 10 kΩ from 3V3 to SNS_AN1, U4 from SNS_AN1 to GND.
const VREF_MV: f64 = 3300.0;
const R6_OHM: f64 = 10_000.0;
/// NCP18XH103F03RB, Murata catalog R44E: R25 = 10 kΩ, B25/50 = 3380 K.
const R25_OHM: f64 = 10_000.0;
const B_KELVIN: f64 = 3380.0;
const T25_KELVIN: f64 = 298.15;
/// Indicator on at 40.0 °C, off again below 38.0 °C.
const ALARM_ON_TENTHS: i32 = 400;
const ALARM_OFF_TENTHS: i32 = 380;
/// The sample is taken this long into each period, after C7 has settled; the
/// rest of the period is the wait to the next one.
const SAMPLE_NS: u32 = 10_000_000;
const PERIOD_NS: u32 = 100_000_000;

struct Monitor {
    sensor_v: u32,
    led: u32,
    ready: bool,
    alarm: bool,
}

static mut MONITOR: Monitor = Monitor {
    sensor_v: 0,
    led: 0,
    ready: false,
    alarm: false,
};

/// Bind the sensor read port (volts on IO1) and the indicator.
#[no_mangle]
pub extern "C" fn setup(sensor_v: u32, led: u32) -> u32 {
    let monitor = unsafe { &mut *core::ptr::addr_of_mut!(MONITOR) };
    monitor.sensor_v = sensor_v;
    monitor.led = led;
    monitor.alarm = false;
    monitor.ready = true;
    unsafe { write(led, 0.0) };
    0
}

/// One period: sample, convert, print and update the indicator.
#[export_name = "loop"]
pub extern "C" fn period() -> u32 {
    let monitor = unsafe { &mut *core::ptr::addr_of_mut!(MONITOR) };
    if !monitor.ready {
        return ERR_NOT_READY;
    }
    unsafe { advance_ns(SAMPLE_NS) };
    // `analogReadMilliVolts()` returns whole millivolts.
    let millivolts = (unsafe { read(monitor.sensor_v) } * 1000.0) as i32;
    if millivolts <= 0 || millivolts as f64 >= VREF_MV {
        let mut line = Line::new();
        line.text("NTC fault mV=").number(millivolts.max(0) as u32);
        line.print();
        unsafe { advance_ns(PERIOD_NS - SAMPLE_NS) };
        return ERR_SENSOR_FAULT;
    }
    let mv = millivolts as f64;
    let r_ntc = R6_OHM * mv / (VREF_MV - mv);
    let kelvin = 1.0 / (1.0 / T25_KELVIN + ln(r_ntc / R25_OHM) / B_KELVIN);
    // `as` truncates toward zero, as C does.
    let tenths = ((kelvin - 273.15) * 10.0) as i32;

    let alarm = if monitor.alarm {
        tenths >= ALARM_OFF_TENTHS
    } else {
        tenths >= ALARM_ON_TENTHS
    };
    if alarm != monitor.alarm {
        monitor.alarm = alarm;
        unsafe { write(monitor.led, if alarm { 1.0 } else { 0.0 }) };
    }

    let mut line = Line::new();
    line.text("T=").tenths(tenths).text(" C mV=").number(millivolts as u32);
    if alarm {
        line.text(" ALARM");
    }
    line.print();

    unsafe { advance_ns(PERIOD_NS - SAMPLE_NS) };
    ((READING + tenths) * 10 + alarm as i32) as u32
}
