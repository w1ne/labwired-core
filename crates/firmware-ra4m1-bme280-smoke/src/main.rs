// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// Arduino UNO R4 Minima: configure IIC1 and read the BME280 chip ID over the
// A4/A5 I2C header. This is a register-level RA4M1 smoke, not an Arduino HAL.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

const HOCOCR: *mut u8 = 0x4001_E036 as *mut u8;
const OSCSF: *const u8 = 0x4001_E03C as *const u8;
const HCSTP: u8 = 1 << 0;
const HOCOSF: u8 = 1 << 0;

const SCI2_TDR: *mut u8 = 0x4007_0043 as *mut u8;

const PFS_BASE: u32 = 0x4004_0800;
const PWPR: *mut u8 = 0x4004_0D03 as *mut u8;
const PWPR_B0WI: u8 = 1 << 7;
const PWPR_PFSWE: u8 = 1 << 6;
const P100PFS: *mut u32 = (PFS_BASE + 0x40) as *mut u32;
const P101PFS: *mut u32 = (PFS_BASE + 0x44) as *mut u32;
const PSEL_IIC1: u32 = 0x07;
const PMR: u32 = 1 << 6;

const PORT1_PCNTR1: *mut u32 = 0x4004_0020 as *mut u32;
const PORT1_PCNTR3: *mut u32 = 0x4004_0028 as *mut u32;
const LED: u32 = 1 << 11;

const IIC1: u32 = 0x4005_3100;
const ICCR1: *mut u8 = IIC1 as *mut u8;
const ICCR2: *mut u8 = (IIC1 + 1) as *mut u8;
const ICMR3: *mut u8 = (IIC1 + 4) as *mut u8;
const ICSR2: *mut u8 = (IIC1 + 9) as *mut u8;
const ICBRL: *mut u8 = (IIC1 + 0x10) as *mut u8;
const ICBRH: *mut u8 = (IIC1 + 0x11) as *mut u8;
const ICDRT: *mut u8 = (IIC1 + 0x12) as *mut u8;
const ICDRR: *const u8 = (IIC1 + 0x13) as *const u8;

const ST: u8 = 1 << 1;
const RS: u8 = 1 << 2;
const SP: u8 = 1 << 3;
const TEND: u8 = 1 << 6;
const RDRF: u8 = 1 << 5;
const NACKF: u8 = 1 << 4;
const STOP: u8 = 1 << 3;
const START: u8 = 1 << 2;
const ACKWP: u8 = 1 << 4;
const ACKBT: u8 = 1 << 3;
const BME280_ADDRESS_WRITE: u8 = 0xEC;
const BME280_ADDRESS_READ: u8 = 0xED;
const BME280_ID_REGISTER: u8 = 0xD0;
const BME280_ID: u8 = 0x60;
const POLL_LIMIT: u32 = 250_000;

#[derive(Clone, Copy)]
enum ResultCode {
    Start,
    AddressWrite,
    IdRegister,
    Restart,
    AddressRead,
    DummyRead,
    ChipId,
    Stop,
}

fn wait_status(set: u8, clear: u8) -> bool {
    let mut left = POLL_LIMIT;
    while left != 0 {
        let status = unsafe { read_volatile(ICSR2) };
        if status & NACKF != 0 {
            return false;
        }
        if status & set == set && status & clear == 0 {
            return true;
        }
        left -= 1;
    }
    false
}

fn uart(bytes: &[u8]) {
    for byte in bytes {
        unsafe { write_volatile(SCI2_TDR, *byte) };
    }
}

fn fail(stage: ResultCode) -> ! {
    let message = match stage {
        ResultCode::Start => b"UNO_R4_BME280_FAIL start\n" as &[u8],
        ResultCode::AddressWrite => b"UNO_R4_BME280_FAIL address_w\n",
        ResultCode::IdRegister => b"UNO_R4_BME280_FAIL register\n",
        ResultCode::Restart => b"UNO_R4_BME280_FAIL restart\n",
        ResultCode::AddressRead => b"UNO_R4_BME280_FAIL address_r\n",
        ResultCode::DummyRead => b"UNO_R4_BME280_FAIL dummy_read\n",
        ResultCode::ChipId => b"UNO_R4_BME280_FAIL chip_id\n",
        ResultCode::Stop => b"UNO_R4_BME280_FAIL stop\n",
    };
    uart(message);
    loop {}
}

fn wait_or_fail(set: u8, clear: u8, stage: ResultCode) {
    if !wait_status(set, clear) {
        fail(stage);
    }
}

unsafe fn configure_iic1_pins() {
    // RA4M1 PFS write protection: clear B0WI, enable PFS writes, set the mux,
    // then restore protection. PSEL 7 selects IIC1 on P100/SCL and P101/SDA.
    write_volatile(PWPR, 0);
    write_volatile(PWPR, PWPR_PFSWE);
    write_volatile(P100PFS, PSEL_IIC1 | PMR);
    write_volatile(P101PFS, PSEL_IIC1 | PMR);
    write_volatile(PWPR, 0);
    write_volatile(PWPR, PWPR_B0WI);
}

unsafe fn configure_iic1() {
    // Reset and release the RIIC master; CKS=0 and BRH/BRL low bits = 2.
    write_volatile(ICCR1, 0x1F);
    write_volatile(ICCR1, 0x5F);
    write_volatile(ICCR1, 0xDF);
    write_volatile(ICBRL, 0xE2);
    write_volatile(ICBRH, 0xE2);
    write_volatile(ICCR1, 0x9F);
}

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(HOCOCR, 0);
        while read_volatile(OSCSF) & HOCOSF == 0 {
            if read_volatile(HOCOCR) & HCSTP != 0 {
                write_volatile(HOCOCR, 0);
            }
        }

        // Keep the existing board smoke's D13 indicator and SCI2 console.
        let pcntr1 = read_volatile(PORT1_PCNTR1);
        write_volatile(PORT1_PCNTR1, pcntr1 | LED);

        configure_iic1_pins();
        configure_iic1();

        // START, send BME280 address and ID register, then repeated START.
        write_volatile(ICCR2, ST);
        wait_or_fail(START, 0, ResultCode::Start);
        write_volatile(ICSR2, 0xE0); // Clear write-0-to-clear condition flags.
        write_volatile(ICDRT, BME280_ADDRESS_WRITE);
        wait_or_fail(TEND, 0, ResultCode::AddressWrite);
        write_volatile(ICDRT, BME280_ID_REGISTER);
        wait_or_fail(TEND, 0, ResultCode::IdRegister);

        write_volatile(ICCR2, RS);
        wait_or_fail(START, 0, ResultCode::Restart);
        write_volatile(ICSR2, 0xE0);
        write_volatile(ICDRT, BME280_ADDRESS_READ);
        wait_or_fail(RDRF, 0, ResultCode::AddressRead);

        // For a one-byte read, NACK the final byte. The first ICDRR read is
        // the RIIC dummy read that starts the data-byte receive.
        write_volatile(ICMR3, ACKWP);
        write_volatile(ICMR3, ACKWP | ACKBT);
        write_volatile(ICMR3, ACKBT);
        let _ = read_volatile(ICDRR);
        wait_or_fail(RDRF, 0, ResultCode::DummyRead);

        // Request STOP before consuming the last byte, matching the RA RIIC
        // receive flow; reading ICDRR releases the deferred STOP condition.
        write_volatile(ICCR2, SP);
        let id = read_volatile(ICDRR);
        if id != BME280_ID {
            fail(ResultCode::ChipId);
        }
        wait_or_fail(STOP, 0, ResultCode::Stop);

        write_volatile(PORT1_PCNTR3, LED);
        uart(b"UNO_R4_BME280_OK id=0x60\n");
    }
    loop {}
}
