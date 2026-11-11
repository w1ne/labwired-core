//! LabWired - STM32F103 I²C silicon-validation trace.
//!
//! Probes an absent I²C address, reads the TMP102 temperature, then writes and
//! reads back its configuration and alert threshold registers through I2C1,
//! including a one-byte MSB update. It also reads the BME280 chip ID and a
//! little-endian calibration word, reads the complete BME280 calibration map,
//! then compensates pressure, temperature and humidity ADC registers using
//! the Bosch fixed-point formulas, including scripted live changes to all
//! three sensor inputs. It also checks I2C write/read-back for the BME280's
//! control-register storage. USART2 emits a human-readable register
//! fingerprint at 115200/8N1. The onboarding system attaches declarative TMP102
//! and BME280 models at addresses 0x48 and 0x76, plus a MAX31855 on SPI1;
//! TMP102's reset temperature is 25 °C (0x1900). The absent I2C address remains
//! an intentional NACK check.
//! These exact sensor values are simulator assertions, not a claim about a
//! physical board's ambient reading.
//!
//! # STM32F103 register map (subset used here)
//!
//! RCC  @ 0x40021000 — APB2ENR.IOPAEN bit 2, AFIOEN bit 0;
//!                    APB1ENR.USART2EN bit 17, I2C1EN bit 21.
//! GPIOA@ 0x40010800 — CRL: PA2 (USART2_TX) = alternate-function push-pull
//!                    50 MHz output = nibble 0b1011 in bits[11:8].
//! USART2@0x40004400 — SR/DR/BRR/CR1; BRR=0x45 → 115200 @ 8 MHz HSI.
//! SPI1  @0x40013000 — CR1/CR2/SR/DR, 8-bit master mode.
//! I2C1 @ 0x40005400 — CR1/CR2/OAR1/DR/SR1/SR2/CCR/TRISE.

#![no_std]
#![no_main]
#![allow(clippy::empty_loop, clippy::identity_op, clippy::needless_range_loop)]

use core::ptr::{read_volatile, write_volatile};

// ── Register addresses ────────────────────────────────────────────────────────

const RCC_BASE: u32 = 0x4002_1000;
const RCC_APB2ENR: u32 = RCC_BASE + 0x18;
const RCC_APB1ENR: u32 = RCC_BASE + 0x1C;

const GPIOA_BASE: u32 = 0x4001_0800;
const GPIOA_CRL: u32 = GPIOA_BASE + 0x00;

const GPIOB_BASE: u32 = 0x4001_0C00;
const GPIOB_CRL: u32 = GPIOB_BASE + 0x00; /* PB6 (SCL) and PB7 (SDA) live here */

const USART2_BASE: u32 = 0x4000_4400;
const USART2_SR: u32 = USART2_BASE + 0x00;
const USART2_DR: u32 = USART2_BASE + 0x04;
const USART2_BRR: u32 = USART2_BASE + 0x08;
const USART2_CR1: u32 = USART2_BASE + 0x0C;
const USART_SR_RXNE: u32 = 1 << 5;
const NVIC_ISPR: *const u32 = 0xE000_E200 as *const u32;
const NVIC_ICPR: *mut u32 = 0xE000_E280 as *mut u32;
const USART2_IRQ: u32 = 38;
const USART2_IRQ_BIT: u32 = 1 << (USART2_IRQ % 32);

const SYSTICK_CSR: u32 = 0xE000_E010;
const SYSTICK_RVR: u32 = 0xE000_E014;
const SYSTICK_CVR: u32 = 0xE000_E018;
const SYSTICK_CALIB: u32 = 0xE000_E01C;
const SYSTICK_COUNTFLAG: u32 = 1 << 16;
const SCB_ICSR: u32 = 0xE000_ED04;
const SCB_VTOR: u32 = 0xE000_ED08;
const ICSR_PENDSTSET: u32 = 1 << 26;
const ICSR_PENDSTCLR: u32 = 1 << 25;
const SYSTICK_HANDLER_COUNT: *mut u32 = 0x2000_0100 as *mut u32;
const SYSTICK_HANDLER_IPSR: *mut u32 = 0x2000_0104 as *mut u32;
const SYSTICK_HANDLER_REPEND: *mut u32 = 0x2000_0108 as *mut u32;
const SYSTICK_HANDLER_ICSR_FIRST: *mut u32 = 0x2000_010C as *mut u32;
const SYSTICK_HANDLER_ICSR_SECOND: *mut u32 = 0x2000_0110 as *mut u32;

const SPI1_BASE: u32 = 0x4001_3000;
const SPI1_IRQ: u32 = 35;
const SPI1_IRQ_BIT: u32 = 1 << (SPI1_IRQ % 32);
const SPI1_CR1: u32 = SPI1_BASE;
const SPI1_CR2: u32 = SPI1_BASE + 0x04;
const SPI1_SR: u32 = SPI1_BASE + 0x08;
const SPI1_DR: u32 = SPI1_BASE + 0x0C;
const SPI_SR_RXNE: u32 = 1 << 0;
const SPI_SR_TXE: u32 = 1 << 1;
const SPI_SR_BSY: u32 = 1 << 7;
const GPIOA_BSRR: u32 = GPIOA_BASE + 0x10;

const I2C1_BASE: u32 = 0x4000_5400;
const I2C1_CR1: u32 = I2C1_BASE + 0x00;
const I2C1_CR2: u32 = I2C1_BASE + 0x04;
const I2C1_OAR1: u32 = I2C1_BASE + 0x08;
const I2C1_DR: u32 = I2C1_BASE + 0x10;
const I2C1_SR1: u32 = I2C1_BASE + 0x14;
const I2C1_SR2: u32 = I2C1_BASE + 0x18;
const I2C1_CCR: u32 = I2C1_BASE + 0x1C;
const I2C1_TRISE: u32 = I2C1_BASE + 0x20;

const TMP102_ADDRESS: u8 = 0x48;
const BME280_ADDRESS: u8 = 0x76;

#[derive(Clone, Copy)]
struct Bme280Calibration {
    t1: u16,
    t2: i16,
    t3: i16,
    p1: u16,
    p2: i16,
    p3: i16,
    p4: i16,
    p5: i16,
    p6: i16,
    p7: i16,
    p8: i16,
    p9: i16,
    h1: u8,
    h2: i16,
    h3: u8,
    h4: i16,
    h5: i16,
    h6: i8,
}

#[derive(Clone, Copy)]
struct Bme280Raw {
    pressure: u32,
    temperature: u32,
    humidity: u32,
}

#[derive(Clone, Copy)]
struct Bme280Compensated {
    temperature_centi_c: i32,
    pressure_q8_pa: u32,
    humidity_q10_pct: u32,
}

impl Bme280Calibration {
    unsafe fn read_from_i2c() -> Option<Self> {
        let h4_high = read_i2c_byte(BME280_ADDRESS, 0xE4)?;
        let h4_h5_low = read_i2c_byte(BME280_ADDRESS, 0xE5)?;
        let h5_high = read_i2c_byte(BME280_ADDRESS, 0xE6)?;
        Some(Self {
            t1: read_bme280_u16_le(0x88)?,
            t2: read_bme280_u16_le(0x8A)? as i16,
            t3: read_bme280_u16_le(0x8C)? as i16,
            p1: read_bme280_u16_le(0x8E)?,
            p2: read_bme280_u16_le(0x90)? as i16,
            p3: read_bme280_u16_le(0x92)? as i16,
            p4: read_bme280_u16_le(0x94)? as i16,
            p5: read_bme280_u16_le(0x96)? as i16,
            p6: read_bme280_u16_le(0x98)? as i16,
            p7: read_bme280_u16_le(0x9A)? as i16,
            p8: read_bme280_u16_le(0x9C)? as i16,
            p9: read_bme280_u16_le(0x9E)? as i16,
            h1: read_i2c_byte(BME280_ADDRESS, 0xA1)?,
            h2: read_bme280_u16_le(0xE1)? as i16,
            h3: read_i2c_byte(BME280_ADDRESS, 0xE3)?,
            h4: ((h4_high as i8 as i16) << 4) | i16::from(h4_h5_low & 0x0F),
            h5: ((h5_high as i8 as i16) << 4) | i16::from(h4_h5_low >> 4),
            h6: read_i2c_byte(BME280_ADDRESS, 0xE7)? as i8,
        })
    }

    fn compensate(self, raw: Bme280Raw) -> Bme280Compensated {
        let adc_t = raw.temperature as i32;
        let t1 = self.t1 as i32;
        let var1 = (((adc_t >> 3) - (t1 << 1)) * self.t2 as i32) >> 11;
        let t_delta = (adc_t >> 4) - t1;
        let var2 = (((t_delta * t_delta) >> 12) * self.t3 as i32) >> 14;
        let t_fine = var1 + var2;
        let temperature_centi_c = (t_fine * 5 + 128) >> 8;

        let mut var1 = t_fine as i64 - 128_000;
        let mut var2 = var1 * var1 * self.p6 as i64;
        var2 += (var1 * self.p5 as i64) << 17;
        var2 += (self.p4 as i64) << 35;
        var1 = ((var1 * var1 * self.p3 as i64) >> 8) + ((var1 * self.p2 as i64) << 12);
        var1 = (((1i64 << 47) + var1) * self.p1 as i64) >> 33;
        let mut pressure = if var1 == 0 {
            0
        } else {
            let mut p = 1_048_576 - raw.pressure as i64;
            p = (((p << 31) - var2) * 3125) / var1;
            var1 = (self.p9 as i64 * (p >> 13) * (p >> 13)) >> 25;
            var2 = (self.p8 as i64 * p) >> 19;
            ((p + var1 + var2) >> 8) + ((self.p7 as i64) << 4)
        };
        pressure = pressure.clamp(0, u32::MAX as i64);

        let mut humidity = t_fine - 76_800;
        let humidity_left = (((raw.humidity as i32) << 14)
            - ((self.h4 as i32) << 20)
            - (self.h5 as i32 * humidity)
            + 16_384)
            >> 15;
        let humidity_var1 = (humidity * self.h6 as i32) >> 10;
        let humidity_var2 = ((humidity * self.h3 as i32) >> 11) + 32_768;
        let humidity_var3 = ((humidity_var1 * humidity_var2) >> 10) + 2_097_152;
        let humidity_right = (humidity_var3 * self.h2 as i32 + 8_192) >> 14;
        humidity = humidity_left * humidity_right;
        humidity -= ((((humidity >> 15) * (humidity >> 15)) >> 7) * self.h1 as i32) >> 4;
        humidity = humidity.clamp(0, 419_430_400);

        Bme280Compensated {
            temperature_centi_c,
            pressure_q8_pa: pressure as u32,
            humidity_q10_pct: (humidity >> 12) as u32,
        }
    }
}

impl Bme280Compensated {
    fn matches(self, temperature_centi_c: i32, pressure_pa: u32, humidity_pct: u32) -> bool {
        self.temperature_centi_c == temperature_centi_c
            && within(self.pressure_q8_pa, pressure_pa * 256, 128)
            && within(self.humidity_q10_pct, humidity_pct * 1024, 16)
    }
}

fn within(actual: u32, expected: u32, tolerance: u32) -> bool {
    if actual >= expected {
        actual - expected <= tolerance
    } else {
        expected - actual <= tolerance
    }
}

// ── Entry ─────────────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    unsafe {
        rcc_init();
        usart2_init();
    }
    putline(b"F103 I2C");
    unsafe { smoke_systick_exception_state() };
    unsafe { smoke_systick_external_pause_resume() };
    unsafe { smoke_systick_handler_delivery() };
    unsafe { smoke_systick_icsr_repend() };
    smoke_usart2_txe_irq();
    smoke_usart2_rx_enable_gate();
    smoke_spi1_irq_and_max31855();
    unsafe {
        i2c1_init();
    }
    print_state(b"INIT");

    // Start the transaction.
    unsafe { rmw32(I2C1_CR1, 1 << 8) }; // CR1.START
    wait_sr1(0x0001); // SB
    print_state(b"START");

    // Send address byte (write) for a non-existent slave (0xA0/2 = 0x50).
    unsafe { write_volatile(I2C1_DR as *mut u32, 0xA0) };
    // Wait for either ADDR (slave ACKed) or AF (no ACK).  On the bare bus
    // we expect AF.  Cap the polling at 16k iterations.
    for _ in 0..16_000 {
        let sr1 = unsafe { read_volatile(I2C1_SR1 as *const u32) };
        if sr1 & 0x0402 != 0 {
            break;
        }
    }
    print_state(b"ADDR");
    let addr_sr1 = unsafe { read_volatile(I2C1_SR1 as *const u32) };
    if addr_sr1 & 0x0400 != 0 {
        putline(b"I2C_NACK_OK");
    } else {
        putline(b"I2C_NACK_MISSING");
    }

    // Generate STOP and wait for the bus to settle.
    unsafe { rmw32(I2C1_CR1, 1 << 9) }; // CR1.STOP
    for _ in 0..16_000 {
        let sr2 = unsafe { read_volatile(I2C1_SR2 as *const u32) };
        if sr2 & 0x0002 == 0 {
            break;
        }
    }
    print_state(b"STOP");
    let stop_sr2 = unsafe { read_volatile(I2C1_SR2 as *const u32) };
    if stop_sr2 & 0x0002 == 0 {
        putline(b"I2C_IDLE_OK");
    } else {
        putline(b"I2C_IDLE_MISSING");
    }

    match unsafe { read_tmp102_register(0x00) } {
        Some(raw) => {
            print_reg(b"TMP102=", raw as u32);
            if raw == 0x1900 {
                putline(b"TMP102_READ_OK");
            } else {
                putline(b"TMP102_READ_MISMATCH");
            }
        }
        None => putline(b"TMP102_READ_FAIL"),
    }

    // Write all bits to the TMP102 configuration register. Its read-only
    // resolution/status bits keep reset state and its unused low bits stay
    // zero, so the declarative write mask should read back 0xFFF0.
    match unsafe { write_tmp102_register(0x01, 0xFFFF).and_then(|()| read_tmp102_register(0x01)) } {
        Some(raw) => {
            print_reg(b"TMP102_CONFIG=", raw as u32);
            if raw == 0xFFF0 {
                putline(b"TMP102_CONFIG_WRITE_OK");
            } else {
                putline(b"TMP102_CONFIG_WRITE_MISMATCH");
            }
        }
        None => putline(b"TMP102_CONFIG_WRITE_FAIL"),
    }

    // The TMP102 threshold registers are writable 16-bit words. D2:D0 are
    // reserved and read as zero; D15:D3 retain the firmware's programmed
    // alert limit. Distinct values also check byte order and register pointer
    // selection on both sides of the I2C transaction.
    match unsafe { write_tmp102_register(0x02, 0x3457).and_then(|()| read_tmp102_register(0x02)) } {
        Some(raw) => {
            print_reg(b"TMP102_TLOW=", raw as u32);
            if raw == 0x3450 {
                putline(b"TMP102_TLOW_WRITE_OK");
            } else {
                putline(b"TMP102_TLOW_WRITE_MISMATCH");
            }
        }
        None => putline(b"TMP102_TLOW_WRITE_FAIL"),
    }

    match unsafe { write_tmp102_register(0x03, 0x678E).and_then(|()| read_tmp102_register(0x03)) } {
        Some(raw) => {
            print_reg(b"TMP102_THIGH=", raw as u32);
            if raw == 0x6788 {
                putline(b"TMP102_THIGH_WRITE_OK");
            } else {
                putline(b"TMP102_THIGH_WRITE_MISMATCH");
            }
        }
        None => putline(b"TMP102_THIGH_WRITE_FAIL"),
    }

    match unsafe { write_tmp102_msb(0x02, 0x22).and_then(|()| read_tmp102_register(0x02)) } {
        Some(raw) => {
            print_reg(b"TMP102_TLOW_SHORT=", raw as u32);
            if raw == 0x2250 {
                putline(b"TMP102_TLOW_SHORT_WRITE_OK");
            } else {
                putline(b"TMP102_TLOW_SHORT_WRITE_MISMATCH");
            }
        }
        None => putline(b"TMP102_TLOW_SHORT_WRITE_FAIL"),
    }

    match unsafe { read_i2c_register(BME280_ADDRESS, 0xD0, 1) } {
        Some(bytes) => {
            let id = bytes[0];
            print_reg(b"BME280_ID=", u32::from(id));
            if id == 0x60 {
                putline(b"BME280_ID_READ_OK");
            } else {
                putline(b"BME280_ID_READ_MISMATCH");
            }
        }
        None => putline(b"BME280_ID_READ_FAIL"),
    }

    match unsafe { read_i2c_register(BME280_ADDRESS, 0x8A, 2) } {
        Some(bytes) => {
            let dig_t2 = u16::from_le_bytes(bytes);
            print_reg(b"BME280_T2=", u32::from(dig_t2));
            if dig_t2 == 26_435 {
                putline(b"BME280_CALIBRATION_READ_OK");
            } else {
                putline(b"BME280_CALIBRATION_READ_MISMATCH");
            }
        }
        None => putline(b"BME280_CALIBRATION_READ_FAIL"),
    }

    let calibration = match unsafe { Bme280Calibration::read_from_i2c() } {
        Some(calibration) => {
            putline(b"BME280_CALIBRATION_MAP_READ_OK");
            calibration
        }
        None => {
            putline(b"BME280_CALIBRATION_MAP_READ_FAIL");
            loop {}
        }
    };

    // Exercise the BME280's oversampling skip codes before enabling its
    // measurement channels. The Bosch datasheet specifies 0x80000 for skipped
    // temperature/pressure outputs and 0x8000 for skipped humidity output.
    putline(b"BME280_WAITING_FOR_OVERSAMPLING_SKIP");
    match unsafe { write_bme280_byte(0xF2, 0x00).and_then(|()| write_bme280_byte(0xF4, 0x03)) } {
        Some(()) => match (
            unsafe { read_bme280_pressure_raw() },
            unsafe { read_bme280_temperature_raw() },
            unsafe { read_bme280_humidity_raw() },
        ) {
            (Some(pressure), Some(temperature), Some(humidity)) => {
                print_reg(b"BME280_SKIPPED_PRESS_RAW=", pressure);
                print_reg(b"BME280_SKIPPED_TEMP_RAW=", temperature);
                print_reg(b"BME280_SKIPPED_HUM_RAW=", humidity);
                if pressure == 0x8_0000 && temperature == 0x8_0000 && humidity == 0x8_000 {
                    putline(b"BME280_OVERSAMPLING_SKIP_OK");
                } else {
                    putline(b"BME280_OVERSAMPLING_SKIP_MISMATCH");
                }
            }
            _ => putline(b"BME280_OVERSAMPLING_SKIP_READ_FAIL"),
        },
        None => putline(b"BME280_OVERSAMPLING_SKIP_CONFIG_FAIL"),
    }

    match unsafe { write_bme280_byte(0xF2, 0x01).and_then(|()| write_bme280_byte(0xF4, 0x27)) } {
        Some(()) => putline(b"BME280_MEASUREMENT_SETUP_OK"),
        None => putline(b"BME280_MEASUREMENT_SETUP_FAIL"),
    }

    // Read the BME280's measurement window as a driver does. Pressure and
    // temperature are 20-bit values split across three bytes; humidity is a
    // 16-bit word. The imported system seeds 40 °C / 80 %RH / 950 hPa; exact
    // raw-code comparisons check that placement config reaches all three
    // channels in the model.
    let pressure = unsafe { read_bme280_pressure_raw() };
    let temperature = unsafe { read_bme280_temperature_raw() };
    let humidity = unsafe { read_bme280_humidity_raw() };

    match (pressure, temperature, humidity) {
        (Some(pressure), Some(temperature), Some(humidity)) => {
            print_reg(b"BME280_PRESS_RAW=", pressure);
            print_reg(b"BME280_TEMP_RAW=", temperature);
            print_reg(b"BME280_HUM_RAW=", humidity);
            if pressure == 362_559 && temperature == 583_952 && humidity == 34_111 {
                putline(b"BME280_SEEDED_ENVIRONMENT_OK");
            } else {
                putline(b"BME280_SEEDED_ENVIRONMENT_MISMATCH");
            }
            let compensated = calibration.compensate(Bme280Raw {
                pressure,
                temperature,
                humidity,
            });
            print_bme280_compensated(
                b"BME280_SEEDED_TEMP_CENTI=",
                b"BME280_SEEDED_PRESS_Q8=",
                b"BME280_SEEDED_HUM_Q10=",
                compensated,
            );
            if compensated.matches(4_000, 95_000, 80) {
                putline(b"BME280_SEEDED_COMPENSATION_OK");
            } else {
                putline(b"BME280_SEEDED_COMPENSATION_MISMATCH");
            }
        }
        _ => putline(b"BME280_RAW_READ_FAIL"),
    }

    // The test script changes the live temperature input after the seeded
    // sample has been read. Poll the real register path until that changed
    // input becomes visible; the smoke then checks the exact raw ADC code.
    putline(b"BME280_WAITING_FOR_TEMPERATURE_STIMULUS");
    if let Some(initial_temperature) = temperature {
        loop {
            if let Some(current_temperature) = unsafe { read_bme280_temperature_raw() } {
                if current_temperature != initial_temperature {
                    print_reg(b"BME280_DYNAMIC_TEMP_RAW=", current_temperature);
                    if current_temperature == 632_080 {
                        putline(b"BME280_DYNAMIC_TEMPERATURE_OK");
                    } else {
                        putline(b"BME280_DYNAMIC_TEMPERATURE_MISMATCH");
                    }
                    if let (Some(pressure), Some(humidity)) =
                        (unsafe { read_bme280_pressure_raw() }, unsafe {
                            read_bme280_humidity_raw()
                        })
                    {
                        let compensated = calibration.compensate(Bme280Raw {
                            pressure,
                            temperature: current_temperature,
                            humidity,
                        });
                        print_bme280_compensated(
                            b"BME280_DYNAMIC_TEMP_CENTI=",
                            b"BME280_DYNAMIC_PRESS_Q8=",
                            b"BME280_DYNAMIC_HUM_Q10=",
                            compensated,
                        );
                        if compensated.matches(5_500, 95_000, 80) {
                            putline(b"BME280_DYNAMIC_TEMP_COMPENSATION_OK");
                        } else {
                            putline(b"BME280_DYNAMIC_TEMP_COMPENSATION_MISMATCH");
                        }
                    } else {
                        putline(b"BME280_DYNAMIC_TEMP_COMPENSATION_READ_FAIL");
                    }
                    break;
                }
            }
        }
    } else {
        putline(b"BME280_DYNAMIC_TEMPERATURE_READ_FAIL");
    }

    // The temperature change also re-derives the humidity ADC code. Wait for
    // compensated 35 %RH so the temperature stimulus cannot satisfy this
    // check by itself.
    putline(b"BME280_WAITING_FOR_HUMIDITY_STIMULUS");
    if let (Some(pressure), Some(temperature)) = (unsafe { read_bme280_pressure_raw() }, unsafe {
        read_bme280_temperature_raw()
    }) {
        loop {
            if let Some(current_humidity) = unsafe { read_bme280_humidity_raw() } {
                let compensated = calibration.compensate(Bme280Raw {
                    pressure,
                    temperature,
                    humidity: current_humidity,
                });
                if compensated.matches(5_500, 95_000, 35) {
                    print_reg(b"BME280_DYNAMIC_HUM_RAW=", current_humidity);
                    putline(b"BME280_DYNAMIC_HUMIDITY_OK");
                    print_bme280_compensated(
                        b"BME280_HUM_TEMP_CENTI=",
                        b"BME280_HUM_PRESS_Q8=",
                        b"BME280_HUM_Q10=",
                        compensated,
                    );
                    putline(b"BME280_DYNAMIC_HUM_COMPENSATION_OK");
                    break;
                }
            }
        }
    } else {
        putline(b"BME280_DYNAMIC_HUMIDITY_COMPENSATION_READ_FAIL");
    }

    // Wait for compensated 1012.5 hPa. The earlier temperature change also
    // re-derived pressure's ADC code, so a raw-code change alone is not enough.
    putline(b"BME280_WAITING_FOR_PRESSURE_STIMULUS");
    if let (Some(temperature), Some(humidity)) =
        (unsafe { read_bme280_temperature_raw() }, unsafe {
            read_bme280_humidity_raw()
        })
    {
        loop {
            if let Some(current_pressure) = unsafe { read_bme280_pressure_raw() } {
                let compensated = calibration.compensate(Bme280Raw {
                    pressure: current_pressure,
                    temperature,
                    humidity,
                });
                if compensated.matches(5_500, 101_250, 35) {
                    print_reg(b"BME280_DYNAMIC_PRESS_RAW=", current_pressure);
                    putline(b"BME280_DYNAMIC_PRESSURE_OK");
                    print_bme280_compensated(
                        b"BME280_PRESS_TEMP_CENTI=",
                        b"BME280_PRESS_Q8=",
                        b"BME280_PRESS_HUM_Q10=",
                        compensated,
                    );
                    putline(b"BME280_DYNAMIC_PRESS_COMPENSATION_OK");
                    break;
                }
            }
        }
    } else {
        putline(b"BME280_DYNAMIC_PRESSURE_COMPENSATION_READ_FAIL");
    }

    // Exercise the existing model-backed BME280 control-register storage over
    // the same STM32F1 I2C path. This checks transaction and read-back only;
    // measurement mode, oversampling, timing and filtering remain unmodelled.
    match unsafe {
        write_bme280_byte(0xF2, 0x01).and_then(|()| read_i2c_byte(BME280_ADDRESS, 0xF2))
    } {
        Some(value) => {
            print_reg(b"BME280_CTRL_HUM_STORAGE=", u32::from(value));
            if value == 0x01 {
                putline(b"BME280_CTRL_HUM_STORAGE_OK");
            } else {
                putline(b"BME280_CTRL_HUM_STORAGE_MISMATCH");
            }
        }
        None => putline(b"BME280_CTRL_HUM_STORAGE_FAIL"),
    }

    match unsafe {
        write_bme280_byte(0xF4, 0x27).and_then(|()| read_i2c_byte(BME280_ADDRESS, 0xF4))
    } {
        Some(value) => {
            print_reg(b"BME280_CTRL_MEAS_STORAGE=", u32::from(value));
            if value == 0x27 {
                putline(b"BME280_CTRL_MEAS_STORAGE_OK");
            } else {
                putline(b"BME280_CTRL_MEAS_STORAGE_MISMATCH");
            }
        }
        None => putline(b"BME280_CTRL_MEAS_STORAGE_FAIL"),
    }

    putline(b"DONE");

    loop {}
}

/// Exercise the Cortex-M3 SysTick peripheral and SCB pending state on the
/// exact imported STM32F103 profile. PRIMASK keeps the request pending while
/// the timer is disabled and COUNTFLAG is read.
unsafe fn smoke_systick_exception_state() -> bool {
    let prior_primask: u32;
    core::arch::asm!(
        "mrs {primask}, PRIMASK",
        "cpsid i",
        primask = out(reg) prior_primask,
        options(nomem, nostack, preserves_flags),
    );

    write_volatile(SYSTICK_CSR as *mut u32, 0);
    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR);
    write_volatile(SYSTICK_RVR as *mut u32, 63);
    write_volatile(SYSTICK_CVR as *mut u32, 0);
    write_volatile(SYSTICK_CSR as *mut u32, 0x7); // ENABLE | TICKINT | CLKSOURCE
    let calibration_is_f103 = read_volatile(SYSTICK_CALIB as *const u32) == 9_000;

    let mut pending_before_disable = false;
    for _ in 0..4_096 {
        if read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSTSET != 0 {
            pending_before_disable = true;
            break;
        }
    }

    // Disabling the counter must leave COUNTFLAG latched. Reading CSR returns
    // and clears COUNTFLAG, independently of the already pending exception.
    write_volatile(SYSTICK_CSR as *mut u32, 0);
    let countflag_after_disable = read_volatile(SYSTICK_CSR as *const u32) & SYSTICK_COUNTFLAG != 0;
    let pending_after_countflag_read = read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSTSET != 0;
    let countflag_cleared_by_read =
        read_volatile(SYSTICK_CSR as *const u32) & SYSTICK_COUNTFLAG == 0;

    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR);
    let pending_cleared = read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSTSET == 0;
    core::arch::asm!(
        "msr PRIMASK, {primask}",
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );

    let ok = prior_primask == 0
        && calibration_is_f103
        && pending_before_disable
        && countflag_after_disable
        && pending_after_countflag_read
        && countflag_cleared_by_read
        && pending_cleared;
    if ok {
        putline(
            b"SYSTICK M3 CALIB=00002328 WRAP=1 COUNTFLAG_WRITE=1 READ_CLEAR=1 PENDING=1 CLEAR=1 ok",
        );
    } else {
        putline(b"SYSTICK M3 CHECK FAILED");
    }
    ok
}

/// RM0008 configures the STM32F103 SysTick reference source as HCLK/8.
/// Disabling ENABLE must freeze the active countdown and re-enabling it must
/// continue the same period.
unsafe fn smoke_systick_external_pause_resume() -> bool {
    const RELOAD: u32 = 4095;

    write_volatile(SYSTICK_CSR as *mut u32, 0);
    write_volatile(SYSTICK_RVR as *mut u32, RELOAD);
    write_volatile(SYSTICK_CVR as *mut u32, 0);
    write_volatile(SYSTICK_CSR as *mut u32, 0x1); // ENABLE; HCLK/8 reference.
    let reference_selected = read_volatile(SYSTICK_CSR as *const u32) & 0x5 == 0x1;

    let mut pause_window = false;
    for _ in 0..20_000 {
        let value = read_volatile(SYSTICK_CVR as *const u32);
        if (1024..=3072).contains(&value) {
            pause_window = true;
            break;
        }
        core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
    }

    write_volatile(SYSTICK_CSR as *mut u32, 0);
    let paused = read_volatile(SYSTICK_CVR as *const u32);
    for _ in 0..128 {
        core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
    }
    let frozen = read_volatile(SYSTICK_CVR as *const u32);

    write_volatile(SYSTICK_CSR as *mut u32, 0x1);
    for _ in 0..128 {
        core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
    }
    let resumed = read_volatile(SYSTICK_CVR as *const u32);
    write_volatile(SYSTICK_CSR as *mut u32, 0);
    write_volatile(SYSTICK_CVR as *mut u32, 0);

    let frozen_ok = paused == frozen;
    let resumed_ok = pause_window && paused > 256 && resumed > 0 && resumed < paused;
    let ok = reference_selected && frozen_ok && resumed_ok;
    if ok {
        putline(b"SYSTICK M3 HCLK8 REFERENCE=1 WINDOW=1 FROZEN=1 RESUMED=1 ok");
    } else {
        putline(b"SYSTICK M3 HCLK8 CHECK FAILED");
    }
    ok
}

/// Exercise periodic exception entry, vector dispatch and exception return on
/// the exact imported Cortex-M3 system. The linker places SysTick_Handler in
/// vector slot 15; require two deliveries so the counter must reload after the
/// first exception returns.
unsafe fn smoke_systick_handler_delivery() -> bool {
    write_volatile(SYSTICK_CSR as *mut u32, 0);
    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR);
    write_volatile(SCB_VTOR as *mut u32, 0x0800_0000);
    write_volatile(SYSTICK_HANDLER_COUNT, 0);
    write_volatile(SYSTICK_HANDLER_IPSR, 0);
    write_volatile(SYSTICK_HANDLER_REPEND, 0);
    write_volatile(SYSTICK_RVR as *mut u32, 255);
    write_volatile(SYSTICK_CVR as *mut u32, 0);
    write_volatile(SYSTICK_CSR as *mut u32, 0x7); // ENABLE | TICKINT | CLKSOURCE

    for _ in 0..4_096 {
        if read_volatile(SYSTICK_HANDLER_COUNT) >= 2 {
            break;
        }
        core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
    }

    let prior_primask: u32;
    core::arch::asm!(
        "mrs {primask}, PRIMASK",
        "cpsid i",
        primask = out(reg) prior_primask,
        options(nostack, preserves_flags),
    );
    write_volatile(SYSTICK_CSR as *mut u32, 0);
    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR);
    let deliveries = read_volatile(SYSTICK_HANDLER_COUNT);
    core::arch::asm!(
        "msr PRIMASK, {primask}",
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );
    for _ in 0..128 {
        core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
    }
    let stable_after_disable = read_volatile(SYSTICK_HANDLER_COUNT) == deliveries;
    let ipsr = read_volatile(SYSTICK_HANDLER_IPSR);
    let vector_installed = read_volatile(SCB_VTOR as *const u32) == 0x0800_0000;
    let ok = prior_primask == 0
        && deliveries >= 2
        && stable_after_disable
        && ipsr == 15
        && vector_installed;

    if ok {
        putline(b"SYSTICK M3 PERIODIC=1 REPEATED=1 VECTOR=15 IPSR=15 RETURN=1 ok");
    } else {
        putline(b"SYSTICK M3 HANDLER CHECK FAILED");
    }
    ok
}

/// Verify ICSR.PENDSTSET/PENDSTCLR and that a software pend written from an
/// active SysTick handler is delivered once the first handler returns.
unsafe fn smoke_systick_icsr_repend() -> bool {
    let source_vtor = read_volatile(SCB_VTOR as *const u32);
    let prior_primask: u32;
    core::arch::asm!(
        "mrs {primask}, PRIMASK",
        "cpsid i",
        primask = out(reg) prior_primask,
        options(nomem, nostack, preserves_flags),
    );

    write_volatile(SYSTICK_CSR as *mut u32, 0);
    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR);
    write_volatile(SCB_VTOR as *mut u32, 0x0800_0000);
    write_volatile(SYSTICK_HANDLER_COUNT, 0);
    write_volatile(SYSTICK_HANDLER_IPSR, 0);
    write_volatile(SYSTICK_HANDLER_REPEND, 1);
    write_volatile(SYSTICK_HANDLER_ICSR_FIRST, 0);
    write_volatile(SYSTICK_HANDLER_ICSR_SECOND, 0);

    // A pending SysTick is a single latched request. Repeated set writes
    // coalesce, and PENDSTCLR cancels it before exception entry.
    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTSET);
    let first_set_readback = read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSTSET != 0;
    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTSET);
    let coalesced_readback = read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSTSET != 0;
    let no_handler_while_masked = read_volatile(SYSTICK_HANDLER_COUNT) == 0;
    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR);
    let clear_readback = read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSTSET == 0;
    for _ in 0..64 {
        core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
    }
    let canceled_before_delivery = read_volatile(SYSTICK_HANDLER_COUNT) == 0;

    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTSET);
    let pending_before_unmask = read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSTSET != 0;
    core::arch::asm!(
        "msr PRIMASK, {primask}",
        "dsb",
        "isb",
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );

    for _ in 0..100_000 {
        if read_volatile(SYSTICK_HANDLER_COUNT) >= 2 {
            break;
        }
        core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
    }

    let mask_again: u32;
    core::arch::asm!(
        "mrs {primask}, PRIMASK",
        "cpsid i",
        primask = out(reg) mask_again,
        options(nomem, nostack, preserves_flags),
    );
    write_volatile(SYSTICK_CSR as *mut u32, 0);
    write_volatile(SYSTICK_HANDLER_REPEND, 0);
    write_volatile(SCB_VTOR as *mut u32, source_vtor);
    let deliveries = read_volatile(SYSTICK_HANDLER_COUNT);
    let pending_after_return = read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSTSET == 0;
    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR);
    core::arch::asm!(
        "msr PRIMASK, {primask}",
        "dsb",
        "isb",
        primask = in(reg) mask_again,
        options(nostack, preserves_flags),
    );
    for _ in 0..128 {
        core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
    }

    let stable_after_repend = read_volatile(SYSTICK_HANDLER_COUNT) == deliveries;
    let first_icsr = read_volatile(SYSTICK_HANDLER_ICSR_FIRST);
    let second_icsr = read_volatile(SYSTICK_HANDLER_ICSR_SECOND);
    let thread_ipsr: u32;
    core::arch::asm!(
        "mrs {ipsr}, IPSR",
        ipsr = out(reg) thread_ipsr,
        options(nomem, nostack, preserves_flags),
    );
    let vector_restored = read_volatile(SCB_VTOR as *const u32) == source_vtor;
    let ok = prior_primask == 0
        && first_set_readback
        && coalesced_readback
        && no_handler_while_masked
        && clear_readback
        && canceled_before_delivery
        && pending_before_unmask
        && deliveries == 2
        && read_volatile(SYSTICK_HANDLER_IPSR) == 15
        && first_icsr & 0x1FF == 15
        && second_icsr & 0x1FF == 15
        && first_icsr & ICSR_PENDSTSET == 0
        && second_icsr & ICSR_PENDSTSET == 0
        && pending_after_return
        && stable_after_repend
        && thread_ipsr == 0
        && source_vtor == 0x0800_0000
        && vector_restored;

    if ok {
        putline(b"SYSTICK M3 ICSR SET=1 COALESCE=1 CLEAR=1 REPEND=1 ENTRIES=2 ACTIVE=15 ok");
    } else {
        putline(b"SYSTICK M3 ICSR REPEND CHECK FAILED");
    }
    ok
}

/// Cortex-M exception handler installed at vector-table slot 15 by minimal.ld.
#[no_mangle]
pub extern "C" fn SysTick_Handler() {
    unsafe {
        let count = read_volatile(SYSTICK_HANDLER_COUNT);
        let icsr = read_volatile(SCB_ICSR as *const u32);
        if count == 0 {
            write_volatile(SYSTICK_HANDLER_ICSR_FIRST, icsr);
            if read_volatile(SYSTICK_HANDLER_REPEND) != 0 {
                write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTSET);
            }
        } else if count == 1 {
            write_volatile(SYSTICK_HANDLER_ICSR_SECOND, icsr);
        }
        let ipsr: u32;
        core::arch::asm!(
            "mrs {ipsr}, IPSR",
            ipsr = out(reg) ipsr,
            options(nomem, nostack, preserves_flags),
        );
        write_volatile(SYSTICK_HANDLER_IPSR, ipsr);
        write_volatile(SYSTICK_HANDLER_COUNT, count.wrapping_add(1));
    }
}

// ── Init helpers ──────────────────────────────────────────────────────────────

unsafe fn rcc_init() {
    // Enable GPIOA + GPIOB + AFIO + SPI1 on APB2 (bits 2, 3, 0, 12). GPIOB is
    // required because I2C1's default pin mapping is PB6 (SCL) / PB7
    // (SDA); without IOPBEN the GPIOB CRL writes go to a dead clock
    // domain and the I²C state machine never gets the bus pins it
    // needs to drive — which surfaces on real silicon as a stalled
    // START condition (CR1.START stays set, SB never fires).  The
    // simulator's I²C model does NOT check GPIO config so the same
    // firmware "works" there even without IOPBEN; matching silicon
    // is mandatory.
    rmw32(RCC_APB2ENR, (1 << 0) | (1 << 2) | (1 << 3) | (1 << 12));
    // Enable USART2 + I2C1 on APB1 (bits 17 and 21).
    rmw32(RCC_APB1ENR, (1 << 17) | (1 << 21));
}

unsafe fn usart2_init() {
    // PA2 = AF push-pull, 50 MHz output → CRL bits[11:8] = 0b1011.
    let crl = read_volatile(GPIOA_CRL as *const u32);
    let crl = (crl & !(0xF << 8)) | (0xB << 8);
    write_volatile(GPIOA_CRL as *mut u32, crl);

    // BRR = PCLK1 / baud.  PCLK1 = 8 MHz (HSI default, no PLL),
    // baud = 115200 → BRR = 0x45 (DIV_M=4, DIV_F=5).
    write_volatile(USART2_BRR as *mut u32, 0x45);
    // CR1: UE (13) | TE (3).
    write_volatile(USART2_CR1 as *mut u32, (1 << 13) | (1 << 3));
}

fn smoke_usart2_txe_irq() {
    // TXE is set at idle. With TXEIE enabled, USART2 must pend its shared
    // external IRQ (IRQ 38, RM0008 vector table). Leave NVIC disabled so the
    // smoke observes ISPR without taking the default handler.
    unsafe {
        write_volatile(
            USART2_CR1 as *mut u32,
            (1 << 13) | (1 << 3) | (1 << 7), // UE | TE | TXEIE
        );
    }

    let bank = (USART2_IRQ / 32) as usize;
    let mut pended = false;
    for _ in 0..64 {
        if unsafe { read_volatile(NVIC_ISPR.add(bank)) } & USART2_IRQ_BIT != 0 {
            pended = true;
            break;
        }
        core::hint::spin_loop();
    }

    unsafe {
        write_volatile(
            USART2_CR1 as *mut u32,
            (1 << 13) | (1 << 3), // UE | TE, TXEIE off
        );
        write_volatile(NVIC_ICPR.add(bank), USART2_IRQ_BIT);
    }
    for _ in 0..64 {
        core::hint::spin_loop();
    }
    let released = unsafe { read_volatile(NVIC_ISPR.add(bank)) } & USART2_IRQ_BIT == 0;

    if pended && released {
        putline(b"USART2 TXEIRQ=1 REARM=0");
    } else {
        putline(b"USART2 TXEIRQ=0 REARM=1");
    }
}

/// Start-injected USART2 bytes must remain on the wire until both legacy
/// receiver enables are set, then arrive in order and drain from DR.
fn smoke_usart2_rx_enable_gate() {
    // `usart2_init` leaves UE and TE set with RE clear.
    let re_off = unsafe { read_volatile(USART2_SR as *const u32) } & USART_SR_RXNE == 0;

    // Isolate UE as the second gate: RE alone must not expose the queued data.
    unsafe { write_volatile(USART2_CR1 as *mut u32, (1 << 3) | (1 << 2)) };
    let ue_off = unsafe {
        read_volatile(USART2_CR1 as *const u32) & (1 << 13) == 0
            && read_volatile(USART2_SR as *const u32) & USART_SR_RXNE == 0
    };

    // Re-enable the UART and receiver, then drain the two injected bytes.
    unsafe { write_volatile(USART2_CR1 as *mut u32, (1 << 13) | (1 << 3) | (1 << 2)) };
    let mut received = [0u8; 2];
    let mut count = 0;
    for _ in 0..1_000_000 {
        if unsafe { read_volatile(USART2_SR as *const u32) } & USART_SR_RXNE != 0
            && count < received.len()
        {
            received[count] = unsafe { read_volatile(USART2_DR as *const u32) } as u8;
            count += 1;
        }
        if count == received.len() {
            break;
        }
        core::hint::spin_loop();
    }
    let released = unsafe { read_volatile(USART2_SR as *const u32) } & USART_SR_RXNE == 0;
    let passed = re_off && ue_off && count == 2 && received == *b"RS" && released;

    putbytes(b"USART2 RX GATE re_off=");
    put_flag(re_off);
    putbytes(b" ue_off=");
    put_flag(ue_off);
    putbytes(b" data=");
    putbytes(b"0000");
    for byte in received {
        for shift in [4, 0] {
            let nibble = (byte >> shift) & 0x0F;
            putbyte(if nibble < 10 {
                b'0' + nibble
            } else {
                b'A' + nibble - 10
            });
        }
    }
    putbytes(b" released=");
    put_flag(released);
    putbytes(if passed { b" ok\n" } else { b" FAIL\n" });
}

fn smoke_spi1_irq_and_max31855() {
    unsafe {
        // PA4 is a GPIO chip select for the fixture MAX31855. Configure it as
        // 50 MHz push-pull output and leave the device deselected.
        let crl = read_volatile(GPIOA_CRL as *const u32);
        write_volatile(GPIOA_CRL as *mut u32, (crl & !(0xF << 16)) | (0x3 << 16));
        write_volatile(GPIOA_BSRR as *mut u32, 1 << 4);

        // SPI1 master, software NSS, fPCLK/8, mode 0, 8-bit frames.
        write_volatile(
            SPI1_CR1 as *mut u32,
            (1 << 9) | (1 << 8) | (0b010 << 3) | (1 << 2),
        );
        write_volatile(
            SPI1_CR1 as *mut u32,
            read_volatile(SPI1_CR1 as *const u32) | (1 << 6), // SPE
        );
        write_volatile(NVIC_ICPR.add((SPI1_IRQ / 32) as usize), SPI1_IRQ_BIT);

        // TXE is high at idle; TXEIE must assert and then release SPI1's line.
        write_volatile(SPI1_CR2 as *mut u32, 1 << 7);
    }
    let txe_pended =
        unsafe { read_volatile(NVIC_ISPR.add((SPI1_IRQ / 32) as usize)) & SPI1_IRQ_BIT != 0 };
    unsafe {
        write_volatile(SPI1_CR2 as *mut u32, 0);
        write_volatile(NVIC_ICPR.add((SPI1_IRQ / 32) as usize), SPI1_IRQ_BIT);
    }
    let txe_released =
        unsafe { read_volatile(NVIC_ISPR.add((SPI1_IRQ / 32) as usize)) & SPI1_IRQ_BIT == 0 };

    unsafe { write_volatile(SPI1_CR2 as *mut u32, 1 << 6) }; // RXNEIE
    unsafe { write_volatile(GPIOA_BSRR as *mut u32, 1 << (4 + 16)) }; // PA4 low
    let mut frame = 0u32;
    let mut rxne_pended = false;
    let mut transfer_ok = true;
    for _ in 0..4 {
        if !wait_spi(SPI1_SR, SPI_SR_TXE, SPI_SR_TXE) {
            transfer_ok = false;
            break;
        }
        unsafe { write_volatile(SPI1_DR as *mut u32, 0) };
        if !wait_spi(SPI1_SR, SPI_SR_RXNE, SPI_SR_RXNE) {
            transfer_ok = false;
            break;
        }
        let rxne = unsafe { read_volatile(SPI1_SR as *const u32) } & SPI_SR_RXNE != 0;
        let pending =
            unsafe { read_volatile(NVIC_ISPR.add((SPI1_IRQ / 32) as usize)) & SPI1_IRQ_BIT != 0 };
        rxne_pended |= rxne && pending;
        frame = (frame << 8) | (unsafe { read_volatile(SPI1_DR as *const u32) } & 0xFF);
    }
    let idle = wait_spi(SPI1_SR, SPI_SR_BSY, 0);
    unsafe {
        write_volatile(GPIOA_BSRR as *mut u32, 1 << 4); // PA4 high
    }
    let rxne_released = unsafe {
        read_volatile(NVIC_ISPR.add((SPI1_IRQ / 32) as usize)) & SPI1_IRQ_BIT == 0
            && read_volatile(SPI1_SR as *const u32) & SPI_SR_RXNE == 0
    };
    unsafe { write_volatile(SPI1_CR2 as *mut u32, 0) };

    print_reg(b"SPI1 MAX31855=", frame);
    putbytes(b"SPI1 TXEIRQ=");
    put_flag(txe_pended);
    putbytes(b" REARM=");
    put_flag(!txe_released);
    putbytes(b" RXNEIRQ=");
    put_flag(rxne_pended);
    putbytes(b" RELEASED=");
    put_flag(rxne_released);
    putbyte(b'\n');
    let checks_passed = transfer_ok
        && idle
        && frame == 0x0190_1600
        && txe_pended
        && txe_released
        && rxne_pended
        && rxne_released;
    putbytes(b"SPI1 CHECKS=");
    put_flag(checks_passed);
    putbyte(b'\n');
    if !checks_passed {
        putline(b"SPI1_CHECK_FAIL");
    }
}

fn wait_spi(addr: u32, mask: u32, want: u32) -> bool {
    for _ in 0..100_000 {
        if unsafe { read_volatile(addr as *const u32) } & mask == want {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

fn put_flag(value: bool) {
    putbyte(if value { b'1' } else { b'0' });
}

unsafe fn i2c1_init() {
    // PB6 (SCL) and PB7 (SDA): AF open-drain 50 MHz → nibble 0xF in
    // GPIOB.CRL bits[27:24] and bits[31:28].  Required on real silicon;
    // ignored by the simulator's I²C model but matching it keeps the
    // sim/HW traces identical.
    let crl = read_volatile(GPIOB_CRL as *const u32);
    let crl = (crl & !(0xFF << 24)) | (0xFF << 24);
    write_volatile(GPIOB_CRL as *mut u32, crl);

    // CR2.FREQ = 8 (8 MHz APB1 clock).
    write_volatile(I2C1_CR2 as *mut u32, 8);
    // CCR for standard-mode 100 kHz: CCR = PCLK1 / (2 * SCL) = 40.
    write_volatile(I2C1_CCR as *mut u32, 40);
    // TRISE = PCLK1_MHz + 1 = 9.
    write_volatile(I2C1_TRISE as *mut u32, 9);
    // OAR1: set the reserved bit 14 per RM0008 §26.6.3.
    write_volatile(I2C1_OAR1 as *mut u32, 1 << 14);
    // PE.
    write_volatile(I2C1_CR1 as *mut u32, 1);
}

/// Write a 16-bit big-endian TMP102 register through the F1 I2C1 master.
unsafe fn write_tmp102_register(pointer: u8, value: u16) -> Option<()> {
    begin_i2c_write(TMP102_ADDRESS, pointer)?;
    let [msb, lsb] = value.to_be_bytes();
    write_volatile(I2C1_DR as *mut u32, u32::from(msb));
    if !wait_sr1(0x0004) {
        return None;
    }
    write_volatile(I2C1_DR as *mut u32, u32::from(lsb));
    if !wait_sr1(0x0004) {
        return None;
    }
    rmw32(I2C1_CR1, 1 << 9); // STOP
    for _ in 0..16_000 {
        if read_volatile(I2C1_SR2 as *const u32) & 0x0002 == 0 {
            return Some(());
        }
    }
    None
}

/// Update only a TMP102 register's most-significant byte, then terminate with
/// STOP so the device commits the documented single-byte update.
unsafe fn write_tmp102_msb(pointer: u8, msb: u8) -> Option<()> {
    begin_i2c_write(TMP102_ADDRESS, pointer)?;
    write_volatile(I2C1_DR as *mut u32, u32::from(msb));
    if !wait_sr1(0x0004) {
        return None;
    }
    rmw32(I2C1_CR1, 1 << 9); // STOP
    for _ in 0..16_000 {
        if read_volatile(I2C1_SR2 as *const u32) & 0x0002 == 0 {
            return Some(());
        }
    }
    None
}

/// Write one byte to a BME280 register through the F1 I2C1 master.
unsafe fn write_bme280_byte(pointer: u8, value: u8) -> Option<()> {
    begin_i2c_write(BME280_ADDRESS, pointer)?;
    write_volatile(I2C1_DR as *mut u32, u32::from(value));
    if !wait_sr1(0x0004) {
        return None;
    }
    rmw32(I2C1_CR1, 1 << 9); // STOP
    for _ in 0..16_000 {
        if read_volatile(I2C1_SR2 as *const u32) & 0x0002 == 0 {
            return Some(());
        }
    }
    None
}

/// Read a 16-bit big-endian TMP102 register using the F1 write-pointer /
/// repeated-START / master-receive sequence.
unsafe fn read_tmp102_register(pointer: u8) -> Option<u16> {
    let [msb, lsb] = read_i2c_register(TMP102_ADDRESS, pointer, 2)?;
    Some(u16::from_be_bytes([msb, lsb]))
}

/// Read one or two bytes from an addressed I2C device after selecting its
/// register. The returned bytes retain their on-wire order; callers apply the
/// device's register endianness.
unsafe fn read_i2c_register(address: u8, pointer: u8, length: u8) -> Option<[u8; 2]> {
    if !(length == 1 || length == 2) {
        return None;
    }
    begin_i2c_write(address, pointer)?;

    // Repeated START, then address for a read. ACK is armed before the address
    // phase for a two-byte transfer; the one-byte path clears it before ADDR.
    rmw32(I2C1_CR1, 1 << 8);
    if !wait_sr1(0x0001) {
        return None;
    }
    if length == 2 {
        rmw32(I2C1_CR1, 1 << 10); // ACK
    }
    write_volatile(I2C1_DR as *mut u32, u32::from((address << 1) | 1));
    if !wait_sr1(0x0002) {
        return None;
    }

    if length == 1 {
        let cr1 = read_volatile(I2C1_CR1 as *const u32);
        write_volatile(I2C1_CR1 as *mut u32, cr1 & !(1 << 10)); // NACK
        clear_addr();
        rmw32(I2C1_CR1, 1 << 9); // STOP
        if !wait_sr1(1 << 6) {
            return None;
        }
        let byte = read_volatile(I2C1_DR as *const u32) as u8;
        for _ in 0..16_000 {
            if read_volatile(I2C1_SR2 as *const u32) & 0x0002 == 0 {
                return Some([byte, 0]);
            }
        }
        return None;
    }

    clear_addr();

    if !wait_sr1(1 << 6) {
        return None;
    }
    let msb = read_volatile(I2C1_DR as *const u32) as u8;
    // The second byte is the final byte of this read, so NACK it.
    let cr1 = read_volatile(I2C1_CR1 as *const u32);
    write_volatile(I2C1_CR1 as *mut u32, cr1 & !(1 << 10));
    if !wait_sr1(1 << 6) {
        return None;
    }
    let lsb = read_volatile(I2C1_DR as *const u32) as u8;

    rmw32(I2C1_CR1, 1 << 9); // STOP
    for _ in 0..16_000 {
        if read_volatile(I2C1_SR2 as *const u32) & 0x0002 == 0 {
            return Some([msb, lsb]);
        }
    }
    None
}

/// Read one byte from an I2C register using the STM32F1 final-byte NACK path.
unsafe fn read_i2c_byte(address: u8, pointer: u8) -> Option<u8> {
    read_i2c_register(address, pointer, 1).map(|bytes| bytes[0])
}

/// Read the BME280's left-aligned 20-bit temperature ADC value over I²C1.
unsafe fn read_bme280_temperature_raw() -> Option<u32> {
    read_i2c_register(BME280_ADDRESS, 0xFA, 2).and_then(|bytes| {
        read_i2c_byte(BME280_ADDRESS, 0xFC).map(|xlsb| {
            (u32::from(bytes[0]) << 12) | (u32::from(bytes[1]) << 4) | (u32::from(xlsb) >> 4)
        })
    })
}

/// Read the BME280's left-aligned 20-bit pressure ADC value over I2C1.
unsafe fn read_bme280_pressure_raw() -> Option<u32> {
    read_i2c_register(BME280_ADDRESS, 0xF7, 2).and_then(|bytes| {
        read_i2c_byte(BME280_ADDRESS, 0xF9).map(|xlsb| {
            (u32::from(bytes[0]) << 12) | (u32::from(bytes[1]) << 4) | (u32::from(xlsb) >> 4)
        })
    })
}

/// Read the BME280's 16-bit humidity ADC value over I2C1.
unsafe fn read_bme280_humidity_raw() -> Option<u32> {
    read_i2c_register(BME280_ADDRESS, 0xFD, 2)
        .map(|bytes| (u32::from(bytes[0]) << 8) | u32::from(bytes[1]))
}

/// Read one little-endian BME280 calibration coefficient through I2C1.
unsafe fn read_bme280_u16_le(register: u8) -> Option<u16> {
    read_i2c_register(BME280_ADDRESS, register, 2).map(u16::from_le_bytes)
}

fn print_bme280_compensated(
    temperature_label: &[u8],
    pressure_label: &[u8],
    humidity_label: &[u8],
    values: Bme280Compensated,
) {
    print_reg(temperature_label, values.temperature_centi_c as u32);
    print_reg(pressure_label, values.pressure_q8_pa);
    print_reg(humidity_label, values.humidity_q10_pct);
}

/// Begin a write-address transaction and select a device register pointer.
unsafe fn begin_i2c_write(address: u8, pointer: u8) -> Option<()> {
    // Clear any prior address-failure flag; error flags are rc_w0.
    write_volatile(I2C1_SR1 as *mut u32, !(1 << 10));
    rmw32(I2C1_CR1, 1 << 8); // START
    if !wait_sr1(0x0001) {
        return None;
    }
    write_volatile(I2C1_DR as *mut u32, u32::from(address << 1));
    if !wait_sr1(0x0002) {
        return None;
    }
    clear_addr();
    write_volatile(I2C1_DR as *mut u32, u32::from(pointer));
    if !wait_sr1(0x0004) {
        return None;
    }
    Some(())
}

unsafe fn clear_addr() {
    // STM32F1 clears ADDR only after the SR1-then-SR2 read sequence.
    let _ = read_volatile(I2C1_SR1 as *const u32);
    let _ = read_volatile(I2C1_SR2 as *const u32);
}

// ── State printer ─────────────────────────────────────────────────────────────

fn print_state(tag: &[u8]) {
    putline(tag);
    print_reg(b"CR1=", unsafe { read_volatile(I2C1_CR1 as *const u32) });
    print_reg(b"CR2=", unsafe { read_volatile(I2C1_CR2 as *const u32) });
    print_reg(b"CCR=", unsafe { read_volatile(I2C1_CCR as *const u32) });
    print_reg(b"TRISE=", unsafe {
        read_volatile(I2C1_TRISE as *const u32)
    });
    print_reg(b"OAR1=", unsafe { read_volatile(I2C1_OAR1 as *const u32) });
    print_reg(b"SR1=", unsafe { read_volatile(I2C1_SR1 as *const u32) });
    print_reg(b"SR2=", unsafe { read_volatile(I2C1_SR2 as *const u32) });
}

fn print_reg(label: &[u8], v: u32) {
    putbytes(label);
    let mut buf = [0u8; 8];
    for i in 0..8 {
        let nib = ((v >> ((7 - i) * 4)) & 0xF) as u8;
        buf[i] = if nib < 10 {
            b'0' + nib
        } else {
            b'A' + nib - 10
        };
    }
    putbytes(&buf);
    putbyte(b'\n');
}

fn putline(s: &[u8]) {
    putbytes(s);
    putbyte(b'\n');
}

fn putbytes(s: &[u8]) {
    for &b in s {
        putbyte(b);
    }
}

fn putbyte(b: u8) {
    unsafe {
        // Wait for USART2.SR.TXE.
        for _ in 0..1_000_000 {
            let sr = read_volatile(USART2_SR as *const u32);
            if sr & (1 << 7) != 0 {
                break;
            }
        }
        write_volatile(USART2_DR as *mut u32, b as u32);
    }
}

// ── Polling helpers ───────────────────────────────────────────────────────────

fn wait_sr1(mask: u32) -> bool {
    for _ in 0..16_000 {
        let sr1 = unsafe { read_volatile(I2C1_SR1 as *const u32) };
        if sr1 & mask == mask {
            return true;
        }
    }
    false
}

unsafe fn rmw32(addr: u32, set_bits: u32) {
    let v = read_volatile(addr as *const u32);
    write_volatile(addr as *mut u32, v | set_bits);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
