#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

const USART2_DR_PTR: *mut u8 = (0x4000_4400 + 0x04) as *mut u8;

const I2C1_CR1: *mut u8 = 0x4000_5400 as *mut u8;
const I2C1_OAR1: *mut u8 = 0x4000_5408 as *mut u8;
const I2C1_DR: *mut u8 = 0x4000_5410 as *mut u8;
const I2C1_SR1: *const u8 = 0x4000_5414 as *const u8;
const I2C1_SR2: *const u8 = 0x4000_5418 as *const u8;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn main() -> ! {
    let ok = smoke_i2c1();
    if ok {
        write_uart(b"I2C_OK\n");
    } else {
        write_uart(b"I2C_FAIL\n");
    }

    loop {}
}

fn smoke_i2c1() -> bool {
    unsafe {
        core::ptr::write_volatile(I2C1_OAR1, 0xAA);
        core::ptr::write_volatile(I2C1_OAR1.add(1), 0x55);
        if core::ptr::read_volatile(I2C1_OAR1) != 0xAA {
            return false;
        }
        if core::ptr::read_volatile(I2C1_OAR1.add(1)) != 0x55 {
            return false;
        }

        // CR1 START is bit 8, so write the high byte at CR1 + 1.
        core::ptr::write_volatile(I2C1_CR1.add(1), 0x01);
        if !wait_sr1(0x01) {
            return false;
        }

        core::ptr::write_volatile(I2C1_DR, 0xA0);
        if !wait_sr1(0x02) {
            return false;
        }
        let _ = core::ptr::read_volatile(I2C1_SR2);

        core::ptr::write_volatile(I2C1_DR, 0x42);
        if !wait_sr1(0x80) || !wait_sr1(0x04) {
            return false;
        }

        // CR1 STOP is bit 9, so write the high byte at CR1 + 1.
        core::ptr::write_volatile(I2C1_CR1.add(1), 0x02);
        wait_sr1_clear(0x86)
    }
}

fn wait_sr1(mask: u8) -> bool {
    for _ in 0..256 {
        let value = unsafe { core::ptr::read_volatile(I2C1_SR1) };
        if (value & mask) == mask {
            return true;
        }
    }
    false
}

fn wait_sr1_clear(mask: u8) -> bool {
    for _ in 0..256 {
        let value = unsafe { core::ptr::read_volatile(I2C1_SR1) };
        if (value & mask) == 0 {
            return true;
        }
    }
    false
}

fn write_uart(bytes: &[u8]) {
    for byte in bytes {
        unsafe {
            core::ptr::write_volatile(USART2_DR_PTR, *byte);
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
