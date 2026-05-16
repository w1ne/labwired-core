//! Tri-color 2.9" e-paper lab — STM32F103 (NUCLEO-F103RB compatible)
//!
//! Drives a Waveshare 2.9" tri-color e-paper module (SSD1680 / GDEM029C90)
//! over SPI1 + four GPIO sidebands. Draws three full-width horizontal bands —
//! WHITE on top, BLACK in the middle, RED on the bottom — and one full refresh.
//!
//! The same ELF runs on real silicon (flash to NUCLEO-F103RB via ST-Link) and
//! inside the LabWired sim (`labwired -s system.yaml -f <this.elf>`). The
//! simulated panel and the physical panel should show pixel-identical output —
//! that's the side-by-side fidelity check.
//!
//! Pin mapping (Nucleo Arduino-header friendly):
//!   PA4  — CS    (A2)         GPIO output push-pull
//!   PA5  — SCK   (D13)        AF push-pull
//!   PA7  — MOSI  (D11)        AF push-pull
//!   PA9  — RST   (D8)         GPIO output push-pull
//!   PB0  — DC    (A3)         GPIO output push-pull
//!   PC7  — BUSY  (D9)         GPIO input floating
//!
//! Wiring to the Waveshare module:
//!   3V3 → VCC,  GND → GND,  PA4 → CS,  PA5 → CLK,  PA7 → DIN,
//!   PB0 → DC,   PA9 → RST,  PC7 ← BUSY

#![no_std]
#![no_main]

use cortex_m_rt::entry;
use panic_halt as _;

// ----- Register addresses -------------------------------------------------

const RCC_APB2ENR: *mut u32   = 0x4002_1018 as *mut u32;
const GPIOA_CRL:   *mut u32   = 0x4001_0800 as *mut u32;
const GPIOA_CRH:   *mut u32   = 0x4001_0804 as *mut u32;
const GPIOA_BSRR:  *mut u32   = 0x4001_0810 as *mut u32;
const GPIOA_BRR:   *mut u32   = 0x4001_0814 as *mut u32;
const GPIOB_CRL:   *mut u32   = 0x4001_0C00 as *mut u32;
const GPIOB_BSRR:  *mut u32   = 0x4001_0C10 as *mut u32;
const GPIOB_BRR:   *mut u32   = 0x4001_0C14 as *mut u32;
const GPIOC_CRL:   *mut u32   = 0x4001_1000 as *mut u32;
const GPIOC_IDR:   *const u32 = 0x4001_1008 as *const u32;
const SPI1_CR1:    *mut u16   = 0x4001_3000 as *mut u16;
const SPI1_SR:     *const u16 = 0x4001_3008 as *const u16;
const SPI1_DR:     *mut u16   = 0x4001_300C as *mut u16;

// ----- Pin masks ----------------------------------------------------------

const CS_MASK:   u32 = 1 << 4;  // PA4
const RST_MASK:  u32 = 1 << 9;  // PA9
const DC_MASK:   u32 = 1 << 0;  // PB0
const BUSY_MASK: u32 = 1 << 7;  // PC7

// ----- Panel geometry -----------------------------------------------------

const WIDTH: u16 = 128;
const HEIGHT: u16 = 296;
const WIDTH_BYTES: u16 = WIDTH / 8;

// ----- GPIO helpers -------------------------------------------------------

#[inline(always)] fn cs_low()  { unsafe { core::ptr::write_volatile(GPIOA_BRR,  CS_MASK)  } }
#[inline(always)] fn cs_high() { unsafe { core::ptr::write_volatile(GPIOA_BSRR, CS_MASK)  } }
#[inline(always)] fn dc_low()  { unsafe { core::ptr::write_volatile(GPIOB_BRR,  DC_MASK)  } }
#[inline(always)] fn dc_high() { unsafe { core::ptr::write_volatile(GPIOB_BSRR, DC_MASK)  } }
#[inline(always)] fn rst_low() { unsafe { core::ptr::write_volatile(GPIOA_BRR,  RST_MASK) } }
#[inline(always)] fn rst_high(){ unsafe { core::ptr::write_volatile(GPIOA_BSRR, RST_MASK) } }

#[inline(always)]
fn busy_high() -> bool {
    unsafe { (core::ptr::read_volatile(GPIOC_IDR) & BUSY_MASK) != 0 }
}

fn delay(cycles: u32) {
    for _ in 0..cycles {
        cortex_m::asm::nop();
    }
}

/// Block while the panel BUSY pin is high (refresh in progress).
/// Bounded so the sim, which never raises BUSY, doesn't hang.
fn wait_idle() {
    for _ in 0..2_000_000 {
        if !busy_high() {
            return;
        }
    }
}

// ----- SPI helpers --------------------------------------------------------

fn spi_write(byte: u8) {
    // Wait for the TX register to drain before queuing the next byte. The
    // panel is write-only over SPI so we deliberately skip the RXNE wait —
    // on real silicon RXNE would set after each byte clocks in, but for a
    // write-only display nothing reads it, and waiting on it forever stalls
    // the simulator (which doesn't drive MISO without a slave).
    for _ in 0..4096 {
        let sr = unsafe { core::ptr::read_volatile(SPI1_SR) };
        if sr & 0x0002 != 0 { break; }
    }
    unsafe { core::ptr::write_volatile(SPI1_DR, byte as u16) };
}

/// Block until the SPI shift register has finished — required before CS-high
/// or the panel will see a truncated final byte.
fn spi_flush() {
    for _ in 0..4096 {
        let sr = unsafe { core::ptr::read_volatile(SPI1_SR) };
        // BSY clear AND TXE set → shift register idle.
        if sr & 0x0080 == 0 && sr & 0x0002 != 0 { break; }
    }
}

// ----- SSD1680 protocol ---------------------------------------------------
//
// Real silicon multiplexes command vs data via the D/C pin:
//   D/C low  = command byte
//   D/C high = data byte
// The LabWired sim ignores D/C and infers state from command-parameter counts;
// driving D/C correctly here keeps the same firmware working on both.

fn ep_cmd(cmd: u8) {
    dc_low();
    cs_low();
    spi_write(cmd);
    spi_flush();
    cs_high();
}

fn ep_cmd_data(cmd: u8, data: &[u8]) {
    dc_low();
    cs_low();
    spi_write(cmd);
    dc_high();
    for &b in data {
        spi_write(b);
    }
    spi_flush();
    cs_high();
}

/// Hardware reset pulse — RST low for ~10ms-equivalent, then back high.
fn ep_hw_reset() {
    rst_high();
    delay(200_000);
    rst_low();
    delay(200_000);
    rst_high();
    delay(200_000);
    wait_idle();
}

/// Mirrors GxEPD2_290_C90c::_InitDisplay() — the byte sequence the AgentDeck
/// firmware emits during boot. Window is set to the full 128x296 panel.
fn ep_init() {
    ep_hw_reset();
    ep_cmd(0x12); // SWRESET
    wait_idle();

    ep_cmd_data(0x01, &[0x27, 0x01, 0x00]); // Driver output control
    ep_cmd_data(0x11, &[0x03]);             // Data entry mode (X+/Y+, X-major)
    ep_cmd_data(0x3C, &[0x05]);             // Border waveform
    ep_cmd_data(0x18, &[0x80]);             // Temp sensor select
    ep_cmd_data(0x21, &[0x00, 0x80]);       // Display update ctrl 1

    // Window: full panel
    ep_cmd_data(0x44, &[0x00, (WIDTH_BYTES - 1) as u8]);                          // RAM-X 0..15 (bytes)
    ep_cmd_data(0x45, &[0x00, 0x00, ((HEIGHT - 1) & 0xFF) as u8, ((HEIGHT - 1) >> 8) as u8]); // RAM-Y 0..295
    ep_cmd_data(0x4E, &[0x00]);             // RAM-X counter = 0
    ep_cmd_data(0x4F, &[0x00, 0x00]);       // RAM-Y counter = 0
}

/// Stream a full-screen plane (4736 bytes) using `byte_for_row()` for each row.
/// Caller picks the command (0x24 = black plane, 0x26 = red plane).
fn ep_stream_plane<F: Fn(u16) -> u8>(cmd: u8, byte_for_row: F) {
    // Reset RAM counter to (0, 0) so the stream starts at the top-left.
    ep_cmd_data(0x4E, &[0x00]);
    ep_cmd_data(0x4F, &[0x00, 0x00]);

    dc_low();
    cs_low();
    spi_write(cmd);
    dc_high();
    // u32 counters compile to dramatically faster code than the u16/u16 nested
    // version (the Cortex-M3 codegen for u16 loop bounds emits extra zero-
    // extensions and bound checks per iteration — measured ~100x slowdown
    // streaming 4736 bytes through this exact path).
    for row in 0..HEIGHT as u32 {
        let v = byte_for_row(row as u16);
        for _ in 0..WIDTH_BYTES as u32 {
            spi_write(v);
        }
    }
    spi_flush();
    cs_high();
}

/// Trigger a full refresh and wait for the panel to finish.
fn ep_refresh() {
    ep_cmd_data(0x22, &[0xF7]); // Sequence: load LUT + display
    ep_cmd(0x20);               // Master activation
    wait_idle();
}

// ----- Test pattern -------------------------------------------------------
//
// Three equal-ish horizontal bands stacked top→bottom, 99/99/98 rows.
// The wire encoding for each plane:
//   black plane (0x24): 1 = white (no ink), 0 = black
//   red plane   (0x26): 1 = no-red,         0 = red
// (GxEPD2 inverts the source bitmap before sending 0x26 — we write the
//  panel-wire byte directly here, so no inversion needed.)
//
// Band 0 (rows 0..=98):    WHITE  → black=0xFF, red=0xFF
// Band 1 (rows 99..=197):  BLACK  → black=0x00, red=0xFF
// Band 2 (rows 198..=295): RED    → black=0xFF, red=0x00

fn black_plane_byte(row: u16) -> u8 {
    match row {
        99..=197 => 0x00, // black band: ink everywhere
        _        => 0xFF, // white & red bands: no black ink
    }
}

fn red_plane_byte(row: u16) -> u8 {
    match row {
        198..=295 => 0x00, // red band: red ink
        _         => 0xFF, // white & black bands: no red ink
    }
}

// ----- Boot ---------------------------------------------------------------

#[entry]
fn main() -> ! {
    unsafe {
        // Enable RCC for GPIOA (bit 2), GPIOB (bit 3), GPIOC (bit 4), SPI1 (bit 12).
        let apb2enr = core::ptr::read_volatile(RCC_APB2ENR);
        core::ptr::write_volatile(
            RCC_APB2ENR,
            apb2enr | (1 << 12) | (1 << 4) | (1 << 3) | (1 << 2),
        );

        // GPIOA CRL: PA4=out PP 50MHz, PA5=AF PP 50MHz, PA6=in float, PA7=AF PP 50MHz.
        //   bits[19:16] PA4 = 0011 (0x3)
        //   bits[23:20] PA5 = 1011 (0xB)
        //   bits[27:24] PA6 = 0100 (0x4)
        //   bits[31:28] PA7 = 1011 (0xB)
        let mut crl = core::ptr::read_volatile(GPIOA_CRL);
        crl &= 0x0000_FFFF;
        crl |= 0xB4B3_0000;
        core::ptr::write_volatile(GPIOA_CRL, crl);

        // GPIOA CRH: PA9 = out PP 50MHz (bits[7:4] = 0011 = 0x3).
        let mut crh = core::ptr::read_volatile(GPIOA_CRH);
        crh &= 0xFFFF_FF0F;
        crh |= 0x0000_0030;
        core::ptr::write_volatile(GPIOA_CRH, crh);

        // GPIOB CRL: PB0 = out PP 50MHz (bits[3:0] = 0011 = 0x3).
        let mut bcrl = core::ptr::read_volatile(GPIOB_CRL);
        bcrl &= 0xFFFF_FFF0;
        bcrl |= 0x0000_0003;
        core::ptr::write_volatile(GPIOB_CRL, bcrl);

        // GPIOC CRL: PC7 = input floating (bits[31:28] = 0100 = 0x4).
        let mut ccrl = core::ptr::read_volatile(GPIOC_CRL);
        ccrl &= 0x0FFF_FFFF;
        ccrl |= 0x4000_0000;
        core::ptr::write_volatile(GPIOC_CRL, ccrl);

        // Idle states: CS high, RST high, DC high.
        core::ptr::write_volatile(GPIOA_BSRR, CS_MASK | RST_MASK);
        core::ptr::write_volatile(GPIOB_BSRR, DC_MASK);

        // SPI1: master mode, BR=001 (f_pclk/4 — at 72MHz APB2 that's 18MHz SCLK,
        // safely under the SSD1680's 20MHz max), CPOL=0 CPHA=0, 8-bit, SPE,
        // software slave management with SSI high so master mode stays asserted.
        //   CR1 = SPE(6) | MSTR(2) | BR=001(bits 5:3) | SSM(9) | SSI(8) = 0x034C
        core::ptr::write_volatile(SPI1_CR1, 0x034Cu16);
    }

    ep_init();

    ep_stream_plane(0x24, black_plane_byte);
    ep_stream_plane(0x26, red_plane_byte);

    ep_refresh();

    // Done — sit forever. On real hardware the panel holds the image without power.
    loop {
        cortex_m::asm::wfi();
    }
}
