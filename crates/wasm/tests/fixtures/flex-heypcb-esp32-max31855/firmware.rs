// SPDX-License-Identifier: MIT
// Portable MAX31855 thermocouple monitor: once per period read the 32-bit
// frame over SPI, decode the thermocouple and cold-junction temperatures as
// the MAX31855 datasheet lays them out, drive an over-temperature indicator
// with hysteresis, and print each reading to the console.
// Nothing here names an MCU: the bus, the indicator and the console are host
// ports.
#![no_std]

#[link(wasm_import_module = "labwired")]
extern "C" {
    fn write(port: u32, value: f64);
    fn advance_ns(delta: u32);
    /// 0: a device is on chip select `cs`. 1: none is; `rx` is not written.
    fn spi_transfer(bus: u32, cs: u32, tx_ptr: *const u8, rx_ptr: *mut u8, len: u32) -> u32;
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

    /// `value / 10^places` with exactly `places` decimals, as `-10.0625`.
    fn fixed(&mut self, value: i32, places: u32) -> &mut Self {
        if value < 0 {
            self.text("-");
        }
        let value = value.unsigned_abs();
        let unit = 10u32.pow(places);
        self.number(value / unit).text(".");
        let fraction = value % unit;
        for place in (1..places).rev() {
            if fraction < 10u32.pow(place) {
                self.text("0");
            }
        }
        self.number(fraction)
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

/// `setup` result other than 0.
const ERR_NO_DEVICE: u32 = 1;

/// Indicator on at 300.00 °C, off again below 280.00 °C.
const ALARM_ON_CENTI_C: i32 = 30_000;
const ALARM_OFF_CENTI_C: i32 = 28_000;
/// Time from the end of one read to the next; the datasheet conversion time
/// is 100 ms at most. Advanced in slices that keep each host call short.
const PERIOD_SLICES: u32 = 10;
const PERIOD_SLICE_NS: u32 = 10_000_000;

struct Monitor {
    bus: u32,
    led: u32,
    cs: u32,
    ready: bool,
    alarm: bool,
}

static mut MONITOR: Monitor = Monitor { bus: 0, led: 0, cs: 0, ready: false, alarm: false };

enum Frame {
    /// Thermocouple in 0.01 °C, cold junction in 0.0001 °C.
    Reading { centi_c: i32, cold_junction: i32 },
    /// Datasheet D16 set; D2..D0 are SCV, SCG and OC.
    Fault(u32),
    NoDevice,
}

/// Read one 32-bit frame. MOSI is not connected on the board; the bytes sent
/// are zero.
fn read_frame(monitor: &Monitor) -> Frame {
    let mut bytes = [0u8; 4];
    let pointer = bytes.as_mut_ptr();
    if unsafe { spi_transfer(monitor.bus, monitor.cs, pointer, pointer, 4) } != 0 {
        return Frame::NoDevice;
    }
    let frame = u32::from_be_bytes(bytes);
    if frame & (1 << 16) != 0 {
        return Frame::Fault(frame & 0b111);
    }
    // D31..D18: 14-bit signed thermocouple temperature, 0.25 °C per LSB.
    // D15..D4: 12-bit signed internal temperature, 0.0625 °C per LSB.
    let thermocouple = (frame as i32) >> 18;
    let internal = ((frame << 16) as i32) >> 20;
    Frame::Reading { centi_c: thermocouple * 25, cold_junction: internal * 625 }
}

/// Select the converter on chip select `cs` of `bus` and read it once.
/// 0: ready.
#[no_mangle]
pub extern "C" fn setup(bus: u32, led: u32, cs: u32) -> u32 {
    let monitor = unsafe { &mut *core::ptr::addr_of_mut!(MONITOR) };
    *monitor = Monitor { bus, led, cs, ready: false, alarm: false };
    unsafe { write(led, 0.0) };
    match read_frame(monitor) {
        Frame::NoDevice => {
            Line::new().text("no MAX31855 on CS ").number(cs).print();
            ERR_NO_DEVICE
        }
        Frame::Fault(code) => {
            monitor.ready = true;
            Line::new().text("MAX31855 fault ").number(code).print();
            0
        }
        Frame::Reading { cold_junction, .. } => {
            monitor.ready = true;
            Line::new()
                .text("MAX31855 on CS ")
                .number(cs)
                .text(" CJ=")
                .fixed(cold_junction, 4)
                .text(" C")
                .print();
            0
        }
    }
}

/// One period: read and update the indicator. Returns the thermocouple
/// temperature in 0.01 °C plus 100,000. 0: no reading.
#[export_name = "loop"]
pub extern "C" fn period() -> u32 {
    let monitor = unsafe { &mut *core::ptr::addr_of_mut!(MONITOR) };
    if !monitor.ready {
        return 0;
    }
    let result = match read_frame(monitor) {
        Frame::Reading { centi_c, cold_junction } => {
            let alarm = if monitor.alarm {
                centi_c >= ALARM_OFF_CENTI_C
            } else {
                centi_c >= ALARM_ON_CENTI_C
            };
            if alarm != monitor.alarm {
                monitor.alarm = alarm;
                unsafe { write(monitor.led, if alarm { 1.0 } else { 0.0 }) };
            }
            Line::new()
                .text("TC=")
                .fixed(centi_c, 2)
                .text(" C CJ=")
                .fixed(cold_junction, 4)
                .text(" C")
                .print();
            (centi_c + 100_000) as u32
        }
        Frame::Fault(code) => {
            Line::new().text("MAX31855 fault ").number(code).print();
            0
        }
        Frame::NoDevice => 0,
    };
    for _ in 0..PERIOD_SLICES {
        unsafe { advance_ns(PERIOD_SLICE_NS) };
    }
    result
}
