// Arduino UNO R4 Minima (R7FA4M1AB) + BME280 smoke firmware.
// Bare-metal IIC1 transaction so the simulator does not enter the board's
// unmodelled USBFS CDC initialization path.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// RA4M1 IIC1 @ 0x40053100. The RIIC master follows the first BME280 access:
// address+write, register 0xD0, repeated start, address+read, one-byte read.
const IIC1: usize = 0x4005_3100;
const ICCR1: *mut u8 = IIC1 as *mut u8;
const ICCR2: *mut u8 = (IIC1 + 0x01) as *mut u8;
const ICMR3: *mut u8 = (IIC1 + 0x04) as *mut u8;
const ICIER: *mut u8 = (IIC1 + 0x07) as *mut u8;
const ICSR2: *mut u8 = (IIC1 + 0x09) as *mut u8;
const ICBRL: *mut u8 = (IIC1 + 0x10) as *mut u8;
const ICBRH: *mut u8 = (IIC1 + 0x11) as *mut u8;
const ICDRT: *mut u8 = (IIC1 + 0x12) as *mut u8;
const ICDRR: *mut u8 = (IIC1 + 0x13) as *mut u8;

// SCI2 @ 0x40070040 — TDR at +0x03, wired to D1 / the host console.
const SCI2_TDR: *mut u8 = 0x4007_0043 as *mut u8;
const PRCR: *mut u16 = 0x4001_E3FE as *mut u16;
const MSTPCRB: *mut u32 = 0x4004_7000 as *mut u32;

const ICCR2_ST: u8 = 1 << 1;
const ICCR2_RS: u8 = 1 << 2;
const ICCR2_SP: u8 = 1 << 3;
const ICSR2_RDRF: u8 = 1 << 5;
const ICSR2_TEND: u8 = 1 << 6;
const ICSR2_NACKF: u8 = 1 << 4;
const ICSR2_START: u8 = 1 << 2;
const ICSR2_STOP: u8 = 1 << 3;
const ICMR3_ACKWP: u8 = 1 << 4;
const ICMR3_ACKBT: u8 = 1 << 3;
const BME280_ADDRESS: u8 = 0x77;
const BME280_CHIP_ID_REGISTER: u8 = 0xD0;

unsafe fn wait_status(mask: u8) -> bool {
    for _ in 0..1_000_000 {
        let status = read_volatile(ICSR2);
        if status & ICSR2_NACKF != 0 {
            return false;
        }
        if status & mask == mask {
            return true;
        }
        cortex_m::asm::nop();
    }
    false
}

unsafe fn write_uart(text: &[u8]) {
    for byte in text {
        write_volatile(SCI2_TDR, *byte);
    }
}

unsafe fn read_bme280_chip_id() -> Option<u8> {
    // FSP-style protected clock enable: MSTPB8=1 stops IIC1 and MSTPB29=1
    // stops SCI2 out of reset. PRC1 permits the module-stop write.
    write_volatile(PRCR, 0xA502);
    write_volatile(MSTPCRB, read_volatile(MSTPCRB) & !((1 << 8) | (1 << 29)));
    write_volatile(PRCR, 0xA500);

    // FSP R_IIC master reset/open sequence. The modeled RIIC uses this
    // configured bit rate.
    write_volatile(ICCR1, 0x1F);
    write_volatile(ICCR1, 0x5F);
    write_volatile(ICCR1, 0xDF);
    write_volatile(ICBRL, 0xE2);
    write_volatile(ICBRH, 0xE2);
    write_volatile(ICIER, 0);
    write_volatile(ICCR1, 0x9F);

    // START, then the sensor's 7-bit write address and CHIP_ID register.
    write_volatile(ICCR2, ICCR2_ST);
    if !wait_status(ICSR2_START) {
        return None;
    }
    write_volatile(ICSR2, 0xE0); // clear START/NACK/STOP event flags
    write_volatile(ICDRT, BME280_ADDRESS << 1);
    if !wait_status(ICSR2_TEND) {
        return None;
    }
    write_volatile(ICDRT, BME280_CHIP_ID_REGISTER);
    if !wait_status(ICSR2_TEND) {
        return None;
    }

    // Repeated START, then the sensor's read address.
    write_volatile(ICCR2, ICCR2_RS);
    if !wait_status(ICSR2_START) {
        return None;
    }
    write_volatile(ICSR2, 0xE0);
    write_volatile(ICDRT, (BME280_ADDRESS << 1) | 1);
    if !wait_status(ICSR2_RDRF) {
        return None;
    }

    // One-byte master receive: NACK the last byte, consume the dummy read,
    // request STOP, then consume the actual sensor byte.
    write_volatile(ICMR3, ICMR3_ACKWP);
    write_volatile(ICMR3, ICMR3_ACKWP | ICMR3_ACKBT);
    write_volatile(ICMR3, ICMR3_ACKBT);
    let _ = read_volatile(ICDRR);
    if !wait_status(ICSR2_RDRF) {
        return None;
    }
    write_volatile(ICCR2, ICCR2_SP);
    let chip_id = read_volatile(ICDRR);
    if !wait_status(ICSR2_STOP) {
        return None;
    }
    Some(chip_id)
}

#[entry]
fn main() -> ! {
    unsafe {
        if let Some(0x60) = read_bme280_chip_id() {
            write_uart(b"UNO_R4_MINIMA_BME280_ID=0x60 IIC1=A4/A5\n");
        } else {
            write_uart(b"UNO_R4_MINIMA_BME280_I2C_ERROR\n");
        }
    }
    loop {}
}
