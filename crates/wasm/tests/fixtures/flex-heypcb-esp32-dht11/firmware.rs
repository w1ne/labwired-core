// SPDX-License-Identifier: MIT
// Portable DHT11 humidity monitor: send the single-wire start signal, time the
// sensor's answer one microsecond at a time, decode the 40-bit frame (Aosong
// DHT11 datasheet: 18 ms start, 80 µs low + 80 µs high response, each bit
// 50 µs low then 26-28 µs high for 0 or 70 µs high for 1), check the
// checksum, drive a humidity indicator with hysteresis and print each reading.
// Nothing here names an MCU: the data line, the indicator and the console are
// host ports.
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

    fn print(&mut self) {
        self.text("\n");
        unsafe { console_write(self.bytes.as_ptr(), self.len as u32) };
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}

/// `loop` results below `READING`.
const ERR_NOT_READY: u32 = 0;
const ERR_NO_RESPONSE: u32 = 1;
const ERR_TIMEOUT: u32 = 2;
const ERR_CHECKSUM: u32 = 3;
/// A reading is returned as `READING + RH * 1000 + T`.
const READING: u32 = 1_000_000;

/// Poll period of the data line.
const POLL_NS: u32 = 1_000;
/// Longest wait for one level of the answer, in polls.
const LEVEL_TIMEOUT_POLLS: u32 = 100;
/// A high bit time longer than this is a 1 (26-28 µs is 0, 70 µs is 1).
const ONE_THRESHOLD_POLLS: u32 = 48;
/// Time from the end of one reading to the next start signal; the datasheet
/// asks for at least 1 s between readings.
const PERIOD_NS: u32 = 1_000_000_000;
/// Indicator on at 70 %RH, off again below 65 %RH.
const ALARM_ON_RH: u8 = 70;
const ALARM_OFF_RH: u8 = 65;

struct Monitor {
    drive: u32,
    line: u32,
    led: u32,
    start_ns: u32,
    ready: bool,
    alarm: bool,
}

static mut MONITOR: Monitor = Monitor {
    drive: 0,
    line: 0,
    led: 0,
    start_ns: 0,
    ready: false,
    alarm: false,
};

fn line_high(monitor: &Monitor) -> bool {
    unsafe { read(monitor.line) != 0.0 }
}

/// Poll until the line reads `level`. Returns the polls it took, or `None`
/// after `LEVEL_TIMEOUT_POLLS`.
fn wait_for(monitor: &Monitor, level: bool) -> Option<u32> {
    for polls in 1..=LEVEL_TIMEOUT_POLLS {
        unsafe { advance_ns(POLL_NS) };
        if line_high(monitor) == level {
            return Some(polls);
        }
    }
    None
}

/// Send the start signal and read the 5-byte frame.
fn read_frame(monitor: &Monitor) -> Result<[u8; 5], u32> {
    unsafe {
        write(monitor.drive, 0.0);
        advance_ns(monitor.start_ns);
        write(monitor.drive, 1.0);
    }
    // Response: the sensor pulls the line low, releases it, pulls it low again
    // for the first bit.
    wait_for(monitor, false).ok_or(ERR_NO_RESPONSE)?;
    wait_for(monitor, true).ok_or(ERR_TIMEOUT)?;
    wait_for(monitor, false).ok_or(ERR_TIMEOUT)?;
    let mut frame = [0u8; 5];
    for bit in 0..40 {
        wait_for(monitor, true).ok_or(ERR_TIMEOUT)?;
        let high_polls = wait_for(monitor, false).ok_or(ERR_TIMEOUT)?;
        if high_polls > ONE_THRESHOLD_POLLS {
            frame[bit / 8] |= 0x80 >> (bit % 8);
        }
    }
    Ok(frame)
}

/// Bind the data line write port (`drive`, 0 pulls low, 1 releases), its read
/// port (`line`), the indicator, and the start-signal length in µs.
#[no_mangle]
pub extern "C" fn setup(drive: u32, line: u32, led: u32, start_us: u32) -> u32 {
    let monitor = unsafe { &mut *core::ptr::addr_of_mut!(MONITOR) };
    monitor.drive = drive;
    monitor.line = line;
    monitor.led = led;
    monitor.start_ns = start_us * 1_000;
    monitor.alarm = false;
    monitor.ready = true;
    unsafe {
        write(drive, 1.0);
        write(led, 0.0);
    }
    0
}

/// One period: read the sensor and update the indicator.
#[export_name = "loop"]
pub extern "C" fn period() -> u32 {
    let monitor = unsafe { &mut *core::ptr::addr_of_mut!(MONITOR) };
    if !monitor.ready {
        return ERR_NOT_READY;
    }
    let result = match read_frame(monitor) {
        Err(ERR_NO_RESPONSE) => {
            Line::new().text("no DHT11 response").print();
            ERR_NO_RESPONSE
        }
        Err(error) => {
            Line::new().text("DHT11 timeout").print();
            error
        }
        Ok(frame) if frame[0].wrapping_add(frame[1]).wrapping_add(frame[2]).wrapping_add(frame[3]) != frame[4] => {
            Line::new().text("DHT11 checksum error").print();
            ERR_CHECKSUM
        }
        Ok(frame) => {
            let (rh, t) = (frame[0], frame[2]);
            let alarm = if monitor.alarm {
                rh >= ALARM_OFF_RH
            } else {
                rh >= ALARM_ON_RH
            };
            if alarm != monitor.alarm {
                monitor.alarm = alarm;
                unsafe { write(monitor.led, if alarm { 1.0 } else { 0.0 }) };
            }
            Line::new()
                .text("T=")
                .number(u32::from(t))
                .text(" C RH=")
                .number(u32::from(rh))
                .text(" %")
                .print();
            READING + u32::from(rh) * 1_000 + u32::from(t)
        }
    };
    unsafe { advance_ns(PERIOD_NS) };
    result
}
