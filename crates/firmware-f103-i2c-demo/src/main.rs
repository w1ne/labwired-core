//! LabWired - STM32F103 I²C silicon-validation trace.
//!
//! Probes an absent I²C address, reads the TMP102 temperature, then writes and
//! reads back its configuration and alert threshold registers through I2C1,
//! including a one-byte MSB update. It also reads the BME280 chip ID and a
//! little-endian calibration word, then reads the BME280 pressure, temperature
//! and humidity ADC registers. USART2 emits a human-readable register
//! fingerprint at 115200/8N1. The onboarding system attaches declarative TMP102
//! and BME280 models at addresses 0x48 and 0x76; TMP102's reset temperature is
//! 25 °C (0x1900). The absent address remains an intentional NACK check.
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

    // Read the BME280's measurement window as a driver does. Pressure and
    // temperature are 20-bit values split across three bytes; humidity is a
    // 16-bit word. The imported system seeds 40 °C / 80 %RH / 950 hPa; exact
    // raw-code comparisons check that placement config reaches all three
    // channels in the model.
    let pressure = unsafe {
        read_i2c_register(BME280_ADDRESS, 0xF7, 2).and_then(|bytes| {
            read_i2c_byte(BME280_ADDRESS, 0xF9).map(|xlsb| {
                (u32::from(bytes[0]) << 12) | (u32::from(bytes[1]) << 4) | (u32::from(xlsb) >> 4)
            })
        })
    };
    let temperature = unsafe { read_bme280_temperature_raw() };
    let humidity = unsafe {
        read_i2c_register(BME280_ADDRESS, 0xFD, 2)
            .map(|bytes| (u32::from(bytes[0]) << 8) | u32::from(bytes[1]))
    };

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
                    break;
                }
            }
        }
    } else {
        putline(b"BME280_DYNAMIC_TEMPERATURE_READ_FAIL");
    }

    putline(b"DONE");

    loop {}
}

// ── Init helpers ──────────────────────────────────────────────────────────────

unsafe fn rcc_init() {
    // Enable GPIOA + GPIOB + AFIO on APB2 (bits 2, 3, 0).  GPIOB is
    // required because I2C1's default pin mapping is PB6 (SCL) / PB7
    // (SDA); without IOPBEN the GPIOB CRL writes go to a dead clock
    // domain and the I²C state machine never gets the bus pins it
    // needs to drive — which surfaces on real silicon as a stalled
    // START condition (CR1.START stays set, SB never fires).  The
    // simulator's I²C model does NOT check GPIO config so the same
    // firmware "works" there even without IOPBEN; matching silicon
    // is mandatory.
    rmw32(RCC_APB2ENR, (1 << 0) | (1 << 2) | (1 << 3));
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
