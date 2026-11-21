// SPDX-License-Identifier: MIT
// Portable functional firmware for the HeyPCB ESP32-S3 MCP9700 source slice.
// This is compiled to WebAssembly; it is not ESP32 firmware or CPU emulation.
//
// Once a second the firmware reads the sensor output in millivolts, as
// Arduino-ESP32 `analogReadMilliVolts()` returns it, converts it with the
// MCP9700 transfer function (500 mV at 0 °C, 10.0 mV/°C), prints the
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

    /// A temperature in tenths of a degree, as `-40.0` or `25.3`.
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

/// `loop` before `setup`.
const ERR_NOT_READY: u32 = 0;
/// A reading is returned as `(READING + T in tenths of °C) * 10 + LED`.
const READING: i32 = 10_000;

/// MCP9700 data sheet DS20001942L, Equation 4-1: V_0°C and T_C.
const V0_MV: i32 = 500;
const TC_MV_PER_C: i32 = 10;
/// Indicator on at 40.0 °C (900 mV), off again below 38.0 °C (880 mV).
const ALARM_ON_TENTHS: i32 = 400;
const ALARM_OFF_TENTHS: i32 = 380;
/// The sample is taken this long into each period; the rest of the period is
/// the wait to the next one.
const SAMPLE_NS: u32 = 1_000_000;
const PERIOD_NS: u32 = 1_000_000_000;

struct Monitor {
    sensor_mv: u32,
    led: u32,
    ready: bool,
    alarm: bool,
}

static mut MONITOR: Monitor = Monitor {
    sensor_mv: 0,
    led: 0,
    ready: false,
    alarm: false,
};

/// Bind the sensor read port (millivolts on IO1) and the indicator.
#[no_mangle]
pub extern "C" fn setup(sensor_mv: u32, led: u32) -> u32 {
    let monitor = unsafe { &mut *core::ptr::addr_of_mut!(MONITOR) };
    monitor.sensor_mv = sensor_mv;
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
    let millivolts = unsafe { read(monitor.sensor_mv) } as i32;
    let tenths = (millivolts - V0_MV) * 10 / TC_MV_PER_C;

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
    line.text("T=").tenths(tenths).text(" C");
    if alarm {
        line.text(" ALARM");
    }
    line.print();

    unsafe { advance_ns(PERIOD_NS - SAMPLE_NS) };
    ((READING + tenths) * 10 + alarm as i32) as u32
}
