// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **Batch 3 displays: one smoke check per part, and the negative control that
//! keeps each check honest.**
//!
//! The sibling of `everyday_sensor_set.rs` for parts on the `display`
//! primitive: `st7735` (Sitronix ST7735R, 128x160 RGB565, 4-wire SPI),
//! `gc9a01` (GalaxyCore GC9A01A, 240x240 RGB565, 4-wire SPI) and
//! `oled-ssd1309` (Solomon Systech SSD1309, 128x64 mono, I²C). Each is a
//! `configs/devices/*.yaml` descriptor with no Rust model.
//!
//! A display answers no register reads, so a check cannot assert a WHO_AM_I.
//! What it asserts instead is the documented reset state of the control flags
//! (sleep in, display off, inversion off — the datasheets' reset tables) and
//! the frame memory a real command sequence leaves behind: a window opened
//! with CASET/RASET (or the SSD1309's page/column pointer commands), pixels
//! written through RAMWR (or the data stream), and the exact bytes at the
//! exact addresses, with every other byte still blank.
//!
//! [`negative_control_a_silent_panel_fails_every_check`] runs each check
//! against a stand-in whose frame memory reads all `0x00` and whose flags are
//! all false, and one whose memory reads all `0xFF` and whose flags are all
//! true. Both must FAIL every check: a check a constant panel could pass
//! proves nothing.

use labwired_core::inspect::InspectOpts;
use labwired_core::peripherals::components::declarative_display::{embedded, GenericDisplay};
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::peripherals::spi::SpiDevice;

// ─── plumbing ──────────────────────────────────────────────────────────────

macro_rules! ensure_eq {
    ($got:expr, $want:expr, $what:expr) => {{
        let got = $got;
        let want = $want;
        if got != want {
            return Err(format!("{}: got {:?}, want {:?}", $what, got, want));
        }
    }};
}

/// What a check may do to a panel and read back from it.
trait Panel {
    /// One command byte followed by its parameter bytes.
    fn command(&mut self, opcode: u8, args: &[u8]);
    /// Bytes on the data stream (pixels / GDDRAM).
    fn data(&mut self, bytes: &[u8]);
    /// Physical frame memory.
    fn fb(&self) -> Vec<u8>;
    fn display_on(&self) -> bool;
    fn awake(&self) -> bool;
    fn inverted(&self) -> bool;
}

/// A 4-wire SPI panel: D/C low for the opcode, high for parameters and pixels.
struct Spi(GenericDisplay);

impl Panel for Spi {
    fn command(&mut self, opcode: u8, args: &[u8]) {
        self.0.set_dc_level(false);
        self.0.transfer(opcode);
        self.0.set_dc_level(true);
        for &a in args {
            self.0.transfer(a);
        }
    }
    fn data(&mut self, bytes: &[u8]) {
        self.0.set_dc_level(true);
        for &b in bytes {
            self.0.transfer(b);
        }
    }
    fn fb(&self) -> Vec<u8> {
        self.0.framebuffer().to_vec()
    }
    fn display_on(&self) -> bool {
        self.0.display_on()
    }
    fn awake(&self) -> bool {
        self.0.awake()
    }
    fn inverted(&self) -> bool {
        self.0.inverted()
    }
}

/// An I²C OLED: one transaction per call, control byte 0x00 (commands) or
/// 0x40 (data) first, as SSD1309 section 8.1.5.2 frames it.
struct I2c(GenericDisplay);

impl I2c {
    fn transaction(&mut self, control: u8, bytes: &[u8]) {
        self.0.start();
        self.0.write(control);
        for &b in bytes {
            self.0.write(b);
        }
        self.0.stop();
    }
}

impl Panel for I2c {
    fn command(&mut self, opcode: u8, args: &[u8]) {
        let mut bytes = vec![opcode];
        bytes.extend_from_slice(args);
        self.transaction(0x00, &bytes);
    }
    fn data(&mut self, bytes: &[u8]) {
        self.transaction(0x40, bytes);
    }
    fn fb(&self) -> Vec<u8> {
        self.0.framebuffer().to_vec()
    }
    fn display_on(&self) -> bool {
        self.0.display_on()
    }
    fn awake(&self) -> bool {
        self.0.awake()
    }
    fn inverted(&self) -> bool {
        self.0.inverted()
    }
}

fn spi(device_type: &str) -> Spi {
    let mut d = embedded(device_type).unwrap_or_else(|e| panic!("{device_type}: {e}"));
    d.set_cs_pin("CS");
    d.set_dc_pin("DC");
    Spi(d)
}

fn i2c(device_type: &str) -> I2c {
    I2c(embedded(device_type).unwrap_or_else(|e| panic!("{device_type}: {e}")))
}

/// Offset of pixel (x, y) in a row-major RGB565 frame memory `width` wide.
fn px(width: usize, x: usize, y: usize) -> usize {
    (y * width + x) * 2
}

/// The reset flags every one of these datasheets gives: sleep in, display off,
/// inversion off.
fn check_reset_flags(d: &dyn Panel, part: &str) -> Result<(), String> {
    ensure_eq!(d.awake(), false, format!("{part} reset: sleep in"));
    ensure_eq!(d.display_on(), false, format!("{part} reset: display off"));
    ensure_eq!(d.inverted(), false, format!("{part} reset: inversion off"));
    Ok(())
}

/// The shared MIPI-DBI-style smoke for an RGB565 TFT `w` x `h`:
///
/// 1. reset flags (sleep in, display off, inversion off);
/// 2. SLPOUT → awake; DISPON → on; INVON → inverted, INVOFF → not;
/// 3. MADCTL whose PARAMETER is 0x2C, i.e. RAMWR's opcode: it must be taken as
///    a parameter (D/C high), not open a pixel stream — the frame stays blank;
///    MADCTL then set back to 0x00;
/// 4. COLMOD 0x05 consumed for its one argument;
/// 5. CASET 2..3 / RASET 4..5 / RAMWR with four distinct RGB565 pixels: each
///    lands big-endian at (2,4) (3,4) (2,5) (3,5), and no other byte changed.
fn check_rgb565_tft(d: &mut dyn Panel, part: &str, w: usize, h: usize) -> Result<(), String> {
    let bytes = w * h * 2;
    ensure_eq!(d.fb().len(), bytes, format!("{part} frame memory size"));
    check_reset_flags(d, part)?;

    d.command(0x11, &[]);
    ensure_eq!(d.awake(), true, format!("{part} SLPOUT"));
    d.command(0x29, &[]);
    ensure_eq!(d.display_on(), true, format!("{part} DISPON"));
    d.command(0x21, &[]);
    ensure_eq!(d.inverted(), true, format!("{part} INVON"));
    d.command(0x20, &[]);
    ensure_eq!(d.inverted(), false, format!("{part} INVOFF"));

    d.command(0x36, &[0x2C]);
    d.data(&[0xAB, 0xCD]);
    ensure_eq!(
        d.fb().iter().filter(|&&b| b != 0).count(),
        0,
        format!("{part}: MADCTL parameter 0x2C must not open RAMWR")
    );
    d.command(0x36, &[0x00]);
    d.command(0x3A, &[0x05]);

    d.command(0x2A, &[0x00, 0x02, 0x00, 0x03]);
    d.command(0x2B, &[0x00, 0x04, 0x00, 0x05]);
    let pixels: [[u8; 2]; 4] = [[0xF8, 0x00], [0x07, 0xE0], [0x00, 0x1F], [0xFF, 0xFF]];
    d.command(0x2C, &pixels.concat());
    let fb = d.fb();
    for (i, (x, y)) in [(2usize, 4usize), (3, 4), (2, 5), (3, 5)]
        .into_iter()
        .enumerate()
    {
        let o = px(w, x, y);
        ensure_eq!(
            [fb[o], fb[o + 1]],
            pixels[i],
            format!("{part} RAMWR pixel {i} at ({x},{y})")
        );
    }
    let mut expect = vec![0u8; bytes];
    for (i, (x, y)) in [(2usize, 4usize), (3, 4), (2, 5), (3, 5)]
        .into_iter()
        .enumerate()
    {
        let o = px(w, x, y);
        expect[o..o + 2].copy_from_slice(&pixels[i]);
    }
    if fb != expect {
        let stray = fb.iter().zip(&expect).filter(|(a, b)| a != b).count();
        return Err(format!(
            "{part}: {stray} bytes outside the 2x2 window changed"
        ));
    }
    Ok(())
}

fn check_st7735(d: &mut dyn Panel) -> Result<(), String> {
    check_rgb565_tft(d, "ST7735", 128, 160)
}

fn check_gc9a01(d: &mut dyn Panel) -> Result<(), String> {
    check_rgb565_tft(d, "GC9A01", 240, 240)
}

/// SSD1309 (Rev 1.1):
///
/// 1. display off at reset (Table 9-1, AEh "(RESET)");
/// 2. an init sequence of parameter-carrying commands whose PARAMETERS are
///    valid opcodes (FDh 12h, 81h CFh, D3h 00h, D5h 80h, DAh 12h, DCh 02h, the
///    seven-byte 26h scroll setup with 00h/07h/7Fh) leaves the pointers alone;
/// 3. reset addressing mode is PAGE (Table 9-3): the datasheet's own Figure
///    10-2 example B2h / 03h / 10h puts the next data byte at column 3 of
///    PAGE2;
/// 4. page mode wraps COL127 → COL0 within the page (section 10.3, Fig. 10-1);
/// 5. 20h 00h horizontal mode, 21h 7Eh..7Fh, 22h 03h..04h: four bytes fill
///    columns 126,127 of page 3 then of page 4;
/// 6. AFh → display on. No byte other than the eight written changed.
fn check_ssd1309(d: &mut dyn Panel) -> Result<(), String> {
    ensure_eq!(d.fb().len(), 1024, "SSD1309 GDDRAM size");
    ensure_eq!(d.display_on(), false, "SSD1309 reset: display off");

    for (op, args) in [
        (0xFDu8, &[0x12u8][..]),
        (0x81, &[0xCF]),
        (0xD3, &[0x00]),
        (0xD5, &[0x80]),
        (0xDA, &[0x12]),
        (0xDC, &[0x02]),
        (0x26, &[0x00, 0x00, 0x07, 0x07, 0x00, 0x00, 0x7F]),
    ] {
        d.command(op, args);
    }

    d.command(0xB2, &[]);
    d.command(0x03, &[]);
    d.command(0x10, &[]);
    d.data(&[0xA5]);
    let mut expect = vec![0u8; 1024];
    expect[2 * 128 + 3] = 0xA5;
    ensure_eq!(
        d.fb()[2 * 128 + 3],
        0xA5,
        "SSD1309 Fig. 10-2: PAGE2 column 3"
    );

    d.command(0xB1, &[]);
    d.command(0x0F, &[]);
    d.command(0x17, &[]);
    d.data(&[0x11, 0x22]);
    expect[128 + 127] = 0x11;
    expect[128] = 0x22;
    ensure_eq!(
        (d.fb()[128 + 127], d.fb()[128]),
        (0x11, 0x22),
        "SSD1309 page mode wraps COL127 -> COL0 in PAGE1"
    );

    d.command(0x20, &[0x00]);
    d.command(0x21, &[0x7E, 0x7F]);
    d.command(0x22, &[0x03, 0x04]);
    d.data(&[0x01, 0x02, 0x03, 0x04]);
    expect[3 * 128 + 126] = 0x01;
    expect[3 * 128 + 127] = 0x02;
    expect[4 * 128 + 126] = 0x03;
    expect[4 * 128 + 127] = 0x04;

    d.command(0xAF, &[]);
    ensure_eq!(d.display_on(), true, "SSD1309 AFh display on");

    let fb = d.fb();
    if fb != expect {
        let diff: Vec<usize> = (0..1024).filter(|&i| fb[i] != expect[i]).collect();
        return Err(format!(
            "SSD1309 GDDRAM differs at {} bytes, first {:?}",
            diff.len(),
            &diff[..diff.len().min(8)]
        ));
    }
    Ok(())
}

type Check = fn(&mut dyn Panel) -> Result<(), String>;

/// (catalog type, transport, frame-memory bytes, check)
const PARTS: &[(&str, Bus, usize, Check)] = &[
    ("st7735", Bus::Spi, 128 * 160 * 2, check_st7735),
    ("gc9a01", Bus::Spi, 240 * 240 * 2, check_gc9a01),
    ("oled-ssd1309", Bus::I2c, 1024, check_ssd1309),
];

#[derive(Clone, Copy)]
enum Bus {
    Spi,
    I2c,
}

// ─── smoke + negative control ──────────────────────────────────────────────

#[test]
fn every_display_passes_its_smoke_check() {
    let mut failures = Vec::new();
    for &(ty, bus, _, check) in PARTS {
        let got = match bus {
            Bus::Spi => check(&mut spi(ty)),
            Bus::I2c => check(&mut i2c(ty)),
        };
        match got {
            Ok(()) => println!("smoke  {ty:<13}: PASS"),
            Err(e) => failures.push(format!("{ty}: {e}")),
        }
    }
    assert!(
        failures.is_empty(),
        "smoke failures:\n{}",
        failures.join("\n")
    );
}

/// A stand-in panel that accepts every byte and changes nothing: its frame
/// memory reads `byte` everywhere and every flag reads `byte != 0`.
struct Silent {
    byte: u8,
    len: usize,
}

impl Panel for Silent {
    fn command(&mut self, _opcode: u8, _args: &[u8]) {}
    fn data(&mut self, _bytes: &[u8]) {}
    fn fb(&self) -> Vec<u8> {
        vec![self.byte; self.len]
    }
    fn display_on(&self) -> bool {
        self.byte != 0
    }
    fn awake(&self) -> bool {
        self.byte != 0
    }
    fn inverted(&self) -> bool {
        self.byte != 0
    }
}

/// The real model's control flags over a frame memory that reads `byte`
/// everywhere. Proves the pixel assertions bite on their own, not only the
/// flag assertions a [`Silent`] panel already trips first.
struct MuteMemory<P: Panel> {
    inner: P,
    byte: u8,
}

impl<P: Panel> Panel for MuteMemory<P> {
    fn command(&mut self, opcode: u8, args: &[u8]) {
        self.inner.command(opcode, args);
    }
    fn data(&mut self, bytes: &[u8]) {
        self.inner.data(bytes);
    }
    fn fb(&self) -> Vec<u8> {
        vec![self.byte; self.inner.fb().len()]
    }
    fn display_on(&self) -> bool {
        self.inner.display_on()
    }
    fn awake(&self) -> bool {
        self.inner.awake()
    }
    fn inverted(&self) -> bool {
        self.inner.inverted()
    }
}

#[test]
fn negative_control_a_silent_panel_fails_every_check() {
    for &(ty, bus, len, check) in PARTS {
        for byte in [0x00u8, 0xFF] {
            let silent = check(&mut Silent { byte, len });
            let mute = match bus {
                Bus::Spi => check(&mut MuteMemory {
                    inner: spi(ty),
                    byte,
                }),
                Bus::I2c => check(&mut MuteMemory {
                    inner: i2c(ty),
                    byte,
                }),
            };
            for (what, got) in [("silent panel", silent), ("live flags, mute memory", mute)] {
                match got {
                    Err(e) => println!(
                        "negctl {ty:<13} {what:<23} reading 0x{byte:02X}: FAILS as required ({e})"
                    ),
                    Ok(()) => panic!(
                        "{ty}: a {what} reading only 0x{byte:02X} PASSED the smoke check — the \
                         check proves nothing"
                    ),
                }
            }
        }
    }
}

// ─── per-part behaviour beyond the smoke ───────────────────────────────────

/// ST7735R reset table 9.15.1, "After S/W Reset": sleep in, display off,
/// inversion off, window back to the full memory — but frame memory AND MADCTL
/// "No Change". So after SWRESET a bare RAMWR writes at the window origin
/// through the RETAINED MY|MX mirror: logical (0,0) is physical (127,159), and
/// the pixel painted before the reset is still there.
#[test]
fn st7735_swreset_keeps_frame_memory_and_madctl() {
    let mut d = spi("st7735");
    d.command(0x11, &[]);
    d.command(0x29, &[]);
    d.command(0x21, &[]);
    d.command(0x2A, &[0, 10, 0, 10]);
    d.command(0x2B, &[0, 20, 0, 20]);
    d.command(0x2C, &[0x12, 0x34]);
    d.command(0x36, &[0xC0]); // MY | MX

    d.command(0x01, &[]);
    assert!(
        !d.awake() && !d.display_on() && !d.inverted(),
        "SWRESET control state"
    );
    let fb = d.fb();
    assert_eq!(
        &fb[px(128, 10, 20)..px(128, 10, 20) + 2],
        &[0x12, 0x34],
        "frame memory kept"
    );

    d.command(0x2C, &[0xBE, 0xEF]);
    let fb = d.fb();
    assert_eq!(
        &fb[px(128, 127, 159)..px(128, 127, 159) + 2],
        &[0xBE, 0xEF],
        "MADCTL MY|MX kept across SWRESET and the window reset to the origin"
    );
}

/// MADCTL MV (row/column exchange) changes what a legal column IS: 10.1.19
/// gives XE ≤ 127 with MV = 0 and ≤ 159 with MV = 1. A CASET end of 0x0200
/// clamps to that edge, and a run of pixels past it wraps to the next row.
#[test]
fn st7735_mv_moves_the_column_clamp() {
    let mut d = spi("st7735");
    d.command(0x36, &[0x20]); // MV
    d.command(0x2A, &[0x00, 0x9E, 0x02, 0x00]); // 158 .. clamped 159
    d.command(0x2B, &[0x00, 0x00, 0x00, 0x01]);
    d.command(0x2C, &[0x11, 0x11, 0x22, 0x22, 0x33, 0x33]);
    let fb = d.fb();
    // MV swaps: logical (col, row) → physical (x = row, y = col).
    assert_eq!(&fb[px(128, 0, 158)..px(128, 0, 158) + 2], &[0x11, 0x11]);
    assert_eq!(&fb[px(128, 0, 159)..px(128, 0, 159) + 2], &[0x22, 0x22]);
    assert_eq!(
        &fb[px(128, 1, 158)..px(128, 1, 158) + 2],
        &[0x33, 0x33],
        "past the clamped column 159 the counter wraps to column 158, row 1"
    );
}

/// ST7735R section 9.6 / GC9A01A section 4.1.12, "Data transfer pause": CSX
/// released after a whole byte of frame memory data, the driver "will wait and
/// continue ... from the point where it was paused." A blit chunked across two
/// CS assertions lands as one contiguous run.
#[test]
fn cs_pause_keeps_the_pixel_stream() {
    for ty in ["st7735", "gc9a01"] {
        let mut d = spi(ty);
        d.0.cs_select();
        d.command(0x2C, &[0x11, 0x22]);
        d.0.cs_select();
        d.data(&[0x33, 0x44]);
        assert_eq!(
            &d.fb()[0..4],
            &[0x11, 0x22, 0x33, 0x44],
            "{ty}: stream survives CS"
        );
    }
}

/// The paint artifact carries the counts the browser overlay and the CLI's
/// `painted bytes=` line read. A 2x2 window of non-zero bytes is 8 painted
/// bytes, and `lit` needs both DISPON and SLPOUT.
#[test]
fn gc9a01_artifact_reports_painted_bytes_and_lit() {
    let mut d = spi("gc9a01");
    let meta = |d: &Spi| {
        SpiDevice::artifacts(&d.0, "tft", &InspectOpts::default())[0]
            .meta
            .clone()
    };
    assert_eq!(meta(&d)["painted_bytes"], 0);
    assert_eq!(meta(&d)["total_bytes"], 240 * 240 * 2);
    d.command(0x2A, &[0, 100, 0, 101]);
    d.command(0x2B, &[0, 200, 0, 201]);
    d.command(0x2C, &[0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0]);
    assert_eq!(meta(&d)["painted_bytes"], 8);
    d.command(0x29, &[]);
    assert_eq!(
        meta(&d)["lit"],
        false,
        "DISPON without SLPOUT is still dark"
    );
    d.command(0x11, &[]);
    assert_eq!(meta(&d)["lit"], true);
}

/// The GC9A01A lists no SWRESET (section 6.1 has no 01h). An 01h is consumed
/// as an unlisted opcode: no flag changes, and — because D/C framing closes
/// the open RAMWR on any command — the next data byte is a stray.
#[test]
fn gc9a01_has_no_swreset_opcode() {
    let mut d = spi("gc9a01");
    d.command(0x11, &[]);
    d.command(0x29, &[]);
    d.command(0x2C, &[0xAA, 0xBB]);
    d.command(0x01, &[]);
    d.data(&[0xCC, 0xDD]);
    assert!(
        d.awake() && d.display_on(),
        "01h must not reset the GC9A01A"
    );
    let fb = d.fb();
    assert_eq!(
        &fb[0..4],
        &[0xAA, 0xBB, 0, 0],
        "01h closed the RAMWR stream"
    );
}

/// MADCTL MY|MX on the GC9A01A: section 5.2 maps the counters to
/// (239 - physical pointer), so logical (0,0) is physical (239,239).
#[test]
fn gc9a01_mirror_is_239_minus_pointer() {
    let mut d = spi("gc9a01");
    d.command(0x36, &[0xC0]);
    d.command(0x2C, &[0x5A, 0xA5]);
    let fb = d.fb();
    assert_eq!(&fb[px(240, 239, 239)..px(240, 239, 239) + 2], &[0x5A, 0xA5]);
}

/// The SSD1309 has no charge pump and no 8Dh (section 2, Tables 9-1..9-5).
/// The SSD1306 init's `8D 14` is therefore decoded per the SSD1309 table: 8Dh
/// is an unlisted opcode, 14h is Set Higher Column Start (column high nibble
/// 4). In page mode the next byte lands at column 0x40.
#[test]
fn ssd1309_has_no_charge_pump_command() {
    let mut d = i2c("oled-ssd1309");
    d.command(0x8D, &[0x14]);
    d.data(&[0x81]);
    let fb = d.fb();
    assert_eq!(fb[0x40], 0x81, "14h decoded as Set Higher Column Start");
    assert_eq!(fb.iter().filter(|&&b| b != 0).count(), 1);
}

/// 21h on the SSD1309 is an eight-bit field with range 0-127 (Table 9-3), so
/// 0x80 clamps to 127 — a seven-bit mask would have made it column 0; 22h's
/// three-bit page field IS masked (0x09 → page 1).
#[test]
fn ssd1309_column_clamps_and_page_masks() {
    let mut d = i2c("oled-ssd1309");
    d.command(0x20, &[0x00]);
    d.command(0x21, &[0x80, 0x80]);
    d.command(0x22, &[0x09, 0x09]);
    d.data(&[0x42]);
    let fb = d.fb();
    assert_eq!(fb[128 + 127], 0x42);
    assert_eq!(fb.iter().filter(|&&b| b != 0).count(), 1);
}

/// The default I²C address is the SA0 = low one, 0111100b.
#[test]
fn ssd1309_default_address_is_0x3c() {
    assert_eq!(i2c("oled-ssd1309").0.address(), 0x3C);
}
