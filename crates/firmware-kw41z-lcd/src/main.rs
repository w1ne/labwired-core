// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
//
// KW41Z "cattle activity tag" display firmware. Reads the on-board FXOS8700
// accelerometer over the Kinetis I2C (I2C1) and renders a live 3-axis activity
// bar-graph onto a Nokia-5110 (PCD8544) LCD over the Kinetis DSPI (SPI0), with
// the display's D/C line driven from a GPIOC output. Also echoes the readings
// over LPUART0 so the boot is observable as text.
//
// Boots on the LabWired KW41Z model unchanged: the behavioural Kinetis I2C /
// DSPI / GPIO peripherals route exactly the register pokes below, and the
// PCD8544 device model renders the framebuffer the firmware streams out.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

// ── LPUART0 (console) ────────────────────────────────────────────────────────
const LPUART0_STAT: *mut u32 = 0x4005_4004 as *mut u32;
const LPUART0_CTRL: *mut u32 = 0x4005_4008 as *mut u32;
const LPUART0_DATA: *mut u32 = 0x4005_400C as *mut u32;
const STAT_TDRE: u32 = 1 << 23;
const CTRL_TE: u32 = 1 << 19;

// ── I2C1 (FXOS8700 @ 0x1f) — byte registers ──────────────────────────────────
const I2C1_C1: *mut u8 = 0x4006_7002 as *mut u8;
const I2C1_S: *mut u8 = 0x4006_7003 as *mut u8;
const I2C1_D: *mut u8 = 0x4006_7004 as *mut u8;
const C1_IICEN: u8 = 0x80;
const C1_MST: u8 = 0x20;
const C1_TX: u8 = 0x10;
const C1_TXAK: u8 = 0x08;
const C1_RSTA: u8 = 0x04;
const S_IICIF: u8 = 0x02;

const FXOS_ADDR: u8 = 0x1f;
const FXOS_WHOAMI: u8 = 0x0D;
const FXOS_OUT_X_MSB: u8 = 0x01;
const FXOS_CTRL_REG1: u8 = 0x2A;

// ── DSPI / SPI0 (PCD8544) — 32-bit registers ─────────────────────────────────
const SPI0_MCR: *mut u32 = 0x4002_C000 as *mut u32;
const SPI0_SR: *mut u32 = 0x4002_C02C as *mut u32;
const SPI0_PUSHR: *mut u32 = 0x4002_C034 as *mut u32;
const MCR_MSTR: u32 = 0x8000_0000;
const SR_TFFF: u32 = 0x0200_0000;
const SR_TCF: u32 = 0x8000_0000;

// ── GPIOC (PCD8544 D/C line = PTC0) — 32-bit PDOR ─────────────────────────────
const GPIOC_PDOR: *mut u32 = 0x400F_F080 as *mut u32;
const DC_BIT: u32 = 1 << 0;

const LCD_W: usize = 84;
const LCD_BANKS: usize = 6;

#[entry]
fn main() -> ! {
    unsafe {
        write_volatile(LPUART0_CTRL, CTRL_TE);
        write_volatile(SPI0_MCR, MCR_MSTR); // DSPI master, HALT cleared

        // Probe the sensor, then put it in active mode (the model returns data
        // regardless, but this is the real bring-up a driver does).
        let who = i2c_read_one(FXOS_ADDR, FXOS_WHOAMI);
        i2c_write_one(FXOS_ADDR, FXOS_CTRL_REG1, 0x01); // ACTIVE

        pcd8544_init();

        print(b"KW41Z_LCD_OK whoami=");
        print_hex(who);
        print(b"\n");

        loop {
            // Read the 6 accel bytes (X/Y/Z MSB:LSB), 14-bit left-justified.
            let mut raw = [0u8; 6];
            i2c_read(FXOS_ADDR, FXOS_OUT_X_MSB, &mut raw);
            let ax = ((raw[0] as i16) << 8 | raw[1] as i16) >> 2;
            let ay = ((raw[2] as i16) << 8 | raw[3] as i16) >> 2;
            let az = ((raw[4] as i16) << 8 | raw[5] as i16) >> 2;

            render_activity(ax, ay, az);

            print(b"X=");
            print_dec(ax);
            print(b" Y=");
            print_dec(ay);
            print(b" Z=");
            print_dec(az);
            print(b"\n");

            // Coarse pacing so the trace shows a few frames.
            for _ in 0..2000 {
                let _ = read_volatile(LPUART0_STAT);
            }
        }
    }
}

// ── PCD8544 ───────────────────────────────────────────────────────────────────

unsafe fn dc(level: bool) {
    write_volatile(GPIOC_PDOR, if level { DC_BIT } else { 0 });
}

unsafe fn spi_send(byte: u8) {
    while read_volatile(SPI0_SR) & SR_TFFF == 0 {}
    write_volatile(SPI0_PUSHR, byte as u32);
    while read_volatile(SPI0_SR) & SR_TCF == 0 {}
    write_volatile(SPI0_SR, SR_TCF); // clear TCF
}

unsafe fn pcd_cmd(c: u8) {
    dc(false);
    spi_send(c);
}

unsafe fn pcd_data(d: u8) {
    dc(true);
    spi_send(d);
}

unsafe fn pcd8544_init() {
    pcd_cmd(0x21); // function set: extended
    pcd_cmd(0xB1); // set Vop (contrast)
    pcd_cmd(0x04); // temperature coefficient
    pcd_cmd(0x14); // bias system
    pcd_cmd(0x20); // function set: basic
    pcd_cmd(0x0C); // display control: normal mode
}

/// Draw a 3-axis horizontal activity bar-graph. Each axis gets two of the six
/// banks; the bar length is the axis magnitude scaled to the 84-px width, so an
/// idle tag shows a tall Z bar (~1 g) and motion lengthens X/Y.
unsafe fn render_activity(ax: i16, ay: i16, az: i16) {
    pcd_cmd(0x40); // Y = 0
    pcd_cmd(0x80); // X = 0
    let bar = |v: i16| -> usize {
        let m = (v.unsigned_abs() as usize) >> 4; // 14-bit → ~0..127, then clamp
        if m > LCD_W {
            LCD_W
        } else {
            m
        }
    };
    let axes = [bar(ax), bar(ay), bar(az)];
    for bank in 0..LCD_BANKS {
        let len = axes[bank / 2];
        // Solid bar on the top row of the bank's first sub-line.
        let fill: u8 = if bank % 2 == 0 { 0x7E } else { 0x00 };
        for x in 0..LCD_W {
            pcd_data(if x < len { fill } else { 0x00 });
        }
    }
}

// ── Kinetis I2C ────────────────────────────────────────────────────────────────

unsafe fn i2c_wait_clear() {
    while read_volatile(I2C1_S) & S_IICIF == 0 {}
    write_volatile(I2C1_S, S_IICIF);
}

unsafe fn i2c_read(addr: u8, reg: u8, buf: &mut [u8]) {
    // START + write register pointer.
    write_volatile(I2C1_C1, C1_IICEN | C1_MST | C1_TX);
    write_volatile(I2C1_D, addr << 1);
    i2c_wait_clear();
    write_volatile(I2C1_D, reg);
    i2c_wait_clear();
    // Repeated START + read address.
    write_volatile(I2C1_C1, C1_IICEN | C1_MST | C1_TX | C1_RSTA);
    write_volatile(I2C1_D, (addr << 1) | 1);
    i2c_wait_clear();
    // Enter receive. NAK immediately if a single byte is requested.
    let n = buf.len();
    if n <= 1 {
        write_volatile(I2C1_C1, C1_IICEN | C1_MST | C1_TXAK);
    } else {
        write_volatile(I2C1_C1, C1_IICEN | C1_MST);
    }
    let _ = read_volatile(I2C1_D); // dummy read kicks the first byte
    i2c_wait_clear();
    for i in 0..n {
        if i == n - 1 {
            // STOP before the final byte read.
            write_volatile(I2C1_C1, C1_IICEN);
        } else if i == n - 2 {
            // NAK the byte after this one (the last).
            write_volatile(I2C1_C1, C1_IICEN | C1_MST | C1_TXAK);
        }
        buf[i] = read_volatile(I2C1_D);
        if i != n - 1 {
            i2c_wait_clear();
        }
    }
}

unsafe fn i2c_read_one(addr: u8, reg: u8) -> u8 {
    let mut b = [0u8; 1];
    i2c_read(addr, reg, &mut b);
    b[0]
}

unsafe fn i2c_write_one(addr: u8, reg: u8, val: u8) {
    write_volatile(I2C1_C1, C1_IICEN | C1_MST | C1_TX);
    write_volatile(I2C1_D, addr << 1);
    i2c_wait_clear();
    write_volatile(I2C1_D, reg);
    i2c_wait_clear();
    write_volatile(I2C1_D, val);
    i2c_wait_clear();
    write_volatile(I2C1_C1, C1_IICEN); // STOP
}

// ── Console helpers ─────────────────────────────────────────────────────────

unsafe fn print(s: &[u8]) {
    for &b in s {
        while read_volatile(LPUART0_STAT) & STAT_TDRE == 0 {}
        write_volatile(LPUART0_DATA, b as u32);
    }
}

unsafe fn print_hex(v: u8) {
    let hex = b"0123456789ABCDEF";
    print(&[hex[(v >> 4) as usize], hex[(v & 0xF) as usize]]);
}

unsafe fn print_dec(v: i16) {
    let mut n = v;
    if n < 0 {
        print(b"-");
        n = -n;
    }
    let mut buf = [0u8; 6];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    print(&buf[i..]);
}
