#![no_std]
// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.
#![no_main]
#![allow(clippy::empty_loop)]

//! BRD2709A "agent deck" — drives every part of the HEStore order at once.
//!
//! ST7789 panel on USART0 (SPI), INMP441 microphone on USART2 (I2S), a slide
//! fader on IADC0, and five GPIO contacts: the rotary encoder's A/B/switch,
//! a pushbutton module and an SPDT toggle. Console on USART1 (VCOM).
//!
//! Runs on the physical board as well as the twin: every register below is
//! real silicon, taken from simplicity_sdk sisdk-2025.6 (`efr32mg26_usart.h`,
//! `efr32mg26_iadc.h`, `efr32mg26_gpio.h`, `efr32mg26b510f3200im48.h`) and the
//! EFR32xG26 Reference Manual Rev 1.0.
//!
//! ⚠️ TWO USARTS, AND THAT IS NOT A STYLE CHOICE. Series 2 has no separate SPI
//! or I2S peripheral — both are a USART wearing a different hat, and
//! `I2SCTRL.EN` (RM section 20.5.22 p.669) switches the WHOLE block. So the
//! panel and the microphone cannot share one. USART1 is the console, which
//! leaves USART0 for the panel and USART2 for the mic.
//!
//! ⚠️ PD04/PD05 ARE THE PTI PADS and are used here as ordinary GPIO. PTI is a
//! debug tap that copies radio packets to the J-Link; BLE does not need it,
//! and debug runs over SWD on PA01/PA02/PA03 either way. Spending them is what
//! makes the encoder's push switch fit.

// ── CMU ──────────────────────────────────────────────────────────────────
const CMU_CLKEN0: *mut u32 = 0x4000_8064 as *mut u32;
const CMU_CLKEN2: *mut u32 = 0x4000_806C as *mut u32;
/// `_CMU_CLKEN0_GPIO_SHIFT`. All four ports share the one GPIO clock.
const CMU_CLKEN0_GPIO: u32 = 1 << 26;
/// `_CMU_CLKEN0_USART0_SHIFT`.
const CMU_CLKEN0_USART0: u32 = 1 << 9;
/// `_CMU_CLKEN0_IADC0_SHIFT`.
const CMU_CLKEN0_IADC0: u32 = 1 << 10;
/// `_CMU_CLKEN2_USART1_SHIFT`. USART1 is on CLKEN2, not CLKEN0 — the Series-2
/// CLKEN registers are not one flat space.
const CMU_CLKEN2_USART1: u32 = 1 << 7;
/// `_CMU_CLKEN2_USART2_SHIFT`.
const CMU_CLKEN2_USART2: u32 = 1 << 8;

// ── GPIO ports. MODEL @0x04, MODEH @0x08, DOUT @0x10, DIN @0x14. ─────────
const GPIOB_BASE: usize = 0x4003_C060;
const GPIOC_BASE: usize = 0x4003_C090;
const GPIOD_BASE: usize = 0x4003_C0C0;
const GPIOB_MODEL: *mut u32 = (GPIOB_BASE + 0x04) as *mut u32;
const GPIOC_MODEL: *mut u32 = (GPIOC_BASE + 0x04) as *mut u32;
const GPIOD_MODEL: *mut u32 = (GPIOD_BASE + 0x04) as *mut u32;
const GPIOC_DOUT: *mut u32 = (GPIOC_BASE + 0x10) as *mut u32;
const GPIOD_DOUT: *mut u32 = (GPIOD_BASE + 0x10) as *mut u32;
const GPIOC_DIN: *const u32 = (GPIOC_BASE + 0x14) as *const u32;
const GPIOD_DIN: *const u32 = (GPIOD_BASE + 0x14) as *const u32;

const MODE_PUSHPULL: u32 = 0x4;
const MODE_INPUTPULL: u32 = 0x2;

// Deck pin map — UG594 Table 3.1 p.10 and Figure 3.5 p.9.
const PC00_DC: u32 = 1 << 0; //  pad 17, panel D/C
const PC01_BTN: u32 = 1 << 1; //  pad 12, pushbutton module SIG
const PC04_CS: u32 = 1 << 4; //  pad 16, panel chip select
const PC05_ENC_CLK: u32 = 1 << 5; //  pad 11, encoder A
const PC06_RES: u32 = 1 << 6; //  pad 18, panel reset (active LOW)
const PC07_ENC_DT: u32 = 1 << 7; //  pad  9, encoder B
const PD03_BLK: u32 = 1 << 3; //  pad  6, panel backlight
const PD04_ENC_SW: u32 = 1 << 4; //  pad 28, encoder shaft switch
const PD05_TOGGLE: u32 = 1 << 5; //  pad 26, SPDT toggle

// ── USART1: the VCOM console ─────────────────────────────────────────────
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
const USART1_CLKDIV_115200: u32 = 2384;

// ── GPIO_USARTROUTE, RM section 24.6 p.879 ───────────────────────────────
// ⚠️ WITHOUT THESE THE DECK DRIVES NOTHING ON A REAL BOARD. A USART's signals
// reach no pad until its route names one. PORT is PA=0, PB=1, PC=2, PD=3
// (RM section 24.3.12.1 p.862); the route word is PORT[1:0] | PIN[19:16].
const USART0_ROUTEEN: *mut u32 = 0x4003_C820 as *mut u32;
const USART0_CLKROUTE: *mut u32 = 0x4003_C834 as *mut u32;
const USART0_TXROUTE: *mut u32 = 0x4003_C838 as *mut u32;
const USART2_ROUTEEN: *mut u32 = 0x4003_C860 as *mut u32;
const USART2_CSROUTE: *mut u32 = 0x4003_C864 as *mut u32;
const USART2_RXROUTE: *mut u32 = 0x4003_C870 as *mut u32;
const USART2_CLKROUTE: *mut u32 = 0x4003_C874 as *mut u32;
const ROUTEEN_CSPEN: u32 = 1 << 0;
const ROUTEEN_RXPEN: u32 = 1 << 2;
const ROUTEEN_CLKPEN: u32 = 1 << 3;
const ROUTEEN_TXPEN: u32 = 1 << 4;

// ── USART0 as SPI, USART2 as I2S ─────────────────────────────────────────
const SPI0_BASE: usize = 0x400A_0000;
const I2S2_BASE: usize = 0x400A_8000;

const USART_EN: usize = 0x04;
const USART_CTRL: usize = 0x08;
const USART_CMD: usize = 0x14;
const USART_STATUS: usize = 0x18;
const USART_CLKDIV: usize = 0x1C;
const USART_RXDATA: usize = 0x24;
const USART_TXDATA: usize = 0x3C;
/// I2SCTRL, RM section 20.5.22 p.669.
const USART_I2SCTRL: usize = 0x54;

const USART_CTRL_SYNC: u32 = 1 << 0;
const USART_CMD_RXEN: u32 = 1 << 0;
const USART_CMD_TXEN: u32 = 1 << 2;
const USART_CMD_MASTEREN: u32 = 1 << 4;
const USART_STATUS_TXC: u32 = 1 << 5;
const USART_STATUS_TXBL: u32 = 1 << 6;

/// I2SCTRL.EN, "Enable I2S Mode" (RM section 20.5.22 p.669).
const I2SCTRL_EN: u32 = 1 << 0;
/// I2SCTRL.FORMAT = 2 is W32D24: a 32-bit word carrying 24 bits of data, which
/// is exactly what the INMP441 puts on the bus (its datasheet p.5: "24-bit
/// data" left-justified in a 32-bit slot).
const I2SCTRL_FORMAT_W32D24: u32 = 2 << 8;

// ── IADC0 ────────────────────────────────────────────────────────────────
const IADC0_BASE: usize = 0x4900_4000;
const IADC_EN: *mut u32 = (IADC0_BASE + 0x04) as *mut u32;
const IADC_CMD: *mut u32 = (IADC0_BASE + 0x0C) as *mut u32;
const IADC_STATUS: *const u32 = (IADC0_BASE + 0x14) as *const u32;
const IADC_SINGLEFIFODATA: *const u32 = (IADC0_BASE + 0x74) as *const u32;
const IADC_SINGLE: *mut u32 = (IADC0_BASE + 0x98) as *mut u32;
const IADC_EN_EN: u32 = 1 << 0;
const IADC_CMD_SINGLESTART: u32 = 1 << 0;
const IADC_STATUS_SINGLEFIFODV: u32 = 1 << 8;
/// SINGLE.PORTPOS = 11 (PORTD), PINPOS = 2 → PD02, the MIKROE_AN pad the
/// fader's wiper lands on. Ports are numbered from PORTA = 8.
const SINGLE_PD02: u32 = (11 << 12) | (2 << 8);

/// A poll that never completes must end. A wedged board says less than one
/// that prints a spin marker and carries on to the next part of the deck.
const POLL_LIMIT: u32 = 100_000;

// ── The panel ────────────────────────────────────────────────────────────
// MIPI DCS, shared by ST7789V and the ILI9341 (ST7789V datasheet section 9.1).
const CMD_SLPOUT: u8 = 0x11;
const CMD_DISPON: u8 = 0x29;
const CMD_CASET: u8 = 0x2A;
const CMD_RASET: u8 = 0x2B;
const CMD_RAMWR: u8 = 0x2C;
const CMD_COLMOD: u8 = 0x3A;
/// COLMOD 0x55: 16 bits/pixel, RGB565 (datasheet section 9.1.28).
const COLMOD_16BPP: u8 = 0x55;

/// The glass this module exposes: 170 columns starting at frame-memory column
/// 35, all 320 rows. Matches `agent-deck-system.yaml`'s visible window, and it
/// is a MODULE fact, not a datasheet one — the ST7789V document describes a
/// 240x320 frame memory and says nothing about which strip a panel shows.
const GLASS_COL0: u16 = 35;
const GLASS_COLS: u16 = 170;
const GLASS_ROWS: u16 = 320;

#[no_mangle]
pub extern "C" fn Reset() -> ! {
    main()
}

fn rd(p: *const u32) -> u32 {
    unsafe { core::ptr::read_volatile(p) }
}

fn wr(p: *mut u32, v: u32) {
    unsafe { core::ptr::write_volatile(p, v) }
}

fn reg(base: usize, off: usize) -> *mut u32 {
    (base + off) as *mut u32
}

fn putc(c: u8) {
    unsafe {
        let mut spins = 0;
        while core::ptr::read_volatile(USART1_STATUS) & USART_STATUS_TXBL == 0 {
            spins += 1;
            if spins >= POLL_LIMIT {
                return;
            }
        }
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

fn puthex32(v: u32) {
    puthex8(v >> 24);
    puthex8(v >> 16);
    puthex8(v >> 8);
    puthex8(v);
}

fn putdec(mut v: u32) {
    let mut buf = [0u8; 10];
    let mut n = 0;
    if v == 0 {
        putc(b'0');
        return;
    }
    while v > 0 {
        buf[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    while n > 0 {
        n -= 1;
        putc(buf[n]);
    }
}

// ── SPI on USART0 ────────────────────────────────────────────────────────

/// Clock one byte out of USART0 and wait for the shift to complete.
fn spi_byte(b: u8) {
    wr(reg(SPI0_BASE, USART_TXDATA), b as u32);
    let mut spins = 0;
    while rd(reg(SPI0_BASE, USART_STATUS)) & USART_STATUS_TXC == 0 {
        spins += 1;
        if spins >= POLL_LIMIT {
            return;
        }
    }
}

fn dc_low() {
    wr(GPIOC_DOUT, rd(GPIOC_DOUT) & !PC00_DC);
}

fn dc_high() {
    wr(GPIOC_DOUT, rd(GPIOC_DOUT) | PC00_DC);
}

/// A command byte: D/C LOW for the opcode. The panel latches D/C per byte, so
/// this must be set BEFORE the write, not after.
fn cmd(c: u8) {
    dc_low();
    spi_byte(c);
}

fn data(b: u8) {
    dc_high();
    spi_byte(b);
}

/// CASET/RASET take a 16-bit start and end, big-endian, inclusive.
fn set_window(x0: u16, x1: u16, y0: u16, y1: u16) {
    cmd(CMD_CASET);
    data((x0 >> 8) as u8);
    data(x0 as u8);
    data((x1 >> 8) as u8);
    data(x1 as u8);
    cmd(CMD_RASET);
    data((y0 >> 8) as u8);
    data(y0 as u8);
    data((y1 >> 8) as u8);
    data(y1 as u8);
}

fn main() -> ! {
    // ── Clocks. Every block this deck touches, and nothing else. ─────────
    wr(
        CMU_CLKEN0,
        CMU_CLKEN0_GPIO | CMU_CLKEN0_USART0 | CMU_CLKEN0_IADC0,
    );
    wr(CMU_CLKEN2, CMU_CLKEN2_USART1 | CMU_CLKEN2_USART2);

    // ── Console first, so everything after it can be seen. ───────────────
    wr(GPIOB_MODEL, MODE_PUSHPULL << 8); // PB02 push-pull
    wr(USART1_TXROUTE, TXROUTE_PB02);
    wr(USART1_ROUTEEN, GPIO_USART_ROUTEEN_TXPEN);
    wr(USART1_EN, 1);
    wr(USART1_CLKDIV, USART1_CLKDIV_115200);
    wr(USART1_CMD, USART_CMD_TXEN);
    puts("MG26-DECK\n");

    // ── GPIO modes. MODEL holds pins 0..7, one 4-bit nibble each. ────────
    // Outputs: PC00 D/C, PC04 CS, PC06 RES. Inputs with pull: PC01 button,
    // PC05 encoder A, PC07 encoder B.
    wr(
        GPIOC_MODEL,
        (MODE_PUSHPULL)
            | (MODE_INPUTPULL << 4)
            | (MODE_PUSHPULL << 16)
            | (MODE_INPUTPULL << 20)
            | (MODE_PUSHPULL << 24)
            | (MODE_INPUTPULL << 28),
    );
    // PD03 backlight out; PD04 encoder switch and PD05 toggle in, pulled up.
    wr(
        GPIOD_MODEL,
        (MODE_PUSHPULL << 12) | (MODE_INPUTPULL << 16) | (MODE_INPUTPULL << 20),
    );

    // A pulled INPUT takes its pull direction from DOUT on Series 2, so the
    // idle level of every contact is set here: the button module DRIVES its
    // SIG line and wants a pull-DOWN (DOUT low); everything else closes to
    // ground and wants a pull-UP (DOUT high).
    wr(GPIOC_DOUT, PC04_CS | PC05_ENC_CLK | PC07_ENC_DT);
    wr(GPIOD_DOUT, PD04_ENC_SW | PD05_TOGGLE);

    // ── Panel ────────────────────────────────────────────────────────────
    // RESX is active LOW (ST7789V datasheet section 6.2.1). Pulse it, then
    // light the backlight.
    wr(GPIOC_DOUT, rd(GPIOC_DOUT) & !PC06_RES);
    wr(GPIOC_DOUT, rd(GPIOC_DOUT) | PC06_RES);
    wr(GPIOD_DOUT, rd(GPIOD_DOUT) | PD03_BLK);

    // ⚠️ THE PANEL'S PINS. USART0 drives SCK on PC03 and MOSI on PC02 (UG594
    // Table 3.1). RX is deliberately NOT routed: this panel is write-only, and
    // the mikroBUS MISO pad PC01 is the deck's pushbutton.
    wr(USART0_CLKROUTE, 2 | (3 << 16));
    wr(USART0_TXROUTE, 2 | (2 << 16));
    wr(USART0_ROUTEEN, ROUTEEN_CLKPEN | ROUTEEN_TXPEN);

    // USART0 as a synchronous master: SYNC in CTRL, MASTEREN/TXEN/RXEN in CMD.
    wr(reg(SPI0_BASE, USART_EN), 1);
    wr(reg(SPI0_BASE, USART_CTRL), USART_CTRL_SYNC);
    wr(reg(SPI0_BASE, USART_CLKDIV), 0);
    wr(
        reg(SPI0_BASE, USART_CMD),
        USART_CMD_MASTEREN | USART_CMD_TXEN | USART_CMD_RXEN,
    );

    // Select the panel for the whole session: CS LOW.
    wr(GPIOC_DOUT, rd(GPIOC_DOUT) & !PC04_CS);

    cmd(CMD_SLPOUT);
    cmd(CMD_COLMOD);
    data(COLMOD_16BPP);
    cmd(CMD_DISPON);
    puts("tft: slpout colmod dispon\n");

    // Fill the whole glass. RGB565 0x001F is blue: two bytes per pixel.
    set_window(GLASS_COL0, GLASS_COL0 + GLASS_COLS - 1, 0, GLASS_ROWS - 1);
    cmd(CMD_RAMWR);
    dc_high();
    for _ in 0..(GLASS_COLS as u32 * GLASS_ROWS as u32) {
        spi_byte(0x00);
        spi_byte(0x1F);
    }
    puts("tft: filled ");
    putdec(GLASS_COLS as u32 * GLASS_ROWS as u32);
    puts(" px\n");

    // Deselect so the mic's traffic cannot be mistaken for panel traffic.
    wr(GPIOC_DOUT, rd(GPIOC_DOUT) | PC04_CS);

    // ── Microphone ───────────────────────────────────────────────────────
    // USART2 as an I2S main. RM section 20.3.3.7: "the main device must
    // generate the bus clock even when it is not transmitting data", so a mic
    // is read by writing TXDATA and then reading RXDATA — a read alone would
    // never clock the bus and would return the same stale word forever.
    // ⚠️ AND THE MICROPHONE'S. On this block the I2S word clock IS the CS
    // signal, so WS routes through CSROUTE — PA05 — while the bit clock is
    // CLK on PA04 and the mic's data arrives on RX at PA07.
    wr(USART2_CLKROUTE, 4 << 16);
    wr(USART2_CSROUTE, 5 << 16);
    wr(USART2_RXROUTE, 7 << 16);
    wr(
        USART2_ROUTEEN,
        ROUTEEN_CLKPEN | ROUTEEN_CSPEN | ROUTEEN_RXPEN,
    );

    wr(reg(I2S2_BASE, USART_EN), 1);
    wr(reg(I2S2_BASE, USART_CTRL), USART_CTRL_SYNC);
    wr(reg(I2S2_BASE, USART_CLKDIV), 0);
    wr(
        reg(I2S2_BASE, USART_CMD),
        USART_CMD_MASTEREN | USART_CMD_TXEN | USART_CMD_RXEN,
    );
    wr(
        reg(I2S2_BASE, USART_I2SCTRL),
        I2SCTRL_EN | I2SCTRL_FORMAT_W32D24,
    );

    // Eight slots. Stereo alternates L/R every word, and the frame restarts on
    // the LEFT when I2SCTRL.EN goes high, so the even slots are left and the
    // odd ones right.
    //
    // ⚠️ COUNT THE HALVES SEPARATELY. A bare "4 of 8 slots carried data" is
    // true whichever channel the mic sits on — a right-channel part drives the
    // other four and still totals four — so it cannot tell a correctly wired
    // mic from one whose L/R pin is up. Splitting the count is what makes the
    // INMP441's L/R-to-GND strap observable.
    let mut left = 0u32;
    let mut right = 0u32;
    puts("mic:");
    for i in 0..8 {
        wr(reg(I2S2_BASE, USART_TXDATA), 0);
        let slot = rd(reg(I2S2_BASE, USART_RXDATA));
        if slot != 0 {
            if i % 2 == 0 {
                left += 1;
            } else {
                right += 1;
            }
        }
        if i < 4 {
            putc(b' ');
            puthex32(slot);
        }
    }
    puts("\nmic: left=");
    putdec(left);
    puts(" right=");
    putdec(right);
    putc(b'\n');

    // ── Fader ────────────────────────────────────────────────────────────
    wr(IADC_EN, IADC_EN_EN);
    wr(IADC_SINGLE, SINGLE_PD02);
    wr(IADC_CMD, IADC_CMD_SINGLESTART);
    let mut spins = 0;
    while rd(IADC_STATUS) & IADC_STATUS_SINGLEFIFODV == 0 {
        spins += 1;
        if spins >= POLL_LIMIT {
            break;
        }
    }
    puts("fader: code=");
    putdec(rd(IADC_SINGLEFIFODATA) & 0xFFF);
    putc(b'\n');

    // ── Contacts. DIN, not DOUT: the pin path, not the output latch. ─────
    let c = rd(GPIOC_DIN);
    let d = rd(GPIOD_DIN);
    puts("in: CLK=");
    putdec((c & PC05_ENC_CLK != 0) as u32);
    puts(" DT=");
    putdec((c & PC07_ENC_DT != 0) as u32);
    puts(" SW=");
    putdec((d & PD04_ENC_SW != 0) as u32);
    puts(" BTN=");
    putdec((c & PC01_BTN != 0) as u32);
    puts(" TOGGLE=");
    putdec((d & PD05_TOGGLE != 0) as u32);
    putc(b'\n');

    puts("MG26-DECK DONE\n");
    loop {}
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
