#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

//! BRD2709A SPI smoke — clocks a MAX31855's 32-bit frame out of USART0.
//!
//! Runs on the physical board as well as the twin: every register is real
//! silicon.
//!
//! ⚠️ Series 2 has NO separate SPI peripheral. SPI is a USART with
//! `CTRL.SYNC` set and `CMD.MASTEREN` issued — the same block the VCOM console
//! uses, which is why this uses USART0 and leaves USART1 to the console.
//!
//! Register facts (simplicity_sdk sisdk-2025.6, `efr32mg26_usart.h`,
//! `efr32mg26b510f3200im48.h`):
//!   USART0_S_BASE 0x400A0000, CMU_CLKEN0.USART0 bit 9.
//!   EN @0x04, CTRL @0x08 (SYNC bit 0), CMD @0x14 (RXEN 0, TXEN 2,
//!   MASTEREN 4), STATUS @0x18 (TXC 5, TXBL 6, RXDATAV 7), CLKDIV @0x1C,
//!   RXDATA @0x24, TXDATA @0x3C.

const CMU_CLKEN0: *mut u32 = 0x4000_8064 as *mut u32;
const CMU_CLKEN2: *mut u32 = 0x4000_806C as *mut u32;
const CMU_CLKEN0_GPIO: u32 = 1 << 26;
const CMU_CLKEN0_USART0: u32 = 1 << 9;
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
const MODE_INPUT: u32 = 0x1;

// ── GPIO_USARTROUTE[0], RM section 24.6 p.879 ────────────────────────────
const GPIOC_MODEL: *mut u32 = 0x4003_C094 as *mut u32;
const USART0_ROUTEEN: *mut u32 = 0x4003_C820 as *mut u32;
const USART0_RXROUTE: *mut u32 = 0x4003_C830 as *mut u32;
const USART0_CLKROUTE: *mut u32 = 0x4003_C834 as *mut u32;
const USART0_TXROUTE: *mut u32 = 0x4003_C838 as *mut u32;
const ROUTEEN_RXPEN: u32 = 1 << 2;
const ROUTEEN_CLKPEN: u32 = 1 << 3;
const ROUTEEN_TXPEN: u32 = 1 << 4;

// ── USART0 in synchronous (SPI) mode ─────────────────────────────────────
const SPI0_BASE: usize = 0x400A_0000;
const SPI_EN: *mut u32 = (SPI0_BASE + 0x04) as *mut u32;
const SPI_CTRL: *mut u32 = (SPI0_BASE + 0x08) as *mut u32;
const SPI_CMD: *mut u32 = (SPI0_BASE + 0x14) as *mut u32;
const SPI_STATUS: *const u32 = (SPI0_BASE + 0x18) as *const u32;
const SPI_CLKDIV: *mut u32 = (SPI0_BASE + 0x1C) as *mut u32;
const SPI_RXDATA: *const u32 = (SPI0_BASE + 0x24) as *const u32;
const SPI_TXDATA: *mut u32 = (SPI0_BASE + 0x3C) as *mut u32;

const SPI_EN_EN: u32 = 1 << 0;
const SPI_CTRL_SYNC: u32 = 1 << 0;
const SPI_CMD_RXEN: u32 = 1 << 0;
const SPI_CMD_TXEN: u32 = 1 << 2;
const SPI_CMD_MASTEREN: u32 = 1 << 4;
const SPI_STATUS_TXC: u32 = 1 << 5;

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

fn puthex8(v: u32) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    putc(HEX[((v >> 4) & 0xF) as usize]);
    putc(HEX[(v & 0xF) as usize]);
}

/// Clock one byte out and return what came back on MISO.
fn spi_transfer(byte: u32) -> u32 {
    write_u32(SPI_TXDATA, byte);
    let mut spins = 0;
    while read_u32(SPI_STATUS) & SPI_STATUS_TXC == 0 {
        spins += 1;
        if spins >= POLL_LIMIT {
            return 0xFF;
        }
    }
    read_u32(SPI_RXDATA) & 0xFF
}

fn main() -> ! {
    write_u32(CMU_CLKEN0, CMU_CLKEN0_GPIO | CMU_CLKEN0_USART0);
    write_u32(CMU_CLKEN2, CMU_CLKEN2_USART1);
    write_u32(GPIOB_MODEL, MODE_PUSHPULL << 8);
    write_u32(USART1_TXROUTE, TXROUTE_PB02);
    write_u32(USART1_ROUTEEN, GPIO_USART_ROUTEEN_TXPEN);
    write_u32(USART1_EN, 1);
    write_u32(USART1_CLKDIV, USART1_CLKDIV_115200);
    write_u32(USART1_CMD, USART_CMD_TXEN);

    puts("MG26-SPI\n");

    // ⚠️ ROUTE THE PINS, OR THIS CLOCKS NOTHING. On Series 2 a USART's signals
    // reach NO pad until GPIO_USARTROUTE says which one. This firmware used to
    // skip that and "worked" only because the twin's route block was a stub —
    // on a real BRD2709A it drove a dead bus.
    //
    // Pins are the kit's mikroBUS mapping, UG594 Table 3.1 p.10: PC03 SCK,
    // PC02 MOSI, PC01 MISO. Registers are RM section 24.6 p.879 and the route
    // word's PORT[1:0] / PIN[19:16] (p.1091-93); PORT is PA=0, PB=1, PC=2
    // (RM section 24.3.12.1 p.862).
    write_u32(
        GPIOC_MODEL,
        (MODE_INPUT << 4) | (MODE_PUSHPULL << 8) | (MODE_PUSHPULL << 12),
    );
    write_u32(USART0_RXROUTE, 2 | (1 << 16)); // MISO <- PC01
    write_u32(USART0_CLKROUTE, 2 | (3 << 16)); // SCLK -> PC03
    write_u32(USART0_TXROUTE, 2 | (2 << 16)); // MOSI -> PC02
    write_u32(
        USART0_ROUTEEN,
        ROUTEEN_TXPEN | ROUTEEN_CLKPEN | ROUTEEN_RXPEN,
    );

    // Synchronous master: SYNC in CTRL, then MASTEREN/TXEN/RXEN in CMD.
    write_u32(SPI_EN, SPI_EN_EN);
    write_u32(SPI_CTRL, SPI_CTRL_SYNC);
    write_u32(SPI_CLKDIV, 0xFF);
    write_u32(SPI_CMD, SPI_CMD_MASTEREN | SPI_CMD_TXEN | SPI_CMD_RXEN);

    // The MAX31855 is read-only: four dummy bytes clock its 32-bit frame out.
    puts("frame=");
    for _ in 0..4 {
        puthex8(spi_transfer(0x00));
    }
    putc(b'\n');

    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
