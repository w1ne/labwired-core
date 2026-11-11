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
const SCB_SHPR3: u32 = 0xE000_ED20;
const ICSR_PENDSTSET: u32 = 1 << 26;
const ICSR_PENDSTCLR: u32 = 1 << 25;
const ICSR_PENDSVSET: u32 = 1 << 28;
const ICSR_PENDSVCLR: u32 = 1 << 27;
const PENDSV_VECTOR_TABLE: u32 = 0x2000_0200;
const SYSTICK_HANDLER_COUNT: *mut u32 = 0x2000_0100 as *mut u32;
const SYSTICK_HANDLER_IPSR: *mut u32 = 0x2000_0104 as *mut u32;
const SYSTICK_HANDLER_REPEND: *mut u32 = 0x2000_0108 as *mut u32;
const SYSTICK_HANDLER_ICSR_FIRST: *mut u32 = 0x2000_010C as *mut u32;
const SYSTICK_HANDLER_ICSR_SECOND: *mut u32 = 0x2000_0110 as *mut u32;
const PENDSV_HANDLER_COUNT: *mut u32 = 0x2000_0114 as *mut u32;
const PENDSV_HANDLER_IPSR: *mut u32 = 0x2000_0118 as *mut u32;
const SYSTICK_HANDLER_PENDSV_REQUEST: *mut u32 = 0x2000_0124 as *mut u32;
const SYSTICK_HANDLER_PENDSV_DEFERRED: *mut u32 = 0x2000_0128 as *mut u32;
const PENDSV_HANDLER_SYSTICK_COUNT: *mut u32 = 0x2000_012C as *mut u32;
const IRQ_FRAME_VECTOR_TABLE: u32 = 0x2000_0400;
const IRQ_FRAME_RECORD: u32 = 0x2000_0500;
const IRQ_FRAME_EXTERNAL: u32 = 38;
const IRQ_FRAME_BIT: u32 = 1 << (IRQ_FRAME_EXTERNAL % 32);
const IRQ_FRAME_PSP_TOP: u32 = 0x2000_4000;
const BASEPRI_IRQ_LOW: u32 = 38;
const BASEPRI_IRQ_HIGH: u32 = 39;
const BASEPRI_IRQ_LOW_BIT: u32 = 1 << (BASEPRI_IRQ_LOW % 32);
const BASEPRI_IRQ_HIGH_BIT: u32 = 1 << (BASEPRI_IRQ_HIGH % 32);
const BASEPRI_IRQ_MASK: u32 = BASEPRI_IRQ_LOW_BIT | BASEPRI_IRQ_HIGH_BIT;
const BASEPRI_RECORD: u32 = 0x2000_0600;
const NVIC_PRIORITY_VECTOR_TABLE: u32 = 0x2000_0800;
const NVIC_PRIORITY_RECORD: u32 = 0x2000_0A00;
const FAULT_RECORD: u32 = 0x2000_0680;
const SVC_RECORD: u32 = 0x2000_0B00;
const SVC_EXCEPTION: usize = 11;
const SCB_SHCSR: u32 = 0xE000_ED24;
const SCB_CFSR: u32 = 0xE000_ED28;
const SCB_HFSR: u32 = 0xE000_ED2C;
const SHCSR_USGFAULTENA: u32 = 1 << 18;
const CFSR_UNDEFINSTR: u32 = 1 << 16;
const HFSR_FORCED: u32 = 1 << 30;

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
    // The SysTick handler reads this gate in several earlier checks, before
    // the PendSV scheduling check itself initializes the rest of its record.
    unsafe { write_volatile(SYSTICK_HANDLER_PENDSV_REQUEST, 0) };
    unsafe {
        rcc_init();
        usart2_init();
    }
    putline(b"F103 I2C");
    unsafe { smoke_systick_exception_state() };
    unsafe { smoke_systick_external_pause_resume() };
    unsafe { smoke_systick_handler_delivery() };
    unsafe { smoke_systick_wfi_wake() };
    unsafe { smoke_systick_icsr_repend() };
    unsafe { smoke_systick_tickint_gate() };
    unsafe { smoke_systick_cvr_write_preserves_pending() };
    unsafe { smoke_pendsv_clear_and_repend() };
    unsafe { smoke_pendsv_waits_for_systick_return() };
    unsafe { smoke_external_irq_stack_frame() };
    unsafe { smoke_external_irq_psp_stack_frame() };
    unsafe { smoke_svcall_exception_return() };
    unsafe { smoke_usagefault_and_hardfault_escalation() };
    unsafe { smoke_primask_external_irq() };
    unsafe { smoke_basepri_priority_mask() };
    unsafe { smoke_nvic_subpriority_order_and_preemption() };
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

/// Verify that a SysTick exception wakes the Cortex-M3 from WFI and that the
/// first instruction after WFI runs after the handler returns.
unsafe fn smoke_systick_wfi_wake() -> bool {
    const WFI_RESUMED: *mut u32 = 0x2000_0120 as *mut u32;

    let prior_primask: u32;
    core::arch::asm!(
        "mrs {primask}, PRIMASK",
        primask = out(reg) prior_primask,
        options(nomem, nostack, preserves_flags),
    );

    write_volatile(SYSTICK_CSR as *mut u32, 0);
    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR);
    write_volatile(SCB_VTOR as *mut u32, 0x0800_0000);
    write_volatile(SYSTICK_HANDLER_COUNT, 0);
    write_volatile(SYSTICK_HANDLER_IPSR, 0);
    write_volatile(SYSTICK_HANDLER_REPEND, 0);
    write_volatile(WFI_RESUMED, 0);
    write_volatile(SYSTICK_RVR as *mut u32, 127);
    write_volatile(SYSTICK_CVR as *mut u32, 0);
    write_volatile(SYSTICK_CSR as *mut u32, 0x7); // ENABLE | TICKINT | CLKSOURCE

    core::arch::asm!("wfi", options(nomem, nostack, preserves_flags));

    // Keep the first fall-through work short. If WFI were a NOP, the timer
    // would not reach its reload before firmware disables it here.
    write_volatile(WFI_RESUMED, 1);
    write_volatile(SYSTICK_CSR as *mut u32, 0);
    let resumed = read_volatile(WFI_RESUMED) == 1;
    let deliveries = read_volatile(SYSTICK_HANDLER_COUNT);
    let ipsr = read_volatile(SYSTICK_HANDLER_IPSR);
    let pending_after_return = read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSTSET;
    let countflag = read_volatile(SYSTICK_CSR as *const u32) & SYSTICK_COUNTFLAG != 0;
    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR);
    write_volatile(SYSTICK_CVR as *mut u32, 0);

    let ok = prior_primask == 0
        && resumed
        && deliveries == 1
        && ipsr == 15
        && pending_after_return == 0
        && countflag;
    if ok {
        putline(b"SYSTICK M3 WFI_WAKE=1 VECTOR=15 ENTRIES=1 RESUMED=1 ok");
    } else {
        putline(b"SYSTICK M3 WFI WAKE CHECK FAILED");
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

/// TICKINT gates exception requests but not the counter or COUNTFLAG. Keep
/// PRIMASK set so both the absent and present pending states can be observed
/// without entering the handler.
unsafe fn smoke_systick_tickint_gate() -> bool {
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
    write_volatile(SYSTICK_RVR as *mut u32, 63);
    write_volatile(SYSTICK_CVR as *mut u32, 0);
    write_volatile(SYSTICK_CSR as *mut u32, 0x5); // ENABLE | CLKSOURCE; TICKINT clear.

    let mut wrapped_without_tickint = false;
    for _ in 0..4_096 {
        if read_volatile(SYSTICK_CSR as *const u32) & SYSTICK_COUNTFLAG != 0 {
            wrapped_without_tickint = true;
            break;
        }
    }
    let no_request_without_tickint = read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSTSET == 0
        && read_volatile(SYSTICK_HANDLER_COUNT) == 0;

    // The counter remains enabled; the next zero hit now asserts SysTick.
    write_volatile(SYSTICK_CSR as *mut u32, 0x7);
    let mut request_with_tickint = false;
    for _ in 0..4_096 {
        if read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSTSET != 0 {
            request_with_tickint = true;
            break;
        }
    }
    let handler_masked = read_volatile(SYSTICK_HANDLER_COUNT) == 0;

    write_volatile(SYSTICK_CSR as *mut u32, 0);
    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR);
    write_volatile(SCB_VTOR as *mut u32, source_vtor);
    core::arch::asm!(
        "msr PRIMASK, {primask}",
        "dsb",
        "isb",
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );

    let ok = prior_primask == 0
        && source_vtor == 0x0800_0000
        && wrapped_without_tickint
        && no_request_without_tickint
        && request_with_tickint
        && handler_masked;
    if ok {
        putline(b"SYSTICK M3 TICKINT=0 COUNTFLAG=1 NO_PEND=1 TICKINT=1 PEND=1 MASKED=1 ok");
    } else {
        putline(b"SYSTICK M3 TICKINT CHECK FAILED");
    }
    ok
}

/// A CVR write clears the SysTick counter and COUNTFLAG, but leaves a wrap's
/// already-latched SCB pending request intact.
unsafe fn smoke_systick_cvr_write_preserves_pending() -> bool {
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
    write_volatile(SYSTICK_RVR as *mut u32, 63);
    write_volatile(SYSTICK_CVR as *mut u32, 0);
    write_volatile(SYSTICK_CSR as *mut u32, 0x7); // ENABLE | TICKINT | CLKSOURCE

    let mut wrapped = false;
    for _ in 0..4_096 {
        if read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSTSET != 0 {
            wrapped = true;
            break;
        }
    }

    // Do not read CSR before this write: that would consume COUNTFLAG. The
    // pending request proves a zero-hit occurred, and must survive the CVR
    // write while the counter value and unconsumed COUNTFLAG clear.
    write_volatile(SYSTICK_CSR as *mut u32, 0);
    let pending_before_cvr = read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSTSET != 0;
    write_volatile(SYSTICK_CVR as *mut u32, 0);
    let counter_zero = read_volatile(SYSTICK_CVR as *const u32) == 0;
    let countflag_cleared = read_volatile(SYSTICK_CSR as *const u32) & SYSTICK_COUNTFLAG == 0;
    let pending_preserved = read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSTSET != 0;
    let handler_masked = read_volatile(SYSTICK_HANDLER_COUNT) == 0;

    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR);
    let pending_cleared = read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSTSET == 0;
    write_volatile(SCB_VTOR as *mut u32, source_vtor);
    core::arch::asm!(
        "msr PRIMASK, {primask}",
        "dsb",
        "isb",
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );

    let ok = prior_primask == 0
        && source_vtor == 0x0800_0000
        && wrapped
        && pending_before_cvr
        && counter_zero
        && countflag_cleared
        && pending_preserved
        && pending_cleared
        && handler_masked;
    if ok {
        putline(
            b"SYSTICK M3 CVR_WRITE WRAP=1 ZERO=1 COUNTFLAG_CLEAR=1 PENDING_PRESERVED=1 MASKED=1 ok",
        );
    } else {
        putline(b"SYSTICK M3 CVR_WRITE CHECK FAILED");
    }
    ok
}

/// Confirm that Cortex-M3 ICSR continues to expose a PendSV request after the
/// SCB hands it to the CPU pending bitmap, and that clear followed by a fresh
/// set still dispatches vector 14 exactly once.
unsafe fn smoke_pendsv_clear_and_repend() -> bool {
    let source_vtor = read_volatile(SCB_VTOR as *const u32);
    let prior_shpr3 = read_volatile(SCB_SHPR3 as *const u32);
    let prior_primask: u32;
    core::arch::asm!(
        "mrs {primask}, PRIMASK",
        "cpsid i",
        primask = out(reg) prior_primask,
        options(nomem, nostack, preserves_flags),
    );

    write_volatile(SYSTICK_CSR as *mut u32, 0);
    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
    for slot in 0..16 {
        write_volatile((PENDSV_VECTOR_TABLE + slot * 4) as *mut u32, 0);
    }
    write_volatile(
        (PENDSV_VECTOR_TABLE + 14 * 4) as *mut u32,
        PendSV_Handler as *const () as usize as u32,
    );
    write_volatile(SCB_VTOR as *mut u32, PENDSV_VECTOR_TABLE);
    write_volatile(PENDSV_HANDLER_COUNT, 0);
    write_volatile(PENDSV_HANDLER_IPSR, 0);

    // A masked request must remain visible until explicitly cancelled.
    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSVSET);
    for _ in 0..16 {
        core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
    }
    let pending_before_clear = read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSVSET != 0;
    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSVCLR);
    let pending_after_clear = read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSVSET == 0;
    core::arch::asm!(
        "msr PRIMASK, {primask}",
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );
    for _ in 0..32 {
        core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
    }
    let clear_cancels_pending = read_volatile(PENDSV_HANDLER_COUNT) == 0;

    // Re-pending after the clear must be visible and enter the installed
    // handler once after interrupts are restored.
    core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSVSET);
    let fresh_pending_visible = read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSVSET != 0;
    core::arch::asm!(
        "msr PRIMASK, {primask}",
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );
    for _ in 0..4_096 {
        if read_volatile(PENDSV_HANDLER_COUNT) != 0 {
            break;
        }
        core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
    }
    for _ in 0..32 {
        core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
    }

    let entries = read_volatile(PENDSV_HANDLER_COUNT);
    let ipsr = read_volatile(PENDSV_HANDLER_IPSR);
    let vector_installed = read_volatile(SCB_VTOR as *const u32) == PENDSV_VECTOR_TABLE;
    let passed = prior_primask == 0
        && source_vtor == 0x0800_0000
        && pending_before_clear
        && pending_after_clear
        && clear_cancels_pending
        && fresh_pending_visible
        && entries == 1
        && ipsr == 14
        && vector_installed;

    core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSVCLR | ICSR_PENDSTCLR);
    write_volatile(SCB_SHPR3 as *mut u32, prior_shpr3);
    write_volatile(SCB_VTOR as *mut u32, source_vtor);
    core::arch::asm!(
        "msr PRIMASK, {primask}",
        "dsb",
        "isb",
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );

    if passed {
        putline(b"PENDSV M3 CLEAR=1 REPEND=1 VECTOR=14 IPSR=14 ok");
    } else {
        putline(b"PENDSV M3 CHECK FAILED");
    }
    passed
}

/// Verify the Cortex-M RTOS scheduling pattern: a higher-priority SysTick
/// handler pends low-priority PendSV, which must wait until SysTick returns.
unsafe fn smoke_pendsv_waits_for_systick_return() -> bool {
    let source_vtor = read_volatile(SCB_VTOR as *const u32);
    let prior_shpr3 = read_volatile(SCB_SHPR3 as *const u32);
    let prior_rvr = read_volatile(SYSTICK_RVR as *const u32);
    let prior_primask: u32;
    core::arch::asm!(
        "mrs {primask}, PRIMASK",
        "cpsid i",
        primask = out(reg) prior_primask,
        options(nomem, nostack, preserves_flags),
    );

    write_volatile(SYSTICK_CSR as *mut u32, 0);
    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
    for slot in 0..16 {
        write_volatile((PENDSV_VECTOR_TABLE + slot * 4) as *mut u32, 0);
    }
    write_volatile(
        (PENDSV_VECTOR_TABLE + 14 * 4) as *mut u32,
        PendSV_Handler as *const () as usize as u32,
    );
    write_volatile(
        (PENDSV_VECTOR_TABLE + 15 * 4) as *mut u32,
        SysTick_Handler as *const () as usize as u32,
    );
    write_volatile(SCB_VTOR as *mut u32, PENDSV_VECTOR_TABLE);
    write_volatile(
        SCB_SHPR3 as *mut u32,
        (prior_shpr3 & 0x0000_FFFF) | 0x00FF_0000,
    );
    write_volatile(SYSTICK_HANDLER_COUNT, 0);
    write_volatile(SYSTICK_HANDLER_IPSR, 0);
    write_volatile(PENDSV_HANDLER_COUNT, 0);
    write_volatile(PENDSV_HANDLER_IPSR, 0);
    write_volatile(SYSTICK_HANDLER_PENDSV_REQUEST, 1);
    write_volatile(SYSTICK_HANDLER_PENDSV_DEFERRED, 0);
    write_volatile(PENDSV_HANDLER_SYSTICK_COUNT, 0);
    write_volatile(SYSTICK_RVR as *mut u32, 31);
    write_volatile(SYSTICK_CVR as *mut u32, 0);
    write_volatile(SYSTICK_CSR as *mut u32, 0x7); // ENABLE | TICKINT | CLKSOURCE

    core::arch::asm!(
        "msr PRIMASK, {primask}",
        "dsb",
        "isb",
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );
    for _ in 0..4_096 {
        if read_volatile(PENDSV_HANDLER_COUNT) != 0 {
            break;
        }
        core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
    }

    core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
    write_volatile(SYSTICK_CSR as *mut u32, 0);
    write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSTCLR | ICSR_PENDSVCLR);
    let systick_entries = read_volatile(SYSTICK_HANDLER_COUNT);
    let systick_ipsr = read_volatile(SYSTICK_HANDLER_IPSR);
    let deferred_while_active = read_volatile(SYSTICK_HANDLER_PENDSV_DEFERRED);
    let pendsv_entries = read_volatile(PENDSV_HANDLER_COUNT);
    let pendsv_ipsr = read_volatile(PENDSV_HANDLER_IPSR);
    let systick_count_at_pendsv = read_volatile(PENDSV_HANDLER_SYSTICK_COUNT);
    let vector_installed = read_volatile(SCB_VTOR as *const u32) == PENDSV_VECTOR_TABLE;

    write_volatile(SCB_SHPR3 as *mut u32, prior_shpr3);
    write_volatile(SCB_VTOR as *mut u32, source_vtor);
    write_volatile(SYSTICK_RVR as *mut u32, prior_rvr);
    write_volatile(SYSTICK_HANDLER_PENDSV_REQUEST, 0);
    core::arch::asm!(
        "msr PRIMASK, {primask}",
        "dsb",
        "isb",
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );

    let passed = prior_primask == 0
        && systick_entries == 1
        && systick_ipsr == 15
        && deferred_while_active == 1
        && pendsv_entries == 1
        && pendsv_ipsr == 14
        && systick_count_at_pendsv == 1
        && vector_installed;
    if passed {
        putline(b"PENDSV M3 DEFERRED=1 AFTER_SYSTICK=1 VECTOR=14 IPSR=14 ok");
    } else {
        putline(b"PENDSV M3 SYSTICK ORDER CHECK FAILED");
    }
    passed
}

/// Take USART2's external vector with known caller-saved registers and inspect
/// the architectural Cortex-M3 frame before a Rust prologue can move MSP.
unsafe fn smoke_external_irq_stack_frame() -> bool {
    const IRQ_FRAME_VECTOR_INDEX: usize = 16 + IRQ_FRAME_EXTERNAL as usize;
    const IRQ_FRAME_VECTOR_WORDS: usize = IRQ_FRAME_VECTOR_INDEX + 1;
    const NVIC_ISER1: u32 = 0xE000_E104;
    const NVIC_ICER1: u32 = 0xE000_E184;
    const NVIC_ISPR1: u32 = 0xE000_E204;
    const NVIC_ICPR1: u32 = 0xE000_E284;

    let source_vtor = read_volatile(SCB_VTOR as *const u32);
    let prior_primask: u32;
    core::arch::asm!(
        "mrs {primask}, PRIMASK",
        "cpsid i",
        primask = out(reg) prior_primask,
        options(nomem, nostack, preserves_flags),
    );

    write_volatile(NVIC_ICER1 as *mut u32, IRQ_FRAME_BIT);
    write_volatile(NVIC_ICPR1 as *mut u32, IRQ_FRAME_BIT);
    let table = IRQ_FRAME_VECTOR_TABLE as *mut u32;
    for slot in 0..IRQ_FRAME_VECTOR_WORDS {
        write_volatile(table.add(slot), 0);
    }
    write_volatile(
        table.add(IRQ_FRAME_VECTOR_INDEX),
        (smoke_external_irq_frame_handler as *const () as usize as u32) | 1,
    );
    for word in 0..18 {
        write_volatile((IRQ_FRAME_RECORD + word * 4) as *mut u32, 0);
    }
    write_volatile(SCB_VTOR as *mut u32, IRQ_FRAME_VECTOR_TABLE);
    core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));

    let msp_before: u32;
    core::arch::asm!("mrs {}, msp", out(reg) msp_before, options(nomem, nostack, preserves_flags));
    write_volatile((IRQ_FRAME_RECORD + 52) as *mut u32, msp_before);
    let vector_installed = read_volatile(SCB_VTOR as *const u32) == IRQ_FRAME_VECTOR_TABLE;
    write_volatile(NVIC_ISER1 as *mut u32, IRQ_FRAME_BIT);
    core::arch::asm!(
        "msr PRIMASK, {primask}",
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );
    smoke_pend_usart2_irq_with_sentinels();
    let msp_after: u32;
    core::arch::asm!("mrs {}, msp", out(reg) msp_after, options(nomem, nostack, preserves_flags));
    core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));

    let frame_sp = read_volatile(IRQ_FRAME_RECORD as *const u32);
    let frame_r0 = read_volatile((IRQ_FRAME_RECORD + 4) as *const u32);
    let frame_r1 = read_volatile((IRQ_FRAME_RECORD + 8) as *const u32);
    let frame_r2 = read_volatile((IRQ_FRAME_RECORD + 12) as *const u32);
    let frame_r3 = read_volatile((IRQ_FRAME_RECORD + 16) as *const u32);
    let frame_r12 = read_volatile((IRQ_FRAME_RECORD + 20) as *const u32);
    let frame_lr = read_volatile((IRQ_FRAME_RECORD + 24) as *const u32);
    let frame_pc = read_volatile((IRQ_FRAME_RECORD + 28) as *const u32);
    let frame_xpsr = read_volatile((IRQ_FRAME_RECORD + 32) as *const u32);
    let exc_return = read_volatile((IRQ_FRAME_RECORD + 36) as *const u32);
    let ipsr = read_volatile((IRQ_FRAME_RECORD + 40) as *const u32);
    let active = read_volatile((IRQ_FRAME_RECORD + 44) as *const u32);
    let pending = read_volatile((IRQ_FRAME_RECORD + 48) as *const u32);
    let msp_before_helper = read_volatile((IRQ_FRAME_RECORD + 52) as *const u32);
    let exception_msp = read_volatile((IRQ_FRAME_RECORD + 60) as *const u32);
    let handled = read_volatile((IRQ_FRAME_RECORD + 56) as *const u32) == 1;

    write_volatile(NVIC_ICER1 as *mut u32, IRQ_FRAME_BIT);
    write_volatile(NVIC_ICPR1 as *mut u32, IRQ_FRAME_BIT);
    write_volatile(SCB_VTOR as *mut u32, source_vtor);
    core::arch::asm!(
        "msr PRIMASK, {primask}",
        "dsb",
        "isb",
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );

    let frame_ok = frame_r0 == 0x11
        && frame_r1 == 0x22
        && frame_r2 == 0x33
        && frame_r3 == 0x44
        && frame_r12 == 0x55
        && frame_lr & 1 != 0
        && (0x0800_0000..0x0802_0000).contains(&frame_pc)
        && frame_xpsr & 0x0100_0000 != 0
        && frame_xpsr & 0x0000_03ff == 0
        && exc_return == 0xffff_fff9
        && msp_before_helper == msp_before
        && exception_msp == msp_before_helper.wrapping_sub(8)
        && frame_sp == exception_msp.wrapping_sub(32)
        && frame_sp & 7 == 0
        && msp_after == msp_before;
    let irq_state_ok = ipsr == 16 + IRQ_FRAME_EXTERNAL
        && active == IRQ_FRAME_BIT
        && pending == 0
        && read_volatile(NVIC_ISPR1 as *const u32) & IRQ_FRAME_BIT == 0;
    let restored = read_volatile(SCB_VTOR as *const u32) == source_vtor
        && read_volatile(NVIC_ICPR1 as *const u32) & IRQ_FRAME_BIT == 0;
    let ok =
        prior_primask == 0 && vector_installed && handled && frame_ok && irq_state_ok && restored;

    if ok {
        putline(b"IRQ M3 FRAME IPSR=54 ACTIVE=1 PENDING=0 SENTINELS=1 EXC_RETURN=FFFFFFF9 STACK=ALIGNED RETURN=1 ok");
    } else {
        putline(b"IRQ M3 FRAME CHECK FAILED");
    }
    ok
}

/// Repeat the external IRQ frame probe with Thread mode using PSP. The handler
/// must stack the interrupted state on PSP, keep MSP for Handler mode, and
/// return to the same PSP address before the trampoline switches back to MSP.
unsafe fn smoke_external_irq_psp_stack_frame() -> bool {
    const IRQ_FRAME_VECTOR_INDEX: usize = 16 + IRQ_FRAME_EXTERNAL as usize;
    const IRQ_FRAME_VECTOR_WORDS: usize = IRQ_FRAME_VECTOR_INDEX + 1;
    const NVIC_ISER1: u32 = 0xE000_E104;
    const NVIC_ICER1: u32 = 0xE000_E184;
    const NVIC_ISPR1: u32 = 0xE000_E204;
    const NVIC_ICPR1: u32 = 0xE000_E284;

    let source_vtor = read_volatile(SCB_VTOR as *const u32);
    let source_control: u32;
    let source_psp: u32;
    let prior_primask: u32;
    core::arch::asm!(
        "mrs {control}, CONTROL",
        "mrs {psp}, PSP",
        "mrs {primask}, PRIMASK",
        "cpsid i",
        control = out(reg) source_control,
        psp = out(reg) source_psp,
        primask = out(reg) prior_primask,
        options(nomem, nostack, preserves_flags),
    );

    write_volatile(NVIC_ICER1 as *mut u32, IRQ_FRAME_BIT);
    write_volatile(NVIC_ICPR1 as *mut u32, IRQ_FRAME_BIT);
    let table = IRQ_FRAME_VECTOR_TABLE as *mut u32;
    for slot in 0..IRQ_FRAME_VECTOR_WORDS {
        write_volatile(table.add(slot), 0);
    }
    write_volatile(
        table.add(IRQ_FRAME_VECTOR_INDEX),
        (smoke_external_irq_frame_handler as *const () as usize as u32) | 1,
    );
    for word in 0..18 {
        write_volatile((IRQ_FRAME_RECORD + word * 4) as *mut u32, 0);
    }
    write_volatile(SCB_VTOR as *mut u32, IRQ_FRAME_VECTOR_TABLE);
    core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));

    let vector_installed = read_volatile(SCB_VTOR as *const u32) == IRQ_FRAME_VECTOR_TABLE;
    write_volatile(NVIC_ISER1 as *mut u32, IRQ_FRAME_BIT);
    core::arch::asm!(
        "msr PRIMASK, {primask}",
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );
    let msp_before: u32;
    core::arch::asm!("mrs {}, MSP", out(reg) msp_before, options(nomem, nostack, preserves_flags));
    smoke_pend_usart2_irq_with_psp_sentinels();
    let msp_after: u32;
    let psp_after: u32;
    let control_after: u32;
    core::arch::asm!(
        "mrs {msp}, MSP",
        "mrs {psp}, PSP",
        "mrs {control}, CONTROL",
        msp = out(reg) msp_after,
        psp = out(reg) psp_after,
        control = out(reg) control_after,
        options(nomem, nostack, preserves_flags),
    );
    core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));

    let frame_sp = read_volatile(IRQ_FRAME_RECORD as *const u32);
    let frame_r0 = read_volatile((IRQ_FRAME_RECORD + 4) as *const u32);
    let frame_r1 = read_volatile((IRQ_FRAME_RECORD + 8) as *const u32);
    let frame_r2 = read_volatile((IRQ_FRAME_RECORD + 12) as *const u32);
    let frame_r3 = read_volatile((IRQ_FRAME_RECORD + 16) as *const u32);
    let frame_r12 = read_volatile((IRQ_FRAME_RECORD + 20) as *const u32);
    let frame_lr = read_volatile((IRQ_FRAME_RECORD + 24) as *const u32);
    let frame_pc = read_volatile((IRQ_FRAME_RECORD + 28) as *const u32);
    let frame_xpsr = read_volatile((IRQ_FRAME_RECORD + 32) as *const u32);
    let exc_return = read_volatile((IRQ_FRAME_RECORD + 36) as *const u32);
    let ipsr = read_volatile((IRQ_FRAME_RECORD + 40) as *const u32);
    let active = read_volatile((IRQ_FRAME_RECORD + 44) as *const u32);
    let pending = read_volatile((IRQ_FRAME_RECORD + 48) as *const u32);
    let handler_msp = read_volatile((IRQ_FRAME_RECORD + 52) as *const u32);
    let returned = read_volatile((IRQ_FRAME_RECORD + 56) as *const u32) == 1;
    let exception_msp = read_volatile((IRQ_FRAME_RECORD + 68) as *const u32);
    let return_marker = read_volatile((IRQ_FRAME_RECORD + 64) as *const u32) == 1;

    write_volatile(NVIC_ICER1 as *mut u32, IRQ_FRAME_BIT);
    write_volatile(NVIC_ICPR1 as *mut u32, IRQ_FRAME_BIT);
    write_volatile(SCB_VTOR as *mut u32, source_vtor);
    core::arch::asm!(
        "msr PSP, {psp}",
        "msr CONTROL, {control}",
        "msr PRIMASK, {primask}",
        "dsb",
        "isb",
        psp = in(reg) source_psp,
        control = in(reg) source_control,
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );
    let restored_psp: u32;
    let restored_control: u32;
    core::arch::asm!(
        "mrs {psp}, PSP",
        "mrs {control}, CONTROL",
        psp = out(reg) restored_psp,
        control = out(reg) restored_control,
        options(nomem, nostack, preserves_flags),
    );

    let frame_ok = frame_r0 == 0x11
        && frame_r1 == 0x22
        && frame_r2 == 0x33
        && frame_r3 == 0x44
        && frame_r12 == 0x55
        && frame_lr & 1 != 0
        && (0x0800_0000..0x0802_0000).contains(&frame_pc)
        && frame_xpsr & 0x0100_0000 != 0
        && frame_xpsr & 0x0000_03ff == 0
        && exc_return == 0xffff_fffd
        && frame_sp == IRQ_FRAME_PSP_TOP.wrapping_sub(32)
        && frame_sp & 7 == 0
        && handler_msp == exception_msp
        && handler_msp & 7 == 0
        && msp_after == msp_before
        && psp_after == IRQ_FRAME_PSP_TOP
        && control_after & 2 == 0;
    let irq_state_ok = ipsr == 16 + IRQ_FRAME_EXTERNAL
        && active == IRQ_FRAME_BIT
        && pending == 0
        && read_volatile(NVIC_ISPR1 as *const u32) & IRQ_FRAME_BIT == 0;
    let restored = read_volatile(SCB_VTOR as *const u32) == source_vtor
        && read_volatile(NVIC_ICPR1 as *const u32) & IRQ_FRAME_BIT == 0
        && restored_psp == source_psp
        && restored_control == source_control;
    let ok = prior_primask == 0
        && source_control & 2 == 0
        && vector_installed
        && returned
        && return_marker
        && frame_ok
        && irq_state_ok
        && restored;

    if ok {
        putline(b"IRQ M3 PSP FRAME IPSR=54 ACTIVE=1 PENDING=0 SENTINELS=1 EXC_RETURN=FFFFFFFD PSP=RESTORED MSP=UNCHANGED ok");
    } else {
        putline(b"IRQ M3 PSP FRAME CHECK FAILED");
    }
    ok
}

/// Exercise SVC exception entry and return on the imported Cortex-M3 profile.
/// The handler checks exception 11, EXC_RETURN, the stacked PC and SVC opcode.
unsafe fn smoke_svcall_exception_return() -> bool {
    let source_vtor = read_volatile(SCB_VTOR as *const u32);
    let vector_table = IRQ_FRAME_VECTOR_TABLE as *mut u32;
    let prior_vector = read_volatile(vector_table.add(SVC_EXCEPTION));
    let source_control: u32;
    let source_psp: u32;
    let prior_primask: u32;
    let prior_basepri: u32;
    let prior_faultmask: u32;
    core::arch::asm!(
        "mrs {control}, CONTROL",
        "mrs {psp}, PSP",
        "mrs {primask}, PRIMASK",
        "mrs {basepri}, BASEPRI",
        "mrs {faultmask}, FAULTMASK",
        "cpsid i",
        control = out(reg) source_control,
        psp = out(reg) source_psp,
        primask = out(reg) prior_primask,
        basepri = out(reg) prior_basepri,
        faultmask = out(reg) prior_faultmask,
        options(nomem, nostack, preserves_flags),
    );

    let record = SVC_RECORD as *mut u32;
    for word in 0..10 {
        write_volatile(record.add(word), 0);
    }
    write_volatile(
        vector_table.add(SVC_EXCEPTION),
        (smoke_svcall_handler as *const () as usize as u32) | 1,
    );
    write_volatile(SCB_VTOR as *mut u32, IRQ_FRAME_VECTOR_TABLE);
    core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));
    core::arch::asm!(
        "msr BASEPRI, {zero}",
        "msr FAULTMASK, {zero}",
        "msr PRIMASK, {zero}",
        zero = in(reg) 0_u32,
        options(nostack, preserves_flags),
    );

    core::arch::asm!("svc #0x42", options(nostack, preserves_flags),);
    write_volatile(record.add(8), 1);
    core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));

    let entries = read_volatile(record.add(0));
    let ipsr = read_volatile(record.add(1));
    let exc_return = read_volatile(record.add(2));
    let frame_sp = read_volatile(record.add(3));
    let stacked_lr = read_volatile(record.add(4));
    let stacked_pc = read_volatile(record.add(5));
    let stacked_xpsr = read_volatile(record.add(6));
    let svc_opcode = read_volatile(record.add(7));
    let resumed = read_volatile(record.add(8)) == 1;

    // Repeat SVC from Thread mode using PSP. Handler mode must use MSP while
    // EXC_RETURN selects PSP for unstacking back to Thread mode.
    for word in 0..19 {
        write_volatile(record.add(word), 0);
    }
    let msp_before_psp: u32;
    core::arch::asm!(
        "mrs {msp}, MSP",
        msp = out(reg) msp_before_psp,
        options(nomem, nostack, preserves_flags),
    );
    let unprivileged_control = source_control | 0x3;
    let attempt_privileged_control = source_control | 0x2;
    let control_psp = unprivileged_control;
    core::arch::asm!(
        "msr BASEPRI, {zero}",
        "msr FAULTMASK, {zero}",
        "msr PRIMASK, {zero}",
        "isb",
        zero = in(reg) 0_u32,
        options(nostack, preserves_flags),
    );
    core::arch::asm!(
        "msr PSP, {psp_top}",
        "msr CONTROL, {unprivileged_control}",
        "isb",
        // Unprivileged Thread mode cannot raise BASEPRI or clear nPRIV.
        "msr BASEPRI, {attempt_basepri}",
        "mrs {scratch}, BASEPRI",
        "str {scratch}, [{record}, #48]",
        "msr CONTROL, {attempt_privileged_control}",
        "isb",
        "mrs {scratch}, CONTROL",
        "str {scratch}, [{record}, #52]",
        // The first SVC tests privileged handler access and returns to the
        // same unprivileged PSP thread. The second SVC returns to privileged
        // Thread mode so this smoke can safely restore its original stack.
        "str {first_svc}, [{record}, #40]",
        "svc #0x42",
        "str {resume_marker}, [{record}, #32]",
        "mrs {scratch}, CONTROL",
        "str {scratch}, [{record}, #56]",
        "mrs {scratch}, BASEPRI",
        "str {scratch}, [{record}, #60]",
        "str {second_svc}, [{record}, #40]",
        "svc #0x43",
        "mrs {scratch}, CONTROL",
        "str {scratch}, [{record}, #64]",
        "mrs {scratch}, PSP",
        "str {scratch}, [{record}, #68]",
        "mrs {scratch}, MSP",
        "str {scratch}, [{record}, #72]",
        "msr PSP, {source_psp}",
        "msr CONTROL, {source_control}",
        "isb",
        psp_top = in(reg) IRQ_FRAME_PSP_TOP,
        record = in(reg) record,
        unprivileged_control = in(reg) unprivileged_control,
        attempt_privileged_control = in(reg) attempt_privileged_control,
        attempt_basepri = in(reg) 0x20_u32,
        first_svc = in(reg) 1_u32,
        second_svc = in(reg) 2_u32,
        resume_marker = in(reg) 1_u32,
        source_control = in(reg) source_control,
        source_psp = in(reg) source_psp,
        scratch = out(reg) _,
        options(nostack),
    );
    core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
    write_volatile(record.add(8), 1);
    let psp_entries = read_volatile(record.add(0));
    let psp_ipsr = read_volatile(record.add(1));
    let psp_exc_return = read_volatile(record.add(2));
    let psp_frame_sp = read_volatile(record.add(3));
    let psp_stacked_pc = read_volatile(record.add(5));
    let psp_stacked_xpsr = read_volatile(record.add(6));
    let psp_svc_opcode = read_volatile(record.add(7));
    let psp_resumed = read_volatile(record.add(8)) == 1;
    let psp_handler_msp = read_volatile(record.add(9));
    let handler_basepri = read_volatile(record.add(11));
    let unprivileged_basepri = read_volatile(record.add(12));
    let unprivileged_control_after_attempt = read_volatile(record.add(13));
    let control_after_unprivileged_svc = read_volatile(record.add(14));
    let basepri_after_unprivileged_svc = read_volatile(record.add(15));
    let control_after_privileged_svc = read_volatile(record.add(16));
    let psp_during = read_volatile(record.add(17));
    let msp_after_psp = read_volatile(record.add(18));
    let control_during = control_after_privileged_svc;
    let restored_psp: u32;
    let restored_control: u32;
    core::arch::asm!(
        "mrs {psp}, PSP",
        "mrs {control}, CONTROL",
        psp = out(reg) restored_psp,
        control = out(reg) restored_control,
        options(nomem, nostack, preserves_flags),
    );

    write_volatile(vector_table.add(SVC_EXCEPTION), prior_vector);
    write_volatile(SCB_VTOR as *mut u32, source_vtor);
    core::arch::asm!(
        "msr BASEPRI, {basepri}",
        "msr FAULTMASK, {faultmask}",
        "msr PRIMASK, {primask}",
        "dsb",
        "isb",
        basepri = in(reg) prior_basepri,
        faultmask = in(reg) prior_faultmask,
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );

    let ok = entries == 1
        && ipsr == SVC_EXCEPTION as u32
        && exc_return == 0xFFFF_FFF9
        && frame_sp & 7 == 0
        && stacked_xpsr & 0x0100_0000 != 0
        && stacked_xpsr & 0x0000_01FF == 0
        && svc_opcode == 0xDF42
        && resumed
        && read_volatile(SCB_VTOR as *const u32) == source_vtor;
    let psp_ok = psp_entries == 1
        && psp_ipsr == SVC_EXCEPTION as u32
        && psp_exc_return == 0xFFFF_FFFD
        && psp_frame_sp == IRQ_FRAME_PSP_TOP.wrapping_sub(32)
        && psp_frame_sp & 7 == 0
        && psp_stacked_xpsr & 0x0100_0000 != 0
        && psp_stacked_xpsr & 0x0000_01FF == 0
        && psp_svc_opcode == 0xDF42
        && psp_resumed
        && handler_basepri == 0x40
        && unprivileged_basepri == 0
        && unprivileged_control_after_attempt & 0x3 == 0x3
        && control_after_unprivileged_svc & 0x3 == 0x3
        && basepri_after_unprivileged_svc == 0
        && psp_during == IRQ_FRAME_PSP_TOP
        && control_during & 0x3 == 0x2
        && msp_after_psp == msp_before_psp
        && psp_handler_msp == msp_before_psp
        && restored_psp == source_psp
        && restored_control == source_control
        && source_control & 2 == 0;
    if ok && psp_ok {
        putline(
            b"SVC M3 MSP VECTOR=11 IMM=42 EXC_RETURN=FFFFFFF9 STACKED_PC=AFTER_SVC RETURN=1 ok",
        );
        putline(b"SVC M3 PSP NPRIV=1 CONTROL_IGNORED=1 BASEPRI_IGNORED=1 SVC_BASEPRI=40 RESTORED=1 RETURN_NPRIV=1 EXC_RETURN=FFFFFFFD PSP=RESTORED MSP=UNCHANGED ok");
    } else {
        putline(b"SVC M3 CHECK FAILED");
        print_reg(b"SVC entries=", entries);
        print_reg(b"SVC IPSR=", ipsr);
        print_reg(b"SVC EXC_RETURN=", exc_return);
        print_reg(b"SVC frame SP=", frame_sp);
        print_reg(b"SVC stacked LR=", stacked_lr);
        print_reg(b"SVC stacked PC=", stacked_pc);
        print_reg(b"SVC stacked xPSR=", stacked_xpsr);
        print_reg(b"SVC opcode=", svc_opcode);
        print_reg(b"SVC PSP entries=", psp_entries);
        print_reg(b"SVC PSP EXC_RETURN=", psp_exc_return);
        print_reg(b"SVC PSP frame SP=", psp_frame_sp);
        print_reg(b"SVC PSP stacked PC=", psp_stacked_pc);
        print_reg(b"SVC PSP handler MSP=", psp_handler_msp);
        print_reg(b"SVC handler BASEPRI=", handler_basepri);
        print_reg(b"SVC unprivileged BASEPRI=", unprivileged_basepri);
        print_reg(
            b"SVC unprivileged CONTROL=",
            unprivileged_control_after_attempt,
        );
        print_reg(
            b"SVC CONTROL after unprivileged return=",
            control_after_unprivileged_svc,
        );
        print_reg(
            b"SVC BASEPRI after unprivileged return=",
            basepri_after_unprivileged_svc,
        );
        print_reg(b"SVC MSP before PSP=", msp_before_psp);
        print_reg(b"SVC PSP after return=", psp_during);
        print_reg(b"SVC CONTROL during=", control_during);
        print_reg(b"SVC MSP after PSP=", msp_after_psp);
        print_reg(b"SVC PSP restored=", restored_psp);
        print_reg(b"SVC CONTROL restored=", restored_control);
        print_reg(b"SVC source CONTROL=", source_control);
        print_reg(b"SVC temporary CONTROL=", control_psp);
    }
    ok && psp_ok
}

/// Record the SVCall frame before returning through the architectural token.
#[unsafe(naked)]
#[no_mangle]
unsafe extern "C" fn smoke_svcall_handler() {
    core::arch::naked_asm!(
        "ldr r0, =0x20000B00",
        "ldr r2, [r0, #40]",
        "cmp r2, #2",
        "beq 3f",
        "ldr r2, [r0, #0]",
        "adds r2, r2, #1",
        "str r2, [r0, #0]",
        "mrs r2, ipsr",
        "str r2, [r0, #4]",
        "mov r2, lr",
        "str r2, [r0, #8]",
        "tst lr, #4",
        "beq 1f",
        "mrs r1, psp",
        "b 2f",
        "1:",
        "mrs r1, msp",
        "2:",
        "str r1, [r0, #12]",
        "ldr r2, [r1, #20]",
        "str r2, [r0, #16]",
        "ldr r2, [r1, #24]",
        "str r2, [r0, #20]",
        "ldr r2, [r1, #28]",
        "str r2, [r0, #24]",
        "ldr r3, [r1, #24]",
        "subs r3, r3, #2",
        "ldrh r3, [r3]",
        "str r3, [r0, #28]",
        "mrs r2, msp",
        "str r2, [r0, #36]",
        "ldr r2, [r0, #40]",
        "cmp r2, #1",
        "bne 3f",
        "mrs r3, BASEPRI",
        "mov r2, r3",
        "movs r3, #0x40",
        "msr BASEPRI, r3",
        "mrs r3, BASEPRI",
        "str r3, [r0, #44]",
        "msr BASEPRI, r2",
        "b 4f",
        "3:",
        // SVC #0x43 returns this smoke to privileged Thread mode on PSP.
        "movs r3, #0",
        "msr CONTROL, r3",
        "isb",
        "4:",
        "bx lr",
    );
}

/// Exercise ARMv7-M undefined-instruction routing on the imported Cortex-M3:
/// first take UsageFault with USGFAULTENA set, then require the same fault to
/// escalate to HardFault with HFSR.FORCED after UsageFault is disabled.
unsafe fn smoke_usagefault_and_hardfault_escalation() -> bool {
    const USAGEFAULT_VECTOR: usize = 6;
    const HARDFAULT_VECTOR: usize = 3;
    const VECTOR_WORDS: usize = USAGEFAULT_VECTOR + 1;
    const RECORD_WORDS: usize = 7;

    let source_vtor = read_volatile(SCB_VTOR as *const u32);
    let source_shcsr = read_volatile(SCB_SHCSR as *const u32);
    let source_cfsr = read_volatile(SCB_CFSR as *const u32);
    let source_hfsr = read_volatile(SCB_HFSR as *const u32);
    let prior_primask: u32;
    let prior_basepri: u32;
    let prior_faultmask: u32;
    core::arch::asm!(
        "mrs {primask}, PRIMASK",
        "mrs {basepri}, BASEPRI",
        "mrs {faultmask}, FAULTMASK",
        "cpsid i",
        primask = out(reg) prior_primask,
        basepri = out(reg) prior_basepri,
        faultmask = out(reg) prior_faultmask,
        options(nomem, nostack, preserves_flags),
    );

    let vector_table = IRQ_FRAME_VECTOR_TABLE as *mut u32;
    for slot in 0..VECTOR_WORDS {
        write_volatile(vector_table.add(slot), 0);
    }
    write_volatile(
        vector_table.add(HARDFAULT_VECTOR),
        (smoke_fault_handler as *const () as usize as u32) | 1,
    );
    write_volatile(
        vector_table.add(USAGEFAULT_VECTOR),
        (smoke_fault_handler as *const () as usize as u32) | 1,
    );
    let record = FAULT_RECORD as *mut u32;
    for word in 0..RECORD_WORDS {
        write_volatile(record.add(word), 0);
    }

    // The startup profile should leave these status bits clear. Record that
    // precondition so the smoke does not accidentally bless stale fault state.
    write_volatile(SCB_CFSR as *mut u32, CFSR_UNDEFINSTR);
    write_volatile(SCB_HFSR as *mut u32, HFSR_FORCED);
    write_volatile(SCB_SHCSR as *mut u32, source_shcsr | SHCSR_USGFAULTENA);
    write_volatile(SCB_VTOR as *mut u32, IRQ_FRAME_VECTOR_TABLE);
    core::arch::asm!(
        "dsb",
        "isb",
        "msr BASEPRI, {zero}",
        "msr FAULTMASK, {zero}",
        "msr PRIMASK, {zero}",
        zero = in(reg) 0_u32,
        options(nostack, preserves_flags),
    );

    smoke_trigger_undefined_instruction();
    core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
    let usage_entries = read_volatile(record.add(0));
    let usage_ipsr = read_volatile(record.add(1));
    let usage_cfsr = read_volatile(record.add(2));
    let usage_hfsr = read_volatile(record.add(3));
    let usage_shcsr = read_volatile(record.add(4));
    let usage_exc_return = read_volatile(record.add(5));
    let usage_stacked_pc = read_volatile(record.add(6));
    let usage_status_after_return = read_volatile(SCB_CFSR as *const u32);

    // Clear the first fault before repeating it with UsageFault disabled.
    write_volatile(SCB_CFSR as *mut u32, CFSR_UNDEFINSTR);
    write_volatile(SCB_HFSR as *mut u32, HFSR_FORCED);
    write_volatile(SCB_SHCSR as *mut u32, source_shcsr & !SHCSR_USGFAULTENA);
    core::arch::asm!(
        "dsb",
        "isb",
        "msr PRIMASK, {zero}",
        zero = in(reg) 0_u32,
        options(nostack, preserves_flags),
    );
    smoke_trigger_undefined_instruction();
    core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
    let hardfault_entries = read_volatile(record.add(0));
    let hardfault_ipsr = read_volatile(record.add(1));
    let hardfault_cfsr = read_volatile(record.add(2));
    let hardfault_hfsr = read_volatile(record.add(3));
    let hardfault_shcsr = read_volatile(record.add(4));
    let hardfault_exc_return = read_volatile(record.add(5));
    let hardfault_stacked_pc = read_volatile(record.add(6));
    let hardfault_status_after_return = read_volatile(SCB_HFSR as *const u32);

    write_volatile(SCB_CFSR as *mut u32, CFSR_UNDEFINSTR);
    write_volatile(SCB_HFSR as *mut u32, HFSR_FORCED);
    write_volatile(SCB_SHCSR as *mut u32, source_shcsr);
    write_volatile(SCB_VTOR as *mut u32, source_vtor);
    core::arch::asm!(
        "msr BASEPRI, {basepri}",
        "msr FAULTMASK, {faultmask}",
        "msr PRIMASK, {primask}",
        "dsb",
        "isb",
        basepri = in(reg) prior_basepri,
        faultmask = in(reg) prior_faultmask,
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );

    let trigger_pc = smoke_trigger_undefined_instruction as *const () as usize as u32 & !1;
    let usage_ok = usage_entries == 1
        && usage_ipsr == USAGEFAULT_VECTOR as u32
        && usage_cfsr & CFSR_UNDEFINSTR != 0
        && usage_hfsr & HFSR_FORCED == 0
        && usage_shcsr & SHCSR_USGFAULTENA != 0
        && usage_exc_return == 0xFFFF_FFF9
        && usage_stacked_pc == trigger_pc
        && usage_status_after_return & CFSR_UNDEFINSTR != 0;
    let hardfault_ok = hardfault_entries == 2
        && hardfault_ipsr == HARDFAULT_VECTOR as u32
        && hardfault_cfsr & CFSR_UNDEFINSTR != 0
        && hardfault_hfsr & HFSR_FORCED != 0
        && hardfault_shcsr & SHCSR_USGFAULTENA == 0
        && hardfault_exc_return == 0xFFFF_FFF9
        && hardfault_stacked_pc == trigger_pc
        && hardfault_status_after_return & HFSR_FORCED != 0;
    let clean_start = source_shcsr & SHCSR_USGFAULTENA == 0
        && source_cfsr & CFSR_UNDEFINSTR == 0
        && source_hfsr & HFSR_FORCED == 0;
    let restored = read_volatile(SCB_VTOR as *const u32) == source_vtor
        && read_volatile(SCB_SHCSR as *const u32) == source_shcsr
        && read_volatile(SCB_CFSR as *const u32) & CFSR_UNDEFINSTR == 0
        && read_volatile(SCB_HFSR as *const u32) & HFSR_FORCED == 0;
    let ok = clean_start
        && prior_primask == 0
        && prior_basepri == 0
        && prior_faultmask == 0
        && usage_ok
        && hardfault_ok
        && restored;

    if ok {
        putline(b"FAULT M3 UNDEFINSTR USAGE=6 ESCALATE=3 CFSR=1 HFSR_FORCED=1 RETURN=1 ok");
    } else {
        putline(b"FAULT M3 UNDEFINSTR CHECK FAILED");
    }
    ok
}

/// Capture a fault's SCB state and skip the two-byte UDF in the stacked frame
/// so exception return resumes at the trampoline's `bx lr`.
#[unsafe(naked)]
#[no_mangle]
unsafe extern "C" fn smoke_fault_handler() {
    core::arch::naked_asm!(
        "ldr r0, =0x20000680",
        "ldr r2, [r0, #0]",
        "adds r2, r2, #1",
        "str r2, [r0, #0]",
        "mrs r2, ipsr",
        "str r2, [r0, #4]",
        "ldr r3, =0xE000ED28",
        "ldr r2, [r3]",
        "str r2, [r0, #8]",
        "ldr r3, =0xE000ED2C",
        "ldr r2, [r3]",
        "str r2, [r0, #12]",
        "ldr r3, =0xE000ED24",
        "ldr r2, [r3]",
        "str r2, [r0, #16]",
        "mov r2, lr",
        "str r2, [r0, #20]",
        "tst lr, #4",
        "beq 2f",
        "mrs r1, psp",
        "b 3f",
        "2:",
        "mrs r1, msp",
        "3:",
        "ldr r2, [r1, #24]",
        "str r2, [r0, #24]",
        "adds r2, r2, #2",
        "str r2, [r1, #24]",
        "bx lr",
    );
}

/// A two-instruction target makes the stacked UDF PC and exception return
/// observable without relying on compiler-generated code after the fault.
#[unsafe(naked)]
unsafe extern "C" fn smoke_trigger_undefined_instruction() {
    core::arch::naked_asm!("udf #0", "bx lr");
}

/// Verify BASEPRI blocks a lower-priority external interrupt while allowing a
/// higher-priority interrupt through, then releases the still-pending request.
unsafe fn smoke_basepri_priority_mask() -> bool {
    const NVIC_ISER1: u32 = 0xE000_E104;
    const NVIC_ICER1: u32 = 0xE000_E184;
    const NVIC_ISPR1: u32 = 0xE000_E204;
    const NVIC_ICPR1: u32 = 0xE000_E284;
    const NVIC_IABR1: u32 = 0xE000_E304;
    const NVIC_IPR_WORD: u32 = 0xE000_E424;
    const VECTOR_LOW: usize = 16 + BASEPRI_IRQ_LOW as usize;
    const VECTOR_HIGH: usize = 16 + BASEPRI_IRQ_HIGH as usize;
    const VECTOR_WORDS: usize = VECTOR_HIGH + 1;
    const RECORD_WORDS: usize = 10;

    let source_vtor = read_volatile(SCB_VTOR as *const u32);
    let prior_primask: u32;
    let prior_basepri: u32;
    core::arch::asm!(
        "mrs {primask}, PRIMASK",
        "mrs {basepri}, BASEPRI",
        "cpsid i",
        primask = out(reg) prior_primask,
        basepri = out(reg) prior_basepri,
        options(nomem, nostack, preserves_flags),
    );

    let priority_before = read_volatile(NVIC_IPR_WORD as *const u32);
    let prior_enabled = read_volatile(NVIC_ISER1 as *const u32) & BASEPRI_IRQ_MASK;
    write_volatile(NVIC_ICER1 as *mut u32, BASEPRI_IRQ_MASK);
    write_volatile(NVIC_ICPR1 as *mut u32, BASEPRI_IRQ_MASK);
    write_volatile(
        NVIC_IPR_WORD as *mut u32,
        (priority_before & 0x0000_FFFF) | 0x4080_0000,
    );

    let table = IRQ_FRAME_VECTOR_TABLE as *mut u32;
    for slot in 0..VECTOR_WORDS {
        write_volatile(table.add(slot), 0);
    }
    write_volatile(
        table.add(VECTOR_LOW),
        (BasepriLowHandler as *const () as usize as u32) | 1,
    );
    write_volatile(
        table.add(VECTOR_HIGH),
        (BasepriHighHandler as *const () as usize as u32) | 1,
    );
    let record = BASEPRI_RECORD as *mut u32;
    for word in 0..RECORD_WORDS {
        write_volatile(record.add(word), 0);
    }
    write_volatile(SCB_VTOR as *mut u32, IRQ_FRAME_VECTOR_TABLE);
    core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));

    let vector_installed = read_volatile(SCB_VTOR as *const u32) == IRQ_FRAME_VECTOR_TABLE;
    let priorities = read_volatile(NVIC_IPR_WORD as *const u32) & 0xFFFF_0000;
    write_volatile(NVIC_ISER1 as *mut u32, BASEPRI_IRQ_MASK);
    core::arch::asm!(
        "msr BASEPRI, {mask}",
        "dsb",
        "isb",
        mask = in(reg) 0x80_u32,
        options(nostack, preserves_flags),
    );
    let basepri_after_max_weaker: u32;
    let basepri_after_max_stronger: u32;
    let basepri_after_restore: u32;
    core::arch::asm!(
        "msr BASEPRI_MAX, {weaker}",
        "mrs {after_weaker}, BASEPRI",
        "msr BASEPRI_MAX, {stronger}",
        "mrs {after_stronger}, BASEPRI",
        "msr BASEPRI, {restore}",
        "mrs {after_restore}, BASEPRI",
        weaker = in(reg) 0xC0_u32,
        stronger = in(reg) 0x40_u32,
        restore = in(reg) 0x80_u32,
        after_weaker = out(reg) basepri_after_max_weaker,
        after_stronger = out(reg) basepri_after_max_stronger,
        after_restore = out(reg) basepri_after_restore,
        options(nostack, preserves_flags),
    );
    write_volatile(NVIC_ISPR1 as *mut u32, BASEPRI_IRQ_MASK);
    core::arch::asm!(
        "msr PRIMASK, {primask}",
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );

    for _ in 0..4_096 {
        if read_volatile(record.add(0)) != 0 {
            break;
        }
        core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
    }
    let high_entries = read_volatile(record.add(0));
    let high_ipsr = read_volatile(record.add(1));
    let high_active = read_volatile(record.add(2));
    let high_pending = read_volatile(record.add(3));
    let high_basepri = read_volatile(record.add(4));
    let low_entries_before_release = read_volatile(record.add(5));
    let low_pending_before_release = read_volatile(NVIC_ISPR1 as *const u32) & BASEPRI_IRQ_MASK;
    let low_active_before_release = read_volatile(NVIC_IABR1 as *const u32) & BASEPRI_IRQ_MASK;
    let basepri_before_release: u32;
    core::arch::asm!(
        "mrs {basepri}, BASEPRI",
        basepri = out(reg) basepri_before_release,
        options(nomem, nostack, preserves_flags),
    );

    core::arch::asm!(
        "msr BASEPRI, {basepri}",
        "dsb",
        "isb",
        basepri = in(reg) prior_basepri,
        options(nostack, preserves_flags),
    );
    for _ in 0..4_096 {
        if read_volatile(record.add(5)) != 0 {
            break;
        }
        core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
    }

    let low_entries = read_volatile(record.add(5));
    let low_ipsr = read_volatile(record.add(6));
    let low_active = read_volatile(record.add(7));
    let low_pending = read_volatile(record.add(8));
    let high_pending_after = read_volatile(NVIC_ISPR1 as *const u32) & BASEPRI_IRQ_MASK;

    core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
    write_volatile(NVIC_ICER1 as *mut u32, BASEPRI_IRQ_MASK);
    write_volatile(NVIC_ICPR1 as *mut u32, BASEPRI_IRQ_MASK);
    write_volatile(NVIC_IPR_WORD as *mut u32, priority_before);
    write_volatile(SCB_VTOR as *mut u32, source_vtor);
    if prior_enabled != 0 {
        write_volatile(NVIC_ISER1 as *mut u32, prior_enabled);
    }
    core::arch::asm!(
        "msr BASEPRI, {basepri}",
        "msr PRIMASK, {primask}",
        "dsb",
        "isb",
        basepri = in(reg) prior_basepri,
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );

    let masked = high_entries == 1
        && high_ipsr == 16 + BASEPRI_IRQ_HIGH
        && high_active == BASEPRI_IRQ_HIGH_BIT
        && high_pending == BASEPRI_IRQ_LOW_BIT
        && high_basepri == 0x80
        && basepri_after_max_weaker == 0x80
        && basepri_after_max_stronger == 0x40
        && basepri_after_restore == 0x80
        && low_entries_before_release == 0
        && low_pending_before_release == BASEPRI_IRQ_LOW_BIT
        && low_active_before_release == 0
        && basepri_before_release == 0x80;
    let released = low_entries == 1
        && low_ipsr == 16 + BASEPRI_IRQ_LOW
        && low_active == BASEPRI_IRQ_LOW_BIT
        && low_pending == 0
        && high_pending_after == 0
        && read_volatile(NVIC_IABR1 as *const u32) & BASEPRI_IRQ_MASK == 0
        && read_volatile(NVIC_ISPR1 as *const u32) & BASEPRI_IRQ_MASK == 0;
    let restored = read_volatile(SCB_VTOR as *const u32) == source_vtor
        && read_volatile(NVIC_IPR_WORD as *const u32) == priority_before
        && read_volatile(NVIC_ICPR1 as *const u32) & BASEPRI_IRQ_MASK == 0;
    let ok = prior_primask == 0
        && prior_basepri == 0
        && vector_installed
        && priorities == 0x4080_0000
        && masked
        && released
        && restored;

    if ok {
        putline(
            b"BASEPRI M3 MAX_KEEP=1 MAX_RAISE=1 HIGH=55 LOW=54 MASKED=1 PENDING=1 RELEASED=1 ok",
        );
    } else {
        putline(b"BASEPRI M3 CHECK FAILED");
    }
    ok
}

/// Check Cortex-M3 priority grouping on the imported system. With PRIGROUP=5,
/// equal group priority IRQs are ordered by subpriority, but a better
/// subpriority cannot preempt an active IRQ in that same group.
unsafe fn smoke_nvic_subpriority_order_and_preemption() -> bool {
    const NVIC_ISER1: u32 = 0xE000_E104;
    const NVIC_ICER1: u32 = 0xE000_E184;
    const NVIC_ISPR1: u32 = 0xE000_E204;
    const NVIC_ICPR1: u32 = 0xE000_E284;
    const NVIC_IABR1: u32 = 0xE000_E304;
    const NVIC_IPR_WORD: u32 = 0xE000_E424;
    const VECTOR_LOW: usize = 16 + BASEPRI_IRQ_LOW as usize;
    const VECTOR_HIGH: usize = 16 + BASEPRI_IRQ_HIGH as usize;
    const VECTOR_WORDS: usize = VECTOR_HIGH + 1;
    const IPR_MASK: u32 = 0xFFFF_0000;
    const PRIORITIES: u32 = 0x8090_0000;

    let source_vtor = read_volatile(SCB_VTOR as *const u32);
    let prior_aircr = read_volatile(0xE000_ED0C as *const u32);
    let prior_priority = read_volatile(NVIC_IPR_WORD as *const u32);
    let prior_enabled = read_volatile(NVIC_ISER1 as *const u32) & BASEPRI_IRQ_MASK;
    let prior_primask: u32;
    let prior_basepri: u32;
    core::arch::asm!(
        "mrs {primask}, PRIMASK",
        "mrs {basepri}, BASEPRI",
        "cpsid i",
        primask = out(reg) prior_primask,
        basepri = out(reg) prior_basepri,
        options(nomem, nostack, preserves_flags),
    );

    let table = NVIC_PRIORITY_VECTOR_TABLE as *mut u32;
    let record = NVIC_PRIORITY_RECORD as *mut u32;
    write_volatile(NVIC_ICER1 as *mut u32, BASEPRI_IRQ_MASK);
    write_volatile(NVIC_ICPR1 as *mut u32, BASEPRI_IRQ_MASK);
    for slot in 0..VECTOR_WORDS {
        write_volatile(table.add(slot), 0);
    }
    for word in 0..8 {
        write_volatile(record.add(word), 0);
    }
    write_volatile(
        table.add(VECTOR_LOW),
        (nvic_priority_order_handler as *const () as usize as u32) | 1,
    );
    write_volatile(
        table.add(VECTOR_HIGH),
        (nvic_priority_order_handler as *const () as usize as u32) | 1,
    );
    write_volatile(
        0xE000_ED0C as *mut u32,
        0x05FA_0000 | (prior_aircr & 0x0000_F8F8) | (5 << 8),
    );
    write_volatile(
        NVIC_IPR_WORD as *mut u32,
        (prior_priority & !IPR_MASK) | PRIORITIES,
    );
    write_volatile(SCB_VTOR as *mut u32, NVIC_PRIORITY_VECTOR_TABLE);
    core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));
    let ordering_setup = read_volatile(SCB_VTOR as *const u32) == NVIC_PRIORITY_VECTOR_TABLE
        && read_volatile(0xE000_ED0C as *const u32) & 0x700 == 5 << 8
        && read_volatile(NVIC_IPR_WORD as *const u32) & IPR_MASK == PRIORITIES;
    write_volatile(NVIC_ISER1 as *mut u32, BASEPRI_IRQ_MASK);
    write_volatile(NVIC_ISPR1 as *mut u32, BASEPRI_IRQ_MASK);
    core::arch::asm!(
        "msr PRIMASK, {primask}",
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );
    for _ in 0..4_096 {
        if read_volatile(record) == 2 {
            break;
        }
        core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
    }
    let order_count = read_volatile(record);
    let first = read_volatile(record.add(1));
    let second = read_volatile(record.add(2));
    let order_pending = read_volatile(NVIC_ISPR1 as *const u32) & BASEPRI_IRQ_MASK;
    let order_active = read_volatile(NVIC_IABR1 as *const u32) & BASEPRI_IRQ_MASK;

    core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
    write_volatile(NVIC_ICER1 as *mut u32, BASEPRI_IRQ_MASK);
    write_volatile(NVIC_ICPR1 as *mut u32, BASEPRI_IRQ_MASK);
    for word in 0..8 {
        write_volatile(record.add(word), 0);
    }
    write_volatile(
        table.add(VECTOR_LOW),
        (nvic_priority_nonpreempt_parent as *const () as usize as u32) | 1,
    );
    write_volatile(
        table.add(VECTOR_HIGH),
        (nvic_priority_nonpreempt_child as *const () as usize as u32) | 1,
    );
    write_volatile(NVIC_ISER1 as *mut u32, BASEPRI_IRQ_MASK);
    write_volatile(NVIC_ISPR1 as *mut u32, BASEPRI_IRQ_LOW_BIT);
    core::arch::asm!(
        "msr PRIMASK, {primask}",
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );
    for _ in 0..4_096 {
        if read_volatile(record.add(7)) != 0 {
            break;
        }
        core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
    }
    let parent_ipsr = read_volatile(record.add(3));
    let active_during = read_volatile(record.add(4));
    let pending_during = read_volatile(record.add(5));
    let ipsr_after_pend = read_volatile(record.add(6));
    let child_ipsr = read_volatile(record.add(7));
    let nonpreempt_pending = read_volatile(NVIC_ISPR1 as *const u32) & BASEPRI_IRQ_MASK;
    let nonpreempt_active = read_volatile(NVIC_IABR1 as *const u32) & BASEPRI_IRQ_MASK;

    core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
    write_volatile(NVIC_ICER1 as *mut u32, BASEPRI_IRQ_MASK);
    write_volatile(NVIC_ICPR1 as *mut u32, BASEPRI_IRQ_MASK);
    write_volatile(NVIC_IPR_WORD as *mut u32, prior_priority);
    write_volatile(
        0xE000_ED0C as *mut u32,
        0x05FA_0000 | (prior_aircr & 0x0000_F8F8) | (prior_aircr & 0x700),
    );
    write_volatile(SCB_VTOR as *mut u32, source_vtor);
    write_volatile(NVIC_ISER1 as *mut u32, prior_enabled);
    core::arch::asm!(
        "msr BASEPRI, {basepri}",
        "msr PRIMASK, {primask}",
        "dsb",
        "isb",
        basepri = in(reg) prior_basepri,
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );

    let ordering_ok = ordering_setup
        && order_count == 2
        && first == 16 + BASEPRI_IRQ_HIGH
        && second == 16 + BASEPRI_IRQ_LOW
        && order_pending == 0
        && order_active == 0;
    let nonpreempt_ok = parent_ipsr == 16 + BASEPRI_IRQ_LOW
        && active_during == BASEPRI_IRQ_LOW_BIT
        && pending_during == BASEPRI_IRQ_HIGH_BIT
        && ipsr_after_pend == 16 + BASEPRI_IRQ_LOW
        && child_ipsr == 16 + BASEPRI_IRQ_HIGH
        && nonpreempt_pending == 0
        && nonpreempt_active == 0;
    let restored = read_volatile(SCB_VTOR as *const u32) == source_vtor
        && read_volatile(NVIC_IPR_WORD as *const u32) == prior_priority
        && read_volatile(0xE000_ED0C as *const u32) & 0x700 == prior_aircr & 0x700
        && read_volatile(NVIC_ISER1 as *const u32) & BASEPRI_IRQ_MASK == prior_enabled
        && read_volatile(NVIC_ISPR1 as *const u32) & BASEPRI_IRQ_MASK == 0
        && read_volatile(NVIC_IABR1 as *const u32) & BASEPRI_IRQ_MASK == 0;
    let ok = prior_primask == 0 && prior_basepri == 0 && ordering_ok && nonpreempt_ok && restored;

    if ok {
        putline(b"NVIC M3 SUBPRIORITY ORDER=55,54 SAME_GROUP_PREEMPT=0 PENDING=1 ok");
    } else {
        putline(b"NVIC M3 SUBPRIORITY CHECK FAILED");
        print_reg(b"first exception=", first);
        print_reg(b"second exception=", second);
        print_reg(b"same-group active=", active_during);
        print_reg(b"same-group pending=", pending_during);
        print_reg(b"same-group IPSR=", ipsr_after_pend);
        print_reg(b"same-group child=", child_ipsr);
    }
    ok
}

/// Verify PRIMASK keeps a pending external IRQ inactive until it is cleared.
/// Reuse the BASEPRI low-priority vector handler to inspect the released IRQ.
unsafe fn smoke_primask_external_irq() -> bool {
    const NVIC_ISER1: u32 = 0xE000_E104;
    const NVIC_ICER1: u32 = 0xE000_E184;
    const NVIC_ISPR1: u32 = 0xE000_E204;
    const NVIC_ICPR1: u32 = 0xE000_E284;
    const NVIC_IABR1: u32 = 0xE000_E304;
    const NVIC_IPR_WORD: u32 = 0xE000_E424;
    const VECTOR_INDEX: usize = 16 + BASEPRI_IRQ_LOW as usize;
    const VECTOR_WORDS: usize = VECTOR_INDEX + 1;
    const RECORD_WORDS: usize = 10;

    let source_vtor = read_volatile(SCB_VTOR as *const u32);
    let prior_primask: u32;
    let prior_basepri: u32;
    core::arch::asm!(
        "mrs {primask}, PRIMASK",
        "mrs {basepri}, BASEPRI",
        "cpsid i",
        primask = out(reg) prior_primask,
        basepri = out(reg) prior_basepri,
        options(nomem, nostack, preserves_flags),
    );

    let priority_before = read_volatile(NVIC_IPR_WORD as *const u32);
    let prior_enabled = read_volatile(NVIC_ISER1 as *const u32) & BASEPRI_IRQ_LOW_BIT;
    write_volatile(NVIC_ICER1 as *mut u32, BASEPRI_IRQ_LOW_BIT);
    write_volatile(NVIC_ICPR1 as *mut u32, BASEPRI_IRQ_LOW_BIT);
    write_volatile(
        NVIC_IPR_WORD as *mut u32,
        (priority_before & !0x00FF_0000) | 0x0080_0000,
    );

    let table = IRQ_FRAME_VECTOR_TABLE as *mut u32;
    for slot in 0..VECTOR_WORDS {
        write_volatile(table.add(slot), 0);
    }
    write_volatile(
        table.add(VECTOR_INDEX),
        (BasepriLowHandler as *const () as usize as u32) | 1,
    );
    let record = BASEPRI_RECORD as *mut u32;
    for word in 0..RECORD_WORDS {
        write_volatile(record.add(word), 0);
    }
    // BasepriLowHandler stores the IRQ 38 result in record words 5 through 9.
    let low_irq_record = record.add(5);
    write_volatile(SCB_VTOR as *mut u32, IRQ_FRAME_VECTOR_TABLE);
    core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));

    let vector_installed = read_volatile(SCB_VTOR as *const u32) == IRQ_FRAME_VECTOR_TABLE;
    let priority_installed =
        read_volatile(NVIC_IPR_WORD as *const u32) & 0x00FF_0000 == 0x0080_0000;
    write_volatile(NVIC_ISER1 as *mut u32, BASEPRI_IRQ_LOW_BIT);
    core::arch::asm!("dsb", "isb", options(nostack, preserves_flags));
    write_volatile(NVIC_ISPR1 as *mut u32, BASEPRI_IRQ_LOW_BIT);
    let pending_while_masked = read_volatile(NVIC_ISPR1 as *const u32) & BASEPRI_IRQ_LOW_BIT;
    let active_while_masked = read_volatile(NVIC_IABR1 as *const u32) & BASEPRI_IRQ_LOW_BIT;
    let entries_while_masked = read_volatile(low_irq_record);
    let primask_value: u32;
    core::arch::asm!(
        "mrs {primask}, PRIMASK",
        primask = out(reg) primask_value,
        options(nomem, nostack, preserves_flags),
    );

    core::arch::asm!(
        "msr PRIMASK, {primask}",
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );
    for _ in 0..4_096 {
        if read_volatile(low_irq_record) != 0 {
            break;
        }
        core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
    }
    let entries_after_release = read_volatile(low_irq_record);
    let handler_ipsr = read_volatile(low_irq_record.add(1));
    let handler_active = read_volatile(low_irq_record.add(2));
    let handler_pending = read_volatile(low_irq_record.add(3));
    let handler_basepri = read_volatile(low_irq_record.add(4));
    let pending_after_release = read_volatile(NVIC_ISPR1 as *const u32) & BASEPRI_IRQ_LOW_BIT;

    core::arch::asm!("cpsid i", options(nomem, nostack, preserves_flags));
    write_volatile(NVIC_ICER1 as *mut u32, BASEPRI_IRQ_LOW_BIT);
    write_volatile(NVIC_ICPR1 as *mut u32, BASEPRI_IRQ_LOW_BIT);
    write_volatile(NVIC_IPR_WORD as *mut u32, priority_before);
    write_volatile(SCB_VTOR as *mut u32, source_vtor);
    if prior_enabled != 0 {
        write_volatile(NVIC_ISER1 as *mut u32, prior_enabled);
    }
    core::arch::asm!(
        "msr BASEPRI, {basepri}",
        "msr PRIMASK, {primask}",
        "dsb",
        "isb",
        basepri = in(reg) prior_basepri,
        primask = in(reg) prior_primask,
        options(nostack, preserves_flags),
    );

    let masked = primask_value == 1
        && pending_while_masked == BASEPRI_IRQ_LOW_BIT
        && active_while_masked == 0
        && entries_while_masked == 0;
    let released = entries_after_release == 1
        && handler_ipsr == 16 + BASEPRI_IRQ_LOW
        && handler_active == BASEPRI_IRQ_LOW_BIT
        && handler_pending == 0
        && handler_basepri == prior_basepri
        && pending_after_release == 0
        && read_volatile(NVIC_IABR1 as *const u32) & BASEPRI_IRQ_LOW_BIT == 0;
    let restored = read_volatile(SCB_VTOR as *const u32) == source_vtor
        && read_volatile(NVIC_IPR_WORD as *const u32) == priority_before
        && read_volatile(NVIC_ISER1 as *const u32) & BASEPRI_IRQ_LOW_BIT == prior_enabled
        && read_volatile(NVIC_ISPR1 as *const u32) & BASEPRI_IRQ_LOW_BIT == 0;
    let ok = prior_primask == 0
        && prior_basepri == 0
        && vector_installed
        && priority_installed
        && masked
        && released
        && restored;

    if ok {
        putline(b"PRIMASK M3 MASKED=1 PENDING=1 RELEASED=1 VECTOR=54 ok");
    } else {
        putline(b"PRIMASK M3 CHECK FAILED");
        print_reg(b"prior PRIMASK=", prior_primask);
        print_reg(b"prior BASEPRI=", prior_basepri);
        print_reg(b"PRIMASK during=", primask_value);
        print_reg(b"pending masked=", pending_while_masked);
        print_reg(b"active masked=", active_while_masked);
        print_reg(b"entries masked=", entries_while_masked);
        print_reg(b"entries released=", entries_after_release);
        print_reg(b"handler IPSR=", handler_ipsr);
        print_reg(b"handler active=", handler_active);
        print_reg(b"handler pending=", handler_pending);
        print_reg(b"handler BASEPRI=", handler_basepri);
        print_reg(b"pending released=", pending_after_release);
        print_reg(
            b"pending after cleanup=",
            read_volatile(NVIC_ISPR1 as *const u32),
        );
    }
    ok
}

#[no_mangle]
pub extern "C" fn BasepriHighHandler() {
    let ipsr: u32;
    let basepri: u32;
    unsafe {
        core::arch::asm!(
            "mrs {ipsr}, IPSR",
            "mrs {basepri}, BASEPRI",
            ipsr = out(reg) ipsr,
            basepri = out(reg) basepri,
            options(nomem, nostack, preserves_flags),
        );
    }
    unsafe {
        let record = BASEPRI_RECORD as *mut u32;
        let entries = read_volatile(record.add(0));
        write_volatile(record.add(0), entries.wrapping_add(1));
        write_volatile(record.add(1), ipsr);
        write_volatile(
            record.add(2),
            read_volatile(0xE000_E304 as *const u32) & BASEPRI_IRQ_MASK,
        );
        write_volatile(
            record.add(3),
            read_volatile(0xE000_E204 as *const u32) & BASEPRI_IRQ_MASK,
        );
        write_volatile(record.add(4), basepri);
    }
}

#[no_mangle]
pub extern "C" fn BasepriLowHandler() {
    let ipsr: u32;
    let basepri: u32;
    unsafe {
        core::arch::asm!(
            "mrs {ipsr}, IPSR",
            "mrs {basepri}, BASEPRI",
            ipsr = out(reg) ipsr,
            basepri = out(reg) basepri,
            options(nomem, nostack, preserves_flags),
        );
    }
    unsafe {
        let record = BASEPRI_RECORD as *mut u32;
        let entries = read_volatile(record.add(5));
        write_volatile(record.add(5), entries.wrapping_add(1));
        write_volatile(record.add(6), ipsr);
        write_volatile(
            record.add(7),
            read_volatile(0xE000_E304 as *const u32) & BASEPRI_IRQ_MASK,
        );
        write_volatile(
            record.add(8),
            read_volatile(0xE000_E204 as *const u32) & BASEPRI_IRQ_MASK,
        );
        write_volatile(record.add(9), basepri);
    }
}

/// Push r4/r7 to preserve the ABI and keep MSP aligned, then pend USART2's
/// NVIC vector with fixed values in the architectural exception-frame regs.
#[unsafe(naked)]
unsafe extern "C" fn smoke_pend_usart2_irq_with_sentinels() {
    core::arch::naked_asm!(
        "push {{r4, r7}}",
        "mrs r4, msp",
        "ldr r7, =0x2000053C",
        "str r4, [r7]",
        "ldr r7, =0xE000E204",
        "movs r0, #0x11",
        "movs r1, #0x22",
        "movs r2, #0x33",
        "movs r3, #0x44",
        "movs r4, #0x55",
        "mov r12, r4",
        "movs r4, #0x40",
        "str r4, [r7]",
        "pop {{r4, r7}}",
        "bx lr",
    );
}

/// Capture the hardware frame directly from MSP before Rust adds a prologue.
#[unsafe(naked)]
#[no_mangle]
unsafe extern "C" fn smoke_external_irq_frame_handler() {
    core::arch::naked_asm!(
        "ldr r0, =0x20000500",
        "tst lr, #4",
        "beq 2f",
        "mrs r1, psp",
        "b 3f",
        "2:",
        "mrs r1, msp",
        "3:",
        "str r1, [r0, #0]",
        "ldr r2, [r1, #0]",
        "str r2, [r0, #4]",
        "ldr r2, [r1, #4]",
        "str r2, [r0, #8]",
        "ldr r2, [r1, #8]",
        "str r2, [r0, #12]",
        "ldr r2, [r1, #12]",
        "str r2, [r0, #16]",
        "ldr r2, [r1, #16]",
        "str r2, [r0, #20]",
        "ldr r2, [r1, #20]",
        "str r2, [r0, #24]",
        "ldr r2, [r1, #24]",
        "str r2, [r0, #28]",
        "ldr r2, [r1, #28]",
        "str r2, [r0, #32]",
        "mov r2, lr",
        "str r2, [r0, #36]",
        "mrs r2, ipsr",
        "str r2, [r0, #40]",
        "ldr r2, =0xE000E304",
        "ldr r3, [r2]",
        "str r3, [r0, #44]",
        "ldr r2, =0xE000E204",
        "ldr r3, [r2]",
        "str r3, [r0, #48]",
        "mrs r3, msp",
        "str r3, [r0, #68]",
        "movs r2, #1",
        "str r2, [r0, #56]",
        "bx lr",
    );
}

/// Record the active exception number for the PRIGROUP=5 arbitration probe.
#[unsafe(naked)]
unsafe extern "C" fn nvic_priority_order_handler() {
    core::arch::naked_asm!(
        "ldr r0, =0x20000A00",
        "ldr r1, [r0]",
        "cmp r1, #2",
        "bhs 1f",
        "mrs r2, ipsr",
        "adds r1, r1, #1",
        "str r1, [r0]",
        "lsls r3, r1, #2",
        "str r2, [r0, r3]",
        "1:",
        "bx lr",
    );
}

/// Pend the better-subpriority IRQ from IRQ 54, then record that IRQ 54 stays
/// active while IRQ 55 is pending and has not preempted it.
#[unsafe(naked)]
unsafe extern "C" fn nvic_priority_nonpreempt_parent() {
    core::arch::naked_asm!(
        "ldr r0, =0x20000A00",
        "mrs r2, ipsr",
        "str r2, [r0, #12]",
        "ldr r3, =0xE000E204",
        "movs r2, #0x80",
        "str r2, [r3]",
        "dsb",
        "isb",
        "ldr r3, =0xE000E304",
        "ldr r2, [r3]",
        "str r2, [r0, #16]",
        "ldr r3, =0xE000E204",
        "ldr r2, [r3]",
        "str r2, [r0, #20]",
        "mrs r2, ipsr",
        "str r2, [r0, #24]",
        "bx lr",
    );
}

/// Capture IRQ 55's exception number after IRQ 54 returns.
#[unsafe(naked)]
unsafe extern "C" fn nvic_priority_nonpreempt_child() {
    core::arch::naked_asm!(
        "ldr r0, =0x20000A00",
        "mrs r1, ipsr",
        "str r1, [r0, #28]",
        "bx lr",
    );
}

/// Run with a dedicated PSP and pend USART2 after loading frame sentinels.
/// MSP remains reserved for Handler mode and the saved caller return address.
#[unsafe(naked)]
unsafe extern "C" fn smoke_pend_usart2_irq_with_psp_sentinels() {
    core::arch::naked_asm!(
        "push {{r4, r5, r6, lr}}",
        "mrs r5, msp",
        "ldr r6, =0x20000534",
        "str r5, [r6]",
        "ldr r5, =0x20004000",
        "msr psp, r5",
        "movs r4, #2",
        "msr control, r4",
        "isb",
        "movs r0, #0x11",
        "movs r1, #0x22",
        "movs r2, #0x33",
        "movs r3, #0x44",
        "movs r5, #0x55",
        "mov r12, r5",
        "movs r4, #0x40",
        "ldr r6, =0xE000E204",
        "str r4, [r6]",
        "ldr r6, =0x20000540",
        "movs r4, #1",
        "str r4, [r6]",
        "movs r4, #0",
        "msr control, r4",
        "isb",
        "pop {{r4, r5, r6, pc}}",
    );
}

/// Cortex-M exception handler installed at vector-table slot 14 by the smoke.
#[no_mangle]
pub extern "C" fn PendSV_Handler() {
    unsafe {
        let ipsr: u32;
        core::arch::asm!(
            "mrs {ipsr}, IPSR",
            ipsr = out(reg) ipsr,
            options(nomem, nostack, preserves_flags),
        );
        let entries = read_volatile(PENDSV_HANDLER_COUNT);
        write_volatile(PENDSV_HANDLER_IPSR, ipsr);
        if read_volatile(SYSTICK_HANDLER_PENDSV_REQUEST) != 0 {
            write_volatile(
                PENDSV_HANDLER_SYSTICK_COUNT,
                read_volatile(SYSTICK_HANDLER_COUNT),
            );
        }
        write_volatile(PENDSV_HANDLER_COUNT, entries.wrapping_add(1));
    }
}

/// Cortex-M exception handler installed at vector-table slot 15 by minimal.ld.
#[no_mangle]
pub extern "C" fn SysTick_Handler() {
    unsafe {
        let count = read_volatile(SYSTICK_HANDLER_COUNT);
        let icsr = read_volatile(SCB_ICSR as *const u32);
        if count == 0 && read_volatile(SYSTICK_HANDLER_PENDSV_REQUEST) != 0 {
            write_volatile(SYSTICK_CSR as *mut u32, 0);
            write_volatile(SCB_ICSR as *mut u32, ICSR_PENDSVSET);
            let pending = read_volatile(SCB_ICSR as *const u32) & ICSR_PENDSVSET != 0;
            let pendsv_inactive = read_volatile(PENDSV_HANDLER_COUNT) == 0;
            write_volatile(
                SYSTICK_HANDLER_PENDSV_DEFERRED,
                u32::from(pending && pendsv_inactive),
            );
        }
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
