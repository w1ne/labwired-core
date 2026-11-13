// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! MIMXRT595-EVK smoke firmware (Cortex-M33).
//!
//! The RT500 boot ROM reads the FlexSPI configuration block (FCB, tag
//! "FCFB") at 0x08000400 and starts the image through the vector table at
//! 0x08001000 (SDK evkmimxrt595 flash linker file). This image carries an FCB
//! header with that tag; the simulator does not parse it.
//!
//! Bring-up, with the register writes the SDK drivers make (MIMXRT595S_cm33.h
//! layouts, fsl_clock / fsl_usart / fsl_gpio / fsl_iopctl):
//! 1. CLKCTL0: FRO DIV = FRO / 4 (FRODIVSEL = 1), MAINCLKSELA = FRO DIV,
//!    MAINCLKSELB = MAINCLKSELA, so the core runs at 48 MHz.
//! 2. CLKCTL1/RSTCTL1: FLEXCOMM0 and HSGPIO0/1/3 clocks on through the
//!    PSCCTLn_SET aliases, out of reset through the PRSTCTLn_CLR aliases.
//! 3. CLKCTL1 FLEXCOMM[0].FCFCLKSEL = FRO_DIV4 (48 MHz).
//! 4. IOPCTL: P0_1/P0_2 to function 1 (FLEXCOMM0 TXD/RXD), the LED pads to
//!    GPIO, the switch pads to GPIO with the input buffer on.
//! 5. FLEXCOMM0: PSELID = USART, 8N1, OSR 16x, BRG 26 -> 48 MHz / (16 * 26)
//!    = 115384 baud, FIFOs on.
//! 6. The RGB LED (P0_14 red, P1_0 green, P3_17 blue) is walked through the
//!    SET, byte (B), word (W), NOT, MPIN/MASK and CLR views with PIN
//!    readback; SW1 (P0_25) and SW2 (P0_10) are read as inputs.

#![no_std]
#![no_main]

use core::ptr::{read_volatile, write_volatile};
use cortex_m_rt::entry;
use panic_halt as _;

/// FlexSPI configuration block header at 0x08000400: tag "FCFB", version
/// 1.4.0 (`flexspi_nor_config_t.memConfig.tag` / `.version`). The rest of the
/// 512-byte block (LUTs, serial clock, device size) is left erased.
#[link_section = ".fcb.rt500"]
#[used]
static FCB: [u32; 128] = {
    let mut fcb = [0xFFFF_FFFFu32; 128];
    fcb[0] = FCB_TAG;
    fcb[1] = 0x5601_0400; // version V1.4.0 ('V' in the top byte)
    fcb
};

const FCB_BASE: u32 = 0x0800_0400;
const FCB_TAG: u32 = 0x4246_4346; // "FCFB"
const SCB_VTOR: *const u32 = 0xE000_ED08 as *const u32;

// CLKCTL0 @ 0x40001000.
const CLKCTL0: u32 = 0x4000_1000;
const FRODIVSEL: u32 = 0x108;
const MAINCLKSELA: u32 = 0x430;
const MAINCLKSELB: u32 = 0x434;

// CLKCTL1 @ 0x40021000 / RSTCTL1 @ 0x40020000.
const CLKCTL1: u32 = 0x4002_1000;
const RSTCTL1: u32 = 0x4002_0000;
const PSCCTL0: u32 = 0x10;
const PSCCTL1: u32 = 0x14;
const PSCCTL0_SET: u32 = 0x40;
const PSCCTL1_SET: u32 = 0x44;
const PRSTCTL0_CLR: u32 = 0x70;
const PRSTCTL1_CLR: u32 = 0x74;
const FC0FCLKSEL: u32 = 0x508;
const FC0_BIT: u32 = 1 << 8; // PSCCTL0.FC0_CLK / PRSTCTL0.FLEXCOMM0
const HSGPIO_BITS: u32 = 0b1011; // HSGPIO0, HSGPIO1, HSGPIO3

// IOPCTL @ 0x40004000: PIO[port][pin] @ 0x80 * port + 4 * pin.
const IOPCTL: u32 = 0x4000_4000;
const PIO_FSEL1: u32 = 1;
const PIO_IBENA: u32 = 1 << 6;

// FLEXCOMM0 / USART0 @ 0x40106000.
const FC0: u32 = 0x4010_6000;
const CFG: u32 = 0x000;
const BRG: u32 = 0x020;
const OSR: u32 = 0x028;
const FIFOCFG: u32 = 0xE00;
const FIFOSTAT: u32 = 0xE04;
const FIFOWR: u32 = 0xE20;
const FIFOSIZE: u32 = 0xE48;
const PSELID: u32 = 0xFF8;
const CFG_ENABLE: u32 = 1 << 0;
const CFG_DATALEN_8: u32 = 1 << 2;
const FIFOCFG_ENABLETX: u32 = 1 << 0;
const FIFOCFG_ENABLERX: u32 = 1 << 1;
const FIFOSTAT_TXNOTFULL: u32 = 1 << 5;
const OSR_VAL: u32 = 15; // 16x
const BRG_VAL: u32 = 25; // divide by 26
const FC_CLK_HZ: u32 = 48_000_000;

// GPIO @ 0x40100000.
const GPIO: u32 = 0x4010_0000;
const MASK: u32 = 0x2080;
const PIN: u32 = 0x2100;
const MPIN: u32 = 0x2180;
const SET: u32 = 0x2200;
const CLR: u32 = 0x2280;
const NOT: u32 = 0x2300;
const DIRSET: u32 = 0x2380;
const DIRCLR: u32 = 0x2400;

const RED: (u32, u32) = (0, 14);
const GREEN: (u32, u32) = (1, 0);
const BLUE: (u32, u32) = (3, 17);
const SW1: (u32, u32) = (0, 25);
const SW2: (u32, u32) = (0, 10);

fn rd(base: u32, offset: u32) -> u32 {
    unsafe { read_volatile((base + offset) as *const u32) }
}

fn wr(base: u32, offset: u32, value: u32) {
    unsafe { write_volatile((base + offset) as *mut u32, value) }
}

fn port_reg(reg: u32, port: u32) -> u32 {
    reg + 4 * port
}

fn pio(port: u32, pin: u32) -> u32 {
    0x80 * port + 4 * pin
}

fn pin_level(pad: (u32, u32)) -> u32 {
    (rd(GPIO, port_reg(PIN, pad.0)) >> pad.1) & 1
}

fn putc(byte: u8) {
    while rd(FC0, FIFOSTAT) & FIFOSTAT_TXNOTFULL == 0 {}
    wr(FC0, FIFOWR, byte as u32);
}

fn puts(text: &str) {
    for byte in text.bytes() {
        putc(byte);
    }
}

fn put_hex(value: u32, digits: u32) {
    for shift in (0..digits).rev() {
        let nibble = ((value >> (shift * 4)) & 0xF) as u8;
        putc(match nibble {
            0..=9 => b'0' + nibble,
            _ => b'a' + nibble - 10,
        });
    }
}

fn put_dec32(value: u32) {
    let mut digits = [0u8; 10];
    let mut remaining = value;
    let mut start = digits.len();
    loop {
        start -= 1;
        digits[start] = b'0' + (remaining % 10) as u8;
        remaining /= 10;
        if remaining == 0 {
            break;
        }
    }
    for &digit in &digits[start..] {
        putc(digit);
    }
}

#[inline(never)]
fn spin(iterations: u32) {
    for _ in 0..iterations {
        unsafe {
            core::arch::asm!("nop", options(nomem, nostack, preserves_flags));
        }
    }
}

fn report_leds(label: &str) {
    puts(label);
    puts(" r=");
    put_dec32(pin_level(RED));
    puts(" g=");
    put_dec32(pin_level(GREEN));
    puts(" b=");
    put_dec32(pin_level(BLUE));
    puts(" pin0=");
    put_hex(rd(GPIO, port_reg(PIN, 0)), 8);
    putc(b'\n');
}

#[entry]
fn main() -> ! {
    // 1. Main clock: FRO DIV (FRO / 4) on MAINCLKSELA, MAINCLKSELB = A.
    wr(CLKCTL0, FRODIVSEL, 1);
    wr(CLKCTL0, MAINCLKSELA, 1);
    wr(CLKCTL0, MAINCLKSELB, 0);

    // 2. Clock gates on, resets released.
    wr(CLKCTL1, PSCCTL0_SET, FC0_BIT);
    wr(CLKCTL1, PSCCTL1_SET, HSGPIO_BITS);
    wr(RSTCTL1, PRSTCTL0_CLR, FC0_BIT);
    wr(RSTCTL1, PRSTCTL1_CLR, HSGPIO_BITS);

    // 3. FLEXCOMM0 function clock: FRO_DIV4.
    wr(CLKCTL1, FC0FCLKSEL, 0);

    // 4. Pads.
    wr(IOPCTL, pio(0, 1), PIO_FSEL1);
    wr(IOPCTL, pio(0, 2), PIO_FSEL1 | PIO_IBENA);
    for &(port, pin) in [RED, GREEN, BLUE].iter() {
        wr(IOPCTL, pio(port, pin), 0);
    }
    for &(port, pin) in [SW1, SW2].iter() {
        wr(IOPCTL, pio(port, pin), PIO_IBENA);
    }

    // 5. FLEXCOMM0 as USART, 8N1 at 115384 baud.
    wr(FC0, PSELID, 1);
    wr(FC0, CFG, 0);
    wr(FC0, OSR, OSR_VAL);
    wr(FC0, BRG, BRG_VAL);
    wr(FC0, FIFOCFG, FIFOCFG_ENABLETX | FIFOCFG_ENABLERX);
    wr(FC0, CFG, CFG_ENABLE | CFG_DATALEN_8);

    puts("MIMXRT595 EVK SMOKE\n");
    puts("cpu=cortex-m33 uart=flexcomm0 pselid=");
    put_hex(rd(FC0, PSELID), 8);
    puts(" fifosize=");
    put_dec32(rd(FC0, FIFOSIZE));
    putc(b'\n');
    puts("fcb tag=");
    put_hex(rd(FCB_BASE, 0), 8);
    puts(" vtor=");
    put_hex(unsafe { read_volatile(SCB_VTOR) }, 8);
    putc(b'\n');
    puts("clk frodivsel=");
    put_dec32(rd(CLKCTL0, FRODIVSEL));
    puts(" mainclksela=");
    put_dec32(rd(CLKCTL0, MAINCLKSELA));
    puts(" pscctl0=");
    put_hex(rd(CLKCTL1, PSCCTL0), 8);
    puts(" pscctl1=");
    put_hex(rd(CLKCTL1, PSCCTL1), 8);
    putc(b'\n');
    puts("uart fclk=");
    put_dec32(FC_CLK_HZ);
    puts(" osr=");
    put_dec32(OSR_VAL + 1);
    puts(" brg=");
    put_dec32(BRG_VAL + 1);
    puts(" baud=");
    put_dec32(FC_CLK_HZ / ((OSR_VAL + 1) * (BRG_VAL + 1)));
    putc(b'\n');

    // 6. LEDs: outputs, all off, then walked through each GPIO view.
    for &(port, pin) in [RED, GREEN, BLUE].iter() {
        wr(GPIO, port_reg(CLR, port), 1 << pin);
        wr(GPIO, port_reg(DIRSET, port), 1 << pin);
    }
    report_leds("leds=off");
    wr(GPIO, port_reg(SET, RED.0), 1 << RED.1);
    report_leds("set red");
    unsafe { write_volatile((GPIO + 0x20 * GREEN.0 + GREEN.1) as *mut u8, 1) };
    report_leds("byte green");
    wr(GPIO, 0x1000 + 0x80 * BLUE.0 + 4 * BLUE.1, 0xFFFF_FFFF);
    report_leds("word blue");
    wr(GPIO, port_reg(NOT, RED.0), 1 << RED.1);
    report_leds("not red");
    // MPIN under MASK: only unmasked (0) bits are written; leave RED free.
    wr(GPIO, port_reg(MASK, RED.0), !(1 << RED.1));
    wr(GPIO, port_reg(MPIN, RED.0), 0xFFFF_FFFF);
    wr(GPIO, port_reg(MASK, RED.0), 0);
    report_leds("mpin red");
    wr(GPIO, port_reg(CLR, GREEN.0), 1 << GREEN.1);
    wr(GPIO, port_reg(CLR, BLUE.0), 1 << BLUE.1);
    wr(GPIO, port_reg(CLR, RED.0), 1 << RED.1);
    report_leds("clr all");

    // Switches: inputs; released reads 1.
    wr(GPIO, port_reg(DIRCLR, 0), (1 << SW1.1) | (1 << SW2.1));
    puts("sw1=");
    put_dec32(pin_level(SW1));
    puts(" sw2=");
    put_dec32(pin_level(SW2));
    putc(b'\n');

    let mut heartbeat: u32 = 0;
    loop {
        // Toggle the green LED (P1_0) through NOT[1] each beat.
        wr(GPIO, port_reg(NOT, GREEN.0), 1 << GREEN.1);
        puts("hb=");
        put_dec32(heartbeat);
        puts(" green=");
        put_dec32(pin_level(GREEN));
        putc(b'\n');
        spin(100_000);
        heartbeat = heartbeat.wrapping_add(1);
    }
}
