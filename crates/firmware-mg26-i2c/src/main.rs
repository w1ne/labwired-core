#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

//! BRD2709A I²C smoke — reads a TMP102's temperature register over I2C0.
//!
//! Runs on the physical board as well as the twin: every register it touches
//! is real silicon. On the bench it needs a TMP102 wired to the EXP header's
//! I²C pins; in the twin the sensor comes from `system.yaml`.
//!
//! The transaction is the one every register-pointer sensor needs, and the one
//! `Wire.beginTransmission / write / endTransmission / requestFrom` compiles
//! to:
//!
//! 1. START, address with W, write the register pointer (0x00 = temperature).
//! 2. REPEATED START, address with R.
//! 3. Read the MSB, `CMD.ACK` for another byte, read the LSB.
//! 4. `CMD.NACK` + `CMD.STOP`.
//!
//! Register facts (simplicity_sdk sisdk-2025.6, `efr32mg26_i2c.h`,
//! `efr32mg26b510f3200im48.h`):
//!   I2C0_S_BASE 0x4B000000 — ⚠️ in the low-energy group, NOT with I2C1..3 at
//!                            0x400B0000 + n*0x4000.
//!   CMU_CLKEN0.I2C0 bit 14.
//!   EN @0x04, CMD @0x0C, STATE @0x10, STATUS @0x14, CLKDIV @0x18,
//!   RXDATA @0x24, TXDATA @0x34, IF @0x3C.

// ── VCOM console: identical to firmware-mg26-demo, see its header ─────────
const CMU_CLKEN0: *mut u32 = 0x4000_8064 as *mut u32;
const CMU_CLKEN2: *mut u32 = 0x4000_806C as *mut u32;
const CMU_CLKEN0_GPIO: u32 = 1 << 26;
const CMU_CLKEN0_I2C0: u32 = 1 << 14;
const CMU_CLKEN2_USART1: u32 = 1 << 7;

const GPIOB_MODEL: *mut u32 = 0x4003_C064 as *mut u32;
const USART1_ROUTEEN: *mut u32 = 0x4003_C840 as *mut u32;
const USART1_TXROUTE: *mut u32 = 0x4003_C858 as *mut u32;
const GPIO_USART_ROUTEEN_TXPEN: u32 = 1 << 4;
const TXROUTE_PB02: u32 = 1 | (2 << 16);

const USART1_BASE: usize = 0x400A_4000;
const USART1_EN: *mut u32 = (USART1_BASE + 0x04) as *mut u32;
const USART1_CMD: *mut u32 = (USART1_BASE + 0x14) as *mut u32;
const USART1_STATUS: *const u32 = (USART1_BASE + 0x18) as *const u32;
const USART1_CLKDIV: *mut u32 = (USART1_BASE + 0x1C) as *mut u32;
const USART1_TXDATA: *mut u32 = (USART1_BASE + 0x38) as *mut u32;
const USART_CMD_TXEN: u32 = 1 << 2;
const USART_STATUS_TXBL: u32 = 1 << 6;
const USART1_CLKDIV_115200: u32 = 2384;
const MODE_PUSHPULL: u32 = 0x4;
/// Series-2 mode 6 is WIREDANDPULLUP — open-drain with a pull-up, which is
/// what an I2C line is. RM section 24.3.12.1 p.862 says it outright: "an I2C
/// SDA should be configured as open-drain".
const MODE_WIREDANDPULLUP: u32 = 0x6;

// ── GPIO_I2C0 route, RM section 24.6 p.875 ───────────────────────────────
// ⚠️ WITHOUT THESE THE BUS HAS NO WIRES. I2C reaches SCL/SDA only through
// these registers; this firmware used to skip them and "worked" only because
// the window was not even mapped and the twin's model answered anyway.
// Pins are the kit's own: PC05 = QWIIC/MIKROE_I2C_SCL, PC07 = ..._SDA
// (UG594 Table 3.1 p.10, pads 11 and 9). PORT is PA=0, PB=1, PC=2.
const GPIOC_MODEL: *mut u32 = 0x4003_C094 as *mut u32;
const I2C0_ROUTEEN: *mut u32 = 0x4003_C528 as *mut u32;
const I2C0_SCLROUTE: *mut u32 = 0x4003_C52C as *mut u32;
const I2C0_SDAROUTE: *mut u32 = 0x4003_C530 as *mut u32;
const I2C_ROUTEEN_SCLPEN: u32 = 1 << 0;
const I2C_ROUTEEN_SDAPEN: u32 = 1 << 1;

// ── I2C0 ─────────────────────────────────────────────────────────────────
const I2C0_BASE: usize = 0x4B00_0000;
const I2C_EN: *mut u32 = (I2C0_BASE + 0x04) as *mut u32;
const I2C_CMD: *mut u32 = (I2C0_BASE + 0x0C) as *mut u32;
const I2C_STATUS: *const u32 = (I2C0_BASE + 0x14) as *const u32;
const I2C_CLKDIV: *mut u32 = (I2C0_BASE + 0x18) as *mut u32;
const I2C_RXDATA: *const u32 = (I2C0_BASE + 0x24) as *const u32;
const I2C_TXDATA: *mut u32 = (I2C0_BASE + 0x34) as *mut u32;
const I2C_IF: *mut u32 = (I2C0_BASE + 0x3C) as *mut u32;

const I2C_EN_EN: u32 = 1 << 0;
const I2C_CMD_START: u32 = 1 << 0;
const I2C_CMD_STOP: u32 = 1 << 1;
const I2C_CMD_ACK: u32 = 1 << 2;
const I2C_CMD_NACK: u32 = 1 << 3;
const I2C_STATUS_RXDATAV: u32 = 1 << 8;
const I2C_IF_ACK: u32 = 1 << 6;
const I2C_IF_NACK: u32 = 1 << 7;

/// TMP102 at its default address (ADD0 = GND).
const TMP102_ADDR: u32 = 0x48;
/// Pointer 0x00 selects the temperature register.
const TMP102_REG_TEMP: u32 = 0x00;

/// Bound every poll: a bus that never answers means the sensor is not wired or
/// the controller is not clocked, and a printed failure says more than a hang.
const POLL_LIMIT: u32 = 100_000;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn read_u32(p: *const u32) -> u32 {
    unsafe { core::ptr::read_volatile(p) }
}

fn write_u32(p: *mut u32, v: u32) {
    unsafe { core::ptr::write_volatile(p, v) }
}

fn putc(c: u8) {
    unsafe {
        while core::ptr::read_volatile(USART1_STATUS) & USART_STATUS_TXBL == 0 {}
        core::ptr::write_volatile(USART1_TXDATA, c as u32);
    }
}

fn puts(s: &str) {
    for &b in s.as_bytes() {
        putc(b);
    }
}

fn puthex16(v: u32) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for i in (0..4).rev() {
        putc(HEX[((v >> (i * 4)) & 0xF) as usize]);
    }
}

/// Wait for one of `mask`'s flags, returning the flags seen or `None` on
/// timeout. Flags are cleared before returning so the next step starts clean.
fn wait_flags(mask: u32) -> Option<u32> {
    for _ in 0..POLL_LIMIT {
        let f = read_u32(I2C_IF as *const u32);
        if f & mask != 0 {
            write_u32(I2C_IF, f & mask);
            return Some(f & mask);
        }
    }
    None
}

fn main() -> ! {
    write_u32(CMU_CLKEN0, CMU_CLKEN0_GPIO | CMU_CLKEN0_I2C0);
    write_u32(CMU_CLKEN2, CMU_CLKEN2_USART1);
    write_u32(GPIOB_MODEL, MODE_PUSHPULL << 8);
    write_u32(USART1_TXROUTE, TXROUTE_PB02);
    write_u32(USART1_ROUTEEN, GPIO_USART_ROUTEEN_TXPEN);
    write_u32(USART1_EN, 1);
    write_u32(USART1_CLKDIV, USART1_CLKDIV_115200);
    write_u32(USART1_CMD, USART_CMD_TXEN);

    puts("MG26-I2C\n");

    // Both wires open-drain with a pull-up, then routed and enabled. One wire
    // alone is not a bus, and the twin refuses the transfer if only one is up.
    write_u32(
        GPIOC_MODEL,
        (MODE_WIREDANDPULLUP << 20) | (MODE_WIREDANDPULLUP << 28),
    );
    write_u32(I2C0_SCLROUTE, 2 | (5 << 16));
    write_u32(I2C0_SDAROUTE, 2 | (7 << 16));
    write_u32(I2C0_ROUTEEN, I2C_ROUTEEN_SCLPEN | I2C_ROUTEEN_SDAPEN);

    write_u32(I2C_EN, I2C_EN_EN);
    // Any non-zero divisor: the model does not pace the bus, and on silicon
    // this is the standard-mode value emlib computes for a 78 MHz PCLK.
    write_u32(I2C_CLKDIV, 0xFF);
    write_u32(I2C_IF, 0xFFFF_FFFF);

    // 1. Point the sensor at its temperature register.
    write_u32(I2C_CMD, I2C_CMD_START);
    write_u32(I2C_TXDATA, TMP102_ADDR << 1);
    match wait_flags(I2C_IF_ACK | I2C_IF_NACK) {
        Some(f) if f & I2C_IF_NACK != 0 => {
            puts("i2c nack: no sensor at 0x48\n");
            loop {}
        }
        None => {
            puts("i2c timeout\n");
            loop {}
        }
        _ => {}
    }
    write_u32(I2C_TXDATA, TMP102_REG_TEMP);
    wait_flags(I2C_IF_ACK | I2C_IF_NACK);

    // 2. Repeated START, this time to read.
    write_u32(I2C_CMD, I2C_CMD_START);
    write_u32(I2C_TXDATA, (TMP102_ADDR << 1) | 1);
    if wait_flags(I2C_IF_ACK | I2C_IF_NACK).is_none() {
        puts("i2c timeout\n");
        loop {}
    }

    // 3. Two bytes, big-endian, ACK between them.
    let mut spins = 0;
    while read_u32(I2C_STATUS) & I2C_STATUS_RXDATAV == 0 {
        spins += 1;
        if spins >= POLL_LIMIT {
            puts("i2c no data\n");
            loop {}
        }
    }
    let msb = read_u32(I2C_RXDATA) & 0xFF;
    write_u32(I2C_CMD, I2C_CMD_ACK);
    let lsb = read_u32(I2C_RXDATA) & 0xFF;

    // 4. Release the bus.
    write_u32(I2C_CMD, I2C_CMD_NACK | I2C_CMD_STOP);

    puts("temp=");
    puthex16((msb << 8) | lsb);
    putc(b'\n');

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
