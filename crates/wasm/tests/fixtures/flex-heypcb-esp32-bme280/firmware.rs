// SPDX-License-Identifier: MIT
// Portable BME280 station: probe the sensor over I2C, read its calibration,
// then once per period read one sample, compensate it with the fixed-point
// reference arithmetic of the Bosch BME280 datasheet (section 4.2.3 and
// appendix 8.2), and drive an over-temperature indicator with hysteresis.
// Nothing here names an MCU: the bus and the indicator are host ports.
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
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    core::arch::wasm32::unreachable()
}

const REG_CALIB_TP: u8 = 0x88;
const REG_CALIB_H1: u8 = 0xA1;
const REG_ID: u8 = 0xD0;
const REG_CALIB_H2: u8 = 0xE1;
const REG_CTRL_HUM: u8 = 0xF2;
const REG_CTRL_MEAS: u8 = 0xF4;
const REG_CONFIG: u8 = 0xF5;
const REG_DATA: u8 = 0xF7;
const CHIP_ID_BME280: u8 = 0x60;

/// `setup` results other than 0.
const ERR_NO_SENSOR: u32 = 1;
const ERR_WRONG_CHIP: u32 = 2;

/// Indicator on at 30.00 °C, off again below 28.00 °C.
const ALARM_ON_CENTI_C: i32 = 3_000;
const ALARM_OFF_CENTI_C: i32 = 2_800;
/// Time from the end of one sample to the start of the next.
const PERIOD_NS: u32 = 1_000_000;

struct Station {
    bus: u32,
    led: u32,
    address: u32,
    ready: bool,
    alarm: bool,
    t: [i32; 3],
    p: [i64; 9],
    h: [i32; 6],
    pressure_pa: u32,
    humidity_centi_rh: u32,
}

static mut STATION: Station = Station {
    bus: 0,
    led: 0,
    address: 0,
    ready: false,
    alarm: false,
    t: [0; 3],
    p: [0; 9],
    h: [0; 6],
    pressure_pa: 0,
    humidity_centi_rh: 0,
};

fn read_registers(station: &Station, register: u8, into: &mut [u8]) -> bool {
    let pointer = [register];
    unsafe {
        i2c_transfer(
            station.bus,
            station.address,
            pointer.as_ptr(),
            1,
            into.as_mut_ptr(),
            into.len() as u32,
        ) == 0
    }
}

fn write_register(station: &Station, register: u8, value: u8) -> bool {
    let frame = [register, value];
    unsafe {
        i2c_transfer(
            station.bus,
            station.address,
            frame.as_ptr(),
            2,
            core::ptr::null_mut(),
            0,
        ) == 0
    }
}

fn u16_le(bytes: &[u8], at: usize) -> i32 {
    i32::from(u16::from_le_bytes([bytes[at], bytes[at + 1]]))
}

fn i16_le(bytes: &[u8], at: usize) -> i32 {
    i32::from(i16::from_le_bytes([bytes[at], bytes[at + 1]]))
}

/// Probe the sensor at `address` on `bus`, load its calibration and start
/// normal mode with 1x oversampling. 0: ready.
#[no_mangle]
pub extern "C" fn setup(bus: u32, led: u32, address: u32) -> u32 {
    let station = unsafe { &mut *core::ptr::addr_of_mut!(STATION) };
    station.bus = bus;
    station.led = led;
    station.address = address;
    station.ready = false;
    station.alarm = false;
    unsafe { write(led, 0.0) };

    let mut id = [0u8; 1];
    if !read_registers(station, REG_ID, &mut id) {
        return ERR_NO_SENSOR;
    }
    if id[0] != CHIP_ID_BME280 {
        return ERR_WRONG_CHIP;
    }
    let mut tp = [0u8; 24];
    let mut h1 = [0u8; 1];
    let mut h2 = [0u8; 7];
    if !read_registers(station, REG_CALIB_TP, &mut tp)
        || !read_registers(station, REG_CALIB_H1, &mut h1)
        || !read_registers(station, REG_CALIB_H2, &mut h2)
    {
        return ERR_NO_SENSOR;
    }
    station.t = [u16_le(&tp, 0), i16_le(&tp, 2), i16_le(&tp, 4)];
    station.p[0] = i64::from(u16_le(&tp, 6));
    for index in 1..9 {
        station.p[index] = i64::from(i16_le(&tp, 6 + 2 * index));
    }
    station.h = [
        i32::from(h1[0]),
        i16_le(&h2, 0),
        i32::from(h2[2]),
        (i32::from(h2[3] as i8) << 4) | i32::from(h2[4] & 0x0f),
        (i32::from(h2[5] as i8) << 4) | i32::from(h2[4] >> 4),
        i32::from(h2[6] as i8),
    ];
    // ctrl_hum must be written before ctrl_meas to take effect.
    if !write_register(station, REG_CTRL_HUM, 0x01)
        || !write_register(station, REG_CTRL_MEAS, 0x27)
        || !write_register(station, REG_CONFIG, 0x00)
    {
        return ERR_NO_SENSOR;
    }
    station.ready = true;
    0
}

/// One period: read a sample and update the indicator. Returns the
/// temperature in 0.01 °C plus 10,000, so that the sensor's whole
/// -40..85 °C range is a positive number. 0: no sample.
#[export_name = "loop"]
pub extern "C" fn period() -> u32 {
    let station = unsafe { &mut *core::ptr::addr_of_mut!(STATION) };
    let mut data = [0u8; 8];
    if !station.ready || !read_registers(station, REG_DATA, &mut data) {
        return 0;
    }
    let adc_p = (i32::from(data[0]) << 12) | (i32::from(data[1]) << 4) | i32::from(data[2] >> 4);
    let adc_t = (i32::from(data[3]) << 12) | (i32::from(data[4]) << 4) | i32::from(data[5] >> 4);
    let adc_h = (i32::from(data[6]) << 8) | i32::from(data[7]);

    let [t1, t2, t3] = station.t;
    let var1 = (((adc_t >> 3) - (t1 << 1)) * t2) >> 11;
    let var2 = (((((adc_t >> 4) - t1) * ((adc_t >> 4) - t1)) >> 12) * t3) >> 14;
    let t_fine = var1 + var2;
    let centi_c = (t_fine * 5 + 128) >> 8;

    let p = &station.p;
    let mut v1 = i64::from(t_fine) - 128_000;
    let mut v2 = v1 * v1 * p[5];
    v2 += (v1 * p[4]) << 17;
    v2 += p[3] << 35;
    v1 = ((v1 * v1 * p[2]) >> 8) + ((v1 * p[1]) << 12);
    v1 = (((1i64 << 47) + v1) * p[0]) >> 33;
    station.pressure_pa = if v1 == 0 {
        0
    } else {
        let mut pressure = 1_048_576 - i64::from(adc_p);
        pressure = (((pressure << 31) - v2) * 3125) / v1;
        v1 = (p[8] * (pressure >> 13) * (pressure >> 13)) >> 25;
        v2 = (p[7] * pressure) >> 19;
        pressure = ((pressure + v1 + v2) >> 8) + (p[6] << 4);
        (pressure >> 8) as u32
    };

    let h = &station.h;
    let mut x = t_fine - 76_800;
    x = ((((adc_h << 14) - (h[3] << 20) - (h[4] * x)) + 16_384) >> 15)
        * (((((((x * h[5]) >> 10) * (((x * h[2]) >> 11) + 32_768)) >> 10) + 2_097_152) * h[1]
            + 8_192)
            >> 14);
    x -= ((((x >> 15) * (x >> 15)) >> 7) * h[0]) >> 4;
    x = x.clamp(0, 419_430_400);
    station.humidity_centi_rh = (((x >> 12) as u32) * 100) >> 10;

    let alarm = if station.alarm {
        centi_c >= ALARM_OFF_CENTI_C
    } else {
        centi_c >= ALARM_ON_CENTI_C
    };
    if alarm != station.alarm {
        station.alarm = alarm;
        unsafe { write(station.led, if alarm { 1.0 } else { 0.0 }) };
    }
    unsafe { advance_ns(PERIOD_NS) };
    (centi_c + 10_000) as u32
}

/// Pressure of the last sample, in Pa.
#[no_mangle]
pub extern "C" fn pressure_pa() -> u32 {
    unsafe { (*core::ptr::addr_of!(STATION)).pressure_pa }
}

/// Relative humidity of the last sample, in 0.01 %RH.
#[no_mangle]
pub extern "C" fn humidity_centi_rh() -> u32 {
    unsafe { (*core::ptr::addr_of!(STATION)).humidity_centi_rh }
}
