// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **SSD1306 and ST7789: the YAML descriptors against the Rust models they
//! replace.**
//!
//! Two hand-written panel models were deleted when the `display` primitive
//! landed. This file is the evidence that nothing moved, and — in the one place
//! something did — that the change is deliberate, named, and asserted in both
//! directions so it cannot be undone or re-broken silently. The template is
//! `vl53l0x_migration_parity.rs`.
//!
//! The old models live on verbatim in `display_oracle/`; see that module's note
//! for why a copy of the model beats a table of golden bytes. Both
//! implementations are driven by the SAME script through the shared harness
//! (`common/transcript.rs`), and three things are compared, not one:
//!
//!   * the WIRE TRANSCRIPT — everything the device put back on the bus;
//!   * the WHOLE FRAMEBUFFER, byte for byte, not a checksum and not a count;
//!   * every field of the paint artifact's `meta`, which is what `painted_bytes`,
//!     the CLI's `painted bytes=` line and the browser's panel overlay all read.
//!
//! ── THE ONE DELIBERATE DIFFERENCE ───────────────────────────────────────────
//!
//! `SETMEMORYMODE` (SSD1306 0x20) used to apply the WRONG BYTE.
//!
//! The old model buffered command parameters in a fixed `[u8; 2]` indexed as
//! `2 - remaining`. A two-parameter command (0x21 SETCOLUMNADDR, 0x22
//! SETPAGEADDR) therefore filled slots 0 and 1, but a ONE-parameter command
//! filled slot 1 only — and `0x20`'s completion read slot **0**. So the
//! addressing mode was taken from whatever byte a previous 0x21/0x22 had left
//! behind, or from 0 at power-on.
//!
//! That is invisible for `0x20 0x00` (horizontal), which is what Adafruit_SSD1306
//! and every shipped lab here send, and it is why no test caught it: mode 0 was
//! selected by accident with exactly the same result. `0x20 0x01` (vertical) and
//! `0x20 0x02` (page) silently stayed horizontal, so a driver that streams a
//! frame column-major painted it transposed and nothing said so.
//!
//! The YAML model reads parameter 0, which is what the datasheet's A[1:0] means.
//! [`ssd1306_vertical_addressing_is_fixed_and_the_old_model_was_wrong`] pins
//! BOTH sides of that: the oracle is asserted to get it wrong and the descriptor
//! to get it right. Deleting either half of that assertion is the only way to
//! lose the fix.

mod common;
mod display_oracle;

use common::transcript::{dc_command, dc_data, run_i2c, run_spi, script, Step, Transcript};
use display_oracle::ili9341::Ili9341 as OldIli9341;
use display_oracle::pcd8544::Pcd8544 as OldPcd8544;
use display_oracle::rm67162::Rm67162 as OldRm67162;
use display_oracle::sh1107::Sh1107 as OldSh1107;
use display_oracle::ssd1306::Ssd1306 as OldSsd1306;
use display_oracle::st7789::St7789 as OldSt7789;
use labwired_core::inspect::{Artifact, InspectOpts};
use labwired_core::peripherals::components::GenericDisplay;
use labwired_core::peripherals::i2c::I2cDevice;
use labwired_core::peripherals::spi::SpiDevice;

const ADDR: u8 = 0x3C;
const CS: &str = "PA4";
const DC: &str = "PB0";

fn opts() -> InspectOpts {
    InspectOpts {
        include_bytes: true,
        peripheral: None,
    }
}

fn new_ssd1306() -> GenericDisplay {
    labwired_core::peripherals::components::ssd1306(ADDR)
}

fn new_st7789() -> GenericDisplay {
    labwired_core::peripherals::components::st7789(CS, DC)
}

/// Compare two artifacts field by field, so a failure names WHICH field moved
/// rather than printing two 150 KB structs side by side.
fn assert_same_artifact(old: &Artifact, new: &Artifact, what: &str) {
    assert_eq!(old.kind, new.kind, "{what}: artifact kind");
    assert_eq!(old.id, new.id, "{what}: artifact id");
    let (o, n) = (
        old.meta.as_object().expect("old meta is an object"),
        new.meta.as_object().expect("new meta is an object"),
    );
    let mut keys: Vec<&String> = o.keys().chain(n.keys()).collect();
    keys.sort();
    keys.dedup();
    for k in keys {
        assert_eq!(
            o.get(k),
            n.get(k),
            "{what}: artifact meta['{k}'] differs — the Rust model said {:?}, the descriptor {:?}",
            o.get(k),
            n.get(k)
        );
    }
    assert_eq!(
        old.bytes.as_ref().map(|b| b.len()),
        new.bytes.as_ref().map(|b| b.len()),
        "{what}: artifact payload length"
    );
    if let (Some(a), Some(b)) = (&old.bytes, &new.bytes) {
        if a != b {
            let first = a
                .iter()
                .zip(b.iter())
                .position(|(x, y)| x != y)
                .expect("lengths equal and contents differ");
            panic!(
                "{what}: artifact payload differs at byte {first}: Rust model 0x{:02X}, \
                 descriptor 0x{:02X}",
                a[first], b[first]
            );
        }
    }
}

// ─── SSD1306 ───────────────────────────────────────────────────────────────

/// The init burst every Adafruit_SSD1306-shaped driver sends, split the way the
/// nRF52 TWIM splits it: one START per transfer, one STOP at the very end. Each
/// command is its own `0x00`-prefixed transfer, which is precisely the framing
/// that used to lose its control byte.
fn ssd1306_init() -> Vec<Step<'static>> {
    let mut steps = Vec::new();
    for burst in [
        [0xAEu8].as_slice(),
        &[0xD5, 0x80],
        &[0xA8, 0x3F],
        &[0xD3, 0x00],
        &[0x40],
        &[0x8D, 0x14],
        &[0x20, 0x00],
        &[0xA1],
        &[0xC8],
        &[0xDA, 0x12],
        &[0x81, 0xCF],
        &[0xD9, 0xF1],
        &[0xDB, 0x40],
        &[0xA4],
        &[0xA6],
        &[0x2E],
        &[0xAF],
    ] {
        steps.push(Step::Start);
        steps.push(Step::Write(0x00));
        steps.extend(burst.iter().map(|&b| Step::Write(b)));
    }
    steps
}

/// Window the full 128×64 panel and stream `bytes` as pixel data.
fn ssd1306_frame(bytes: &[u8]) -> Vec<Step<'static>> {
    let mut steps = vec![
        Step::Start,
        Step::Write(0x00),
        Step::Write(0x21),
        Step::Write(0x00),
        Step::Write(0x7F),
        Step::Start,
        Step::Write(0x00),
        Step::Write(0x22),
        Step::Write(0x00),
        Step::Write(0x07),
        Step::Start,
        Step::Write(0x40),
    ];
    steps.extend(bytes.iter().map(|&b| Step::Write(b)));
    steps.push(Step::Stop);
    steps
}

/// A full 1024-byte frame, so the wrap at the column end and the wrap at the
/// page end are both exercised rather than assumed.
fn ssd1306_full_frame() -> Vec<u8> {
    (0..1024u32)
        .map(|i| (i.wrapping_mul(37) ^ 0x5A) as u8)
        .collect()
}

fn drive_both_i2c(steps: &[Step<'_>]) -> (OldSsd1306, GenericDisplay, Transcript, Transcript) {
    let mut old = OldSsd1306::new(ADDR);
    let mut new = new_ssd1306();
    let t_old = run_i2c(&mut old, steps);
    let t_new = run_i2c(&mut new, steps);
    (old, new, t_old, t_new)
}

#[test]
fn ssd1306_init_and_a_full_frame_are_byte_identical() {
    let frame = ssd1306_full_frame();
    let steps = script([ssd1306_init(), ssd1306_frame(&frame)]);
    let (old, new, t_old, t_new) = drive_both_i2c(&steps);

    assert_eq!(t_old, t_new, "wire transcript");
    assert_eq!(
        old.framebuffer(),
        new.framebuffer(),
        "GDDRAM differs after a full frame"
    );
    // Not a tautology against an all-zero buffer: the frame is 1024 pseudo-random
    // bytes and lands in full.
    assert_eq!(
        new.framebuffer(),
        &frame[..],
        "the frame did not land intact"
    );
    assert_same_artifact(
        &I2cDevice::artifacts(&old, "oled", &opts())[0],
        &I2cDevice::artifacts(&new, "oled", &opts())[0],
        "ssd1306 full frame",
    );
}

/// The nRF52 TWIM shape: several transfers, ONE trailing STOP. Every transfer
/// after the first must re-read its control byte, or the `0x40` in front of the
/// framebuffer is decoded as "set display start line" and 1024 pixel bytes are
/// parsed as commands — a panel shifted 49 columns with its rows in the wrong
/// pages, which is worse than a blank one because it looks like it works.
#[test]
fn ssd1306_transfers_under_one_stop_keep_their_control_bytes() {
    let steps = script([
        ssd1306_init(),
        ssd1306_frame(&[0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87]),
    ]);
    let (old, new, _, _) = drive_both_i2c(&steps);
    assert_eq!(
        &new.framebuffer()[..8],
        &[0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87],
        "data must land at column 0 of page 0, not wherever a mis-parsed control \
         byte moved the cursor"
    );
    assert_eq!(old.framebuffer(), new.framebuffer());
}

/// A split init must not leave the cursor anywhere but column 0, and an init
/// parameter must never be read as a column-nibble command.
#[test]
fn ssd1306_split_init_does_not_shift_the_framebuffer() {
    let steps = script([ssd1306_init(), ssd1306_frame(&[0xAA])]);
    let (old, new, _, _) = drive_both_i2c(&steps);
    assert_eq!(new.framebuffer()[0], 0xAA);
    assert_eq!(new.framebuffer()[39], 0);
    assert_eq!(old.framebuffer(), new.framebuffer());
}

/// Page addressing: 0xB3 selects page 3, then the two column-nibble commands
/// place the cursor at column 0x25. The old model and the descriptor must agree
/// on where the byte lands and on the clamp at the last column.
#[test]
fn ssd1306_page_addressing_and_column_nibbles_agree() {
    let mut steps = vec![Step::Start, Step::Write(0x00)];
    for b in [0x20u8, 0x02] {
        steps.push(Step::Write(b));
    }
    steps.push(Step::Start);
    steps.push(Step::Write(0x00));
    for b in [0xB3u8, 0x05, 0x12] {
        steps.push(Step::Write(b));
    }
    steps.push(Step::Start);
    steps.push(Step::Write(0x40));
    for b in [0x11u8, 0x22, 0x33] {
        steps.push(Step::Write(b));
    }
    steps.push(Step::Stop);

    let (old, new, t_old, t_new) = drive_both_i2c(&steps);
    assert_eq!(t_old, t_new);
    assert_eq!(old.framebuffer(), new.framebuffer());
    // Page 3, column 0x25 — and NOT wherever a mis-decoded nibble would put it.
    let base = 3 * 128 + 0x25;
    assert_eq!(
        &new.framebuffer()[base..base + 3],
        &[0x11, 0x22, 0x33],
        "page addressing put the bytes somewhere else"
    );
}

/// ⚠️ THE ONE DELIBERATE DIFFERENCE — see the module note.
///
/// Both halves are asserted. The oracle is pinned to the wrong behaviour so
/// nobody can "fix" the copy and quietly make this test vacuous; the descriptor
/// is pinned to the right one so the fix cannot regress.
#[test]
fn ssd1306_vertical_addressing_is_fixed_and_the_old_model_was_wrong() {
    // Vertical addressing over a 2-column × 2-page window: bytes walk DOWN the
    // pages first. Horizontal addressing would walk across the columns.
    let steps = script([vec![
        Step::Start,
        Step::Write(0x00),
        Step::Write(0x21),
        Step::Write(0x00),
        Step::Write(0x01),
        Step::Start,
        Step::Write(0x00),
        Step::Write(0x22),
        Step::Write(0x00),
        Step::Write(0x01),
        // 0x20 0x01 = vertical.
        Step::Start,
        Step::Write(0x00),
        Step::Write(0x20),
        Step::Write(0x01),
        Step::Start,
        Step::Write(0x40),
        Step::Write(0xA1),
        Step::Write(0xA2),
        Step::Write(0xA3),
        Step::Write(0xA4),
        Step::Stop,
    ]]);
    let (old, new, _, _) = drive_both_i2c(&steps);

    // The descriptor: vertical. (0,0) (0,p1) (1,0) (1,p1).
    assert_eq!(
        [
            new.framebuffer()[0],
            new.framebuffer()[128],
            new.framebuffer()[1],
            new.framebuffer()[129],
        ],
        [0xA1, 0xA2, 0xA3, 0xA4],
        "0x20 0x01 must select VERTICAL addressing — the descriptor reads parameter 0, \
         which is what the datasheet's A[1:0] is"
    );

    // The old model: horizontal, because SETMEMORYMODE read the wrong buffer
    // slot. (0,0) (1,0) (0,p1) (1,p1).
    assert_eq!(
        [
            old.framebuffer()[0],
            old.framebuffer()[1],
            old.framebuffer()[128],
            old.framebuffer()[129],
        ],
        [0xA1, 0xA2, 0xA3, 0xA4],
        "the deleted model is expected to get this WRONG (it applied parameter slot 0, \
         which a one-parameter command never filled). If this assertion fails the oracle \
         was edited, and every parity claim in this file is worth less than it reads."
    );
    assert_ne!(
        old.framebuffer(),
        new.framebuffer(),
        "the two must actually disagree here — an equal pair would mean the script \
         never reached the vertical path and this test proves nothing"
    );
}

/// The 0.91″ panel: four pages, half the GDDRAM, same command table.
#[test]
fn ssd1306_128x32_matches_the_old_four_page_variant() {
    let mut steps = vec![Step::Start, Step::Write(0x00)];
    for b in [0x20u8, 0x00, 0x21, 0x00, 0x7F, 0x22, 0x00, 0x03] {
        steps.push(Step::Write(b));
    }
    steps.push(Step::Start);
    steps.push(Step::Write(0x00));
    for b in [0xB3u8, 0x00, 0x10] {
        steps.push(Step::Write(b));
    }
    steps.push(Step::Start);
    steps.push(Step::Write(0x40));
    steps.push(Step::Write(0xFF));
    steps.push(Step::Stop);

    let mut old = OldSsd1306::new_128x32(ADDR);
    let mut new = labwired_core::peripherals::components::ssd1306_128x32(ADDR);
    assert_eq!(run_i2c(&mut old, &steps), run_i2c(&mut new, &steps));
    assert_eq!(new.framebuffer().len(), 128 * 4, "128×32 GDDRAM is 4 pages");
    assert_eq!(old.framebuffer(), new.framebuffer());
    assert_eq!(
        new.framebuffer()[3 * 128],
        0xFF,
        "page 3 (rows 24..31) must be addressable on the 128×32 panel"
    );
    assert_same_artifact(
        &I2cDevice::artifacts(&old, "oled", &opts())[0],
        &I2cDevice::artifacts(&new, "oled", &opts())[0],
        "ssd1306 128x32",
    );
}

// ─── ST7789 ────────────────────────────────────────────────────────────────

fn st7789_window(xs: u16, xe: u16, ys: u16, ye: u16) -> Vec<Step<'static>> {
    script([
        dc_command(
            0x2A,
            &[(xs >> 8) as u8, xs as u8, (xe >> 8) as u8, xe as u8],
        ),
        dc_command(
            0x2B,
            &[(ys >> 8) as u8, ys as u8, (ye >> 8) as u8, ye as u8],
        ),
    ])
}

fn st7789_pixels(px: &[u16]) -> Vec<Step<'static>> {
    let bytes: Vec<u8> = px
        .iter()
        .flat_map(|p| [(p >> 8) as u8, (p & 0xFF) as u8])
        .collect();
    script([dc_command(0x2C, &[]), dc_data(&bytes)])
}

fn drive_both_spi(steps: &[Step<'_>]) -> (OldSt7789, GenericDisplay, Transcript, Transcript) {
    let mut old = OldSt7789::new(CS).with_dc_pin(DC);
    let mut new = new_st7789();
    let t_old = run_spi(&mut old, steps);
    let t_new = run_spi(&mut new, steps);
    (old, new, t_old, t_new)
}

/// The full init a real ST7789 driver sends, including the multi-parameter
/// commands this model does NOT decode (0xB2, 0xB7, 0xBB, 0xC0…, 0xE0/0xE1).
/// Those parameters must be consumed as strays, not written into frame memory:
/// the 0x2C hiding inside a gamma table is the exact byte that a
/// framing-by-value model decodes as RAMWR.
fn st7789_init() -> Vec<Step<'static>> {
    script([
        dc_command(0x01, &[]),
        dc_command(0x11, &[]),
        dc_command(0x3A, &[0x55]),
        dc_command(0x36, &[0x00]),
        dc_command(0xB2, &[0x0C, 0x0C, 0x00, 0x33, 0x33]),
        dc_command(0xB7, &[0x35]),
        dc_command(0xBB, &[0x19]),
        dc_command(0xC0, &[0x2C]),
        dc_command(0xC2, &[0x01]),
        dc_command(0xC3, &[0x12]),
        dc_command(0xC4, &[0x20]),
        dc_command(0xC6, &[0x0F]),
        dc_command(0xD0, &[0xA4, 0xA1]),
        dc_command(
            0xE0,
            &[
                0xD0, 0x04, 0x0D, 0x11, 0x13, 0x2B, 0x3F, 0x54, 0x4C, 0x18, 0x0D, 0x0B, 0x1F, 0x23,
            ],
        ),
        dc_command(0x21, &[]),
        dc_command(0x13, &[]),
        dc_command(0x29, &[]),
    ])
}

#[test]
fn st7789_init_and_a_painted_window_are_byte_identical() {
    // A 32×32 block of white: 1024 pixels, 2048 painted bytes.
    let px: Vec<u16> = vec![0xFFFF; 32 * 32];
    let steps = script([
        vec![Step::CsSelect],
        st7789_init(),
        st7789_window(0, 31, 0, 31),
        st7789_pixels(&px),
        vec![Step::CsRelease],
    ]);
    let (old, new, t_old, t_new) = drive_both_spi(&steps);

    assert_eq!(t_old, t_new, "wire transcript");
    assert_eq!(
        old.framebuffer(),
        new.framebuffer(),
        "frame memory differs after a painted window"
    );
    let art_new = SpiDevice::artifacts(&new, "tft", &opts());
    assert_eq!(
        art_new[0].meta["painted_bytes"], 2048,
        "1024 pixels of 0xFFFF — a parity test that compared two blank panels \
         would pass while painting nothing"
    );
    assert_same_artifact(
        &SpiDevice::artifacts(&old, "tft", &opts())[0],
        &art_new[0],
        "st7789 painted window",
    );
}

/// The bug the D/C-only framing exists for: a parameter byte of 0x2C must not
/// open the pixel stream. `dc_command(0xC0, &[0x2C])` sends exactly that.
#[test]
fn st7789_a_parameter_byte_of_2c_is_not_decoded_as_ramwr() {
    let steps = script([
        vec![Step::CsSelect],
        dc_command(0x11, &[]),
        dc_command(0xC0, &[0x2C]),
        dc_data(&[0xFF, 0xFF, 0xFF, 0xFF]),
        vec![Step::CsRelease],
    ]);
    let (old, new, _, _) = drive_both_spi(&steps);
    assert!(
        new.framebuffer().iter().all(|&b| b == 0),
        "bytes after an unmodelled command's parameter landed in frame memory"
    );
    assert_eq!(old.framebuffer(), new.framebuffer());
}

/// MADCTL MV exchanges the axes, so a 320-column window becomes legal and the
/// pixels land rotated in physical memory.
#[test]
fn st7789_madctl_orientation_matches() {
    for madctl in [0x00u8, 0x20, 0x40, 0x80, 0x60, 0xC0, 0xE0] {
        let steps = script([
            vec![Step::CsSelect],
            dc_command(0x11, &[]),
            dc_command(0x29, &[]),
            dc_command(0x36, &[madctl]),
            st7789_window(0, 319, 0, 239),
            // A dominant colour, deliberately: a four-way tie in the colour
            // histogram is the ONE place the two models differ, and it has its own
            // test below rather than being smuggled in here.
            st7789_pixels(&[0xF800, 0xF800, 0xF800, 0x07E0]),
            vec![Step::CsRelease],
        ]);
        let (old, new, t_old, t_new) = drive_both_spi(&steps);
        assert_eq!(t_old, t_new, "MADCTL 0x{madctl:02X}: wire transcript");
        assert_eq!(
            old.framebuffer(),
            new.framebuffer(),
            "MADCTL 0x{madctl:02X}: frame memory"
        );
        assert_same_artifact(
            &SpiDevice::artifacts(&old, "tft", &opts())[0],
            &SpiDevice::artifacts(&new, "tft", &opts())[0],
            &format!("st7789 MADCTL 0x{madctl:02X}"),
        );
    }
}

/// ⚠️ A SECOND DELIBERATE DIFFERENCE, and the only other one: `top_colour` on a
/// TIE.
///
/// The old model counted colours in a `HashMap` and took `max_by_key`. With two
/// or more colours at the same pixel count, which one wins depends on the hash
/// iteration order — it is not stable between runs, between a native build and
/// the wasm build, or between two machines. That is a nondeterministic field in
/// an artifact whose entire purpose is byte-exact comparison, and the browser
/// prints it as "dominant colour" beside a picture.
///
/// The descriptor counts in a `BTreeMap`, so a tie resolves to the HIGHEST
/// RGB565 value, the same way every time and everywhere. The untied case — every
/// other test in this file, and every real frame, which has a background — is
/// unchanged.
///
/// This test does not assert what the old model returned (it cannot: that is the
/// point). It asserts that the new one is stable and picks the documented
/// winner.
#[test]
fn st7789_top_colour_breaks_a_tie_deterministically() {
    let paint = |px: &[u16]| {
        let steps = script([
            vec![Step::CsSelect],
            dc_command(0x11, &[]),
            st7789_window(0, (px.len() - 1) as u16, 0, 0),
            st7789_pixels(px),
            vec![Step::CsRelease],
        ]);
        let mut dev = new_st7789();
        run_spi(&mut dev, &steps);
        SpiDevice::artifacts(&dev, "tft", &opts())[0].meta.clone()
    };

    // Four colours, one pixel each: a four-way tie.
    let tied = [0xF800u16, 0x07E0, 0x001F, 0x1234];
    let meta = paint(&tied);
    assert_eq!(
        meta["top_colour"], "0xF800",
        "a tie must resolve to the highest RGB565 value, the same way on every run"
    );
    assert_eq!(meta["top_colour_pixels"], 1);

    // Same four colours in a different order: the same answer, because the
    // tie-break is on the VALUE and not on insertion or hash order.
    let reordered = [0x1234u16, 0x001F, 0xF800, 0x07E0];
    assert_eq!(paint(&reordered)["top_colour"], "0xF800");

    // And an untied frame still reports the actual majority, which is the case
    // every real picture is.
    let majority = [0x001Fu16, 0x001F, 0x001F, 0xF800];
    let meta = paint(&majority);
    assert_eq!(meta["top_colour"], "0x001F");
    assert_eq!(meta["top_colour_pixels"], 3);
}

/// WRMEMC (0x3C) continues from where the last write stopped; RAMWR (0x2C)
/// rewinds to the window start. Getting these the same way round is the
/// difference between a scrolling console and one that overwrites line 1.
#[test]
fn st7789_wrmemc_continues_where_ramwr_rewinds() {
    let steps = script([
        vec![Step::CsSelect],
        dc_command(0x11, &[]),
        st7789_window(0, 3, 0, 0),
        st7789_pixels(&[0x1111, 0x2222]),
        // WRMEMC: the next two pixels must land at columns 2 and 3.
        dc_command(0x3C, &[]),
        dc_data(&[0x33, 0x33, 0x44, 0x44]),
        vec![Step::CsRelease],
    ]);
    let (old, new, _, _) = drive_both_spi(&steps);
    let fb = new.framebuffer();
    let px = |x: usize| u16::from_be_bytes([fb[x * 2], fb[x * 2 + 1]]);
    assert_eq!(
        [px(0), px(1), px(2), px(3)],
        [0x1111, 0x2222, 0x3333, 0x4444],
        "WRMEMC rewound instead of continuing"
    );
    assert_eq!(old.framebuffer(), new.framebuffer());
}

/// SWRESET returns the control state to power-on and KEEPS frame memory —
/// §9.1.22 p.202, "Contents of memory is not cleared".
#[test]
fn st7789_swreset_keeps_frame_memory_and_clears_control_state() {
    let steps = script([
        vec![Step::CsSelect],
        dc_command(0x11, &[]),
        dc_command(0x29, &[]),
        dc_command(0x21, &[]),
        dc_command(0x36, &[0x60]),
        st7789_window(0, 1, 0, 0),
        // One colour twice: no histogram tie, so this test measures SWRESET and
        // nothing else (the tie-break difference has its own test).
        st7789_pixels(&[0xABCD, 0xABCD]),
        dc_command(0x01, &[]),
        vec![Step::CsRelease],
    ]);
    let (old, new, _, _) = drive_both_spi(&steps);
    assert_eq!(old.framebuffer(), new.framebuffer());
    let art = &SpiDevice::artifacts(&new, "tft", &opts())[0];
    assert_eq!(art.meta["painted_bytes"], 4, "SWRESET erased frame memory");
    assert_eq!(art.meta["display_on"], false);
    assert_eq!(art.meta["awake"], false);
    assert_eq!(art.meta["inverted"], false);
    assert_eq!(
        art.meta["w"], 240,
        "SWRESET must clear MADCTL back to portrait"
    );
    assert_same_artifact(
        &SpiDevice::artifacts(&old, "tft", &opts())[0],
        art,
        "st7789 after SWRESET",
    );
}

/// DISPON without SLPOUT is not a lit panel, and the artifact must say so.
#[test]
fn st7789_dispon_without_slpout_is_not_lit() {
    let steps = script([
        vec![Step::CsSelect],
        dc_command(0x29, &[]),
        st7789_pixels(&[0x07E0]),
        vec![Step::CsRelease],
    ]);
    let (old, new, _, _) = drive_both_spi(&steps);
    let art = &SpiDevice::artifacts(&new, "tft", &opts())[0];
    assert_eq!(art.meta["display_on"], true);
    assert_eq!(art.meta["awake"], false);
    assert_eq!(art.meta["lit"], false, "DISPON alone must not read as lit");
    assert_eq!(art.meta["painted_bytes"], 2, "the pixel still landed");
    assert_same_artifact(
        &SpiDevice::artifacts(&old, "tft", &opts())[0],
        art,
        "st7789 dispon without slpout",
    );
}

/// INVON/INVOFF are recorded, never applied to the stored bytes.
#[test]
fn st7789_inversion_is_tracked_and_not_baked_into_the_pixels() {
    for (cmd, expected) in [(0x21u8, true), (0x20, false)] {
        let steps = script([
            vec![Step::CsSelect],
            dc_command(0x11, &[]),
            dc_command(cmd, &[]),
            st7789_pixels(&[0x07E0]),
            vec![Step::CsRelease],
        ]);
        let (old, new, _, _) = drive_both_spi(&steps);
        let art = &SpiDevice::artifacts(&new, "tft", &opts())[0];
        assert_eq!(art.meta["inverted"], expected);
        assert_eq!(
            art.meta["top_colour"], "0x07E0",
            "inversion must not rewrite the stored pixel"
        );
        assert_same_artifact(
            &SpiDevice::artifacts(&old, "tft", &opts())[0],
            art,
            "st7789 inversion",
        );
    }
}

/// An unpowered module ignores the bus entirely and reports dark on every
/// field. Nothing accumulates, however long firmware clocks at it.
#[test]
fn st7789_unpowered_ignores_the_bus_on_both_models() {
    let steps = script([
        vec![Step::CsSelect],
        st7789_init(),
        st7789_window(0, 31, 0, 31),
        st7789_pixels(&vec![0xFFFF; 32 * 32]),
        vec![Step::CsRelease],
    ]);
    let mut old = OldSt7789::new(CS).with_dc_pin(DC).with_powered(false);
    let mut new = new_st7789();
    new.set_powered(false);
    assert_eq!(run_spi(&mut old, &steps), run_spi(&mut new, &steps));

    let art = &SpiDevice::artifacts(&new, "tft", &opts())[0];
    assert_eq!(art.meta["powered"], false);
    assert_eq!(art.meta["lit"], false);
    assert_eq!(art.meta["display_on"], false);
    assert_eq!(art.meta["awake"], false);
    assert_eq!(art.meta["painted_bytes"], 0, "no supply, no paint");
    assert_same_artifact(
        &SpiDevice::artifacts(&old, "tft", &opts())[0],
        art,
        "st7789 unpowered",
    );
}

/// Absent supply information means POWERED. The emitter writes `powered: false`
/// and nothing else, so an absent key must not darken every hand-written lab.
#[test]
fn st7789_absent_supply_information_means_powered() {
    assert!(new_st7789().powered());
    assert!(OldSt7789::new(CS).with_dc_pin(DC).powered());
}

/// A declared glass crops the ARTIFACT and moves nothing in frame memory, and
/// its corners stay put under every orientation.
#[test]
fn st7789_glass_crop_matches_for_every_orientation() {
    use labwired_core::peripherals::components::declarative_display::GlassWindow;
    for madctl in [0x00u8, 0x20, 0x40, 0x80, 0xC0] {
        let steps = script([
            vec![Step::CsSelect],
            dc_command(0x11, &[]),
            dc_command(0x29, &[]),
            dc_command(0x36, &[madctl]),
            st7789_window(0, 9, 0, 9),
            st7789_pixels(&[0x07E0; 100]),
            vec![Step::CsRelease],
        ]);
        let mut old = OldSt7789::new(CS).with_dc_pin(DC).with_visible_window(
            display_oracle::st7789::VisibleWindow {
                col_offset: 35,
                row_offset: 0,
                cols: 170,
                rows: 320,
            },
        );
        let mut new = new_st7789();
        new.set_glass_window(GlassWindow {
            col_offset: 35,
            row_offset: 0,
            cols: 170,
            rows: 320,
        });
        assert_eq!(run_spi(&mut old, &steps), run_spi(&mut new, &steps));
        let art = &SpiDevice::artifacts(&new, "tft", &opts())[0];
        assert_eq!(art.meta["w"], 170);
        assert_eq!(art.meta["h"], 320);
        assert_eq!(art.meta["total_bytes"], 170 * 320 * 2);
        assert_same_artifact(
            &SpiDevice::artifacts(&old, "tft", &opts())[0],
            art,
            &format!("st7789 glass crop, MADCTL 0x{madctl:02X}"),
        );
    }
}

// ─── SH1107 ────────────────────────────────────────────────────────────────

fn new_sh1107() -> GenericDisplay {
    labwired_core::peripherals::components::sh1107(ADDR)
}

fn drive_both_sh1107(steps: &[Step<'_>]) -> (OldSh1107, GenericDisplay, Transcript, Transcript) {
    let mut old = OldSh1107::new(ADDR);
    let mut new = new_sh1107();
    let t_old = run_i2c(&mut old, steps);
    let t_new = run_i2c(&mut new, steps);
    (old, new, t_old, t_new)
}

/// The Adafruit_SH110X-shaped init burst, split the way a TWIM splits it: one
/// START per transfer, one STOP at the very end. Eight of these commands carry
/// a parameter; if any parameter were decoded as an opcode the cursor would
/// move and the first frame would land shifted.
fn sh1107_init() -> Vec<Step<'static>> {
    let mut steps = Vec::new();
    for burst in [
        [0xAEu8].as_slice(),
        &[0xD5, 0x51],
        &[0x81, 0x4F],
        &[0xAD, 0x8A],
        &[0xA8, 0x7F],
        &[0xD3, 0x60],
        &[0xDC, 0x00],
        &[0xD9, 0x22],
        &[0xDB, 0x35],
        &[0xA4],
        &[0xA6],
        &[0xAF],
    ] {
        steps.push(Step::Start);
        steps.push(Step::Write(0x00));
        steps.extend(burst.iter().map(|&b| Step::Write(b)));
    }
    steps
}

/// Move the cursor with the three commands that do it: page, column low
/// nibble, column high nibble.
fn sh1107_cursor(page: u8, col: u8) -> Vec<Step<'static>> {
    vec![
        Step::Start,
        Step::Write(0x00),
        Step::Write(0xB0 | (page & 0x0F)),
        Step::Write(col & 0x0F),
        Step::Write(0x10 | ((col >> 4) & 0x07)),
    ]
}

fn sh1107_data(bytes: &[u8]) -> Vec<Step<'_>> {
    let mut steps = vec![Step::Start, Step::Write(0x40)];
    steps.extend(bytes.iter().map(|&b| Step::Write(b)));
    steps.push(Step::Stop);
    steps
}

/// A whole 2048-byte GDDRAM, so both the page wrap and the column wrap of
/// VERTICAL addressing are exercised rather than assumed.
fn sh1107_full_frame() -> Vec<u8> {
    (0..2048u32)
        .map(|i| (i.wrapping_mul(29) ^ 0xA5) as u8)
        .collect()
}

#[test]
fn sh1107_init_and_a_full_frame_are_byte_identical() {
    let frame = sh1107_full_frame();
    let steps = script([
        sh1107_init(),
        // 0x21 — vertical addressing, a COMPLETE command on this part. On the
        // SSD1306 the same byte is SETCOLUMNADDR and eats two parameters.
        vec![Step::Start, Step::Write(0x00), Step::Write(0x21)],
        sh1107_cursor(0, 0),
        sh1107_data(&frame),
    ]);
    let (old, new, t_old, t_new) = drive_both_sh1107(&steps);

    assert_eq!(t_old, t_new, "wire transcript");
    assert_eq!(
        old.framebuffer(),
        new.framebuffer(),
        "GDDRAM differs after a full frame"
    );
    // Not a tautology against an all-zero buffer: vertical addressing walks the
    // sixteen pages of a column before moving on, so byte `i` lands at page
    // `i % 16`, column `i / 16`, and all 2048 of them land.
    for (i, b) in frame.iter().enumerate() {
        let (page, col) = (i % 16, i / 16);
        assert_eq!(
            new.framebuffer()[page * 128 + col],
            *b,
            "byte {i} of a vertical-addressed full frame"
        );
    }
    assert_same_artifact(
        &I2cDevice::artifacts(&old, "oled", &opts())[0],
        &I2cDevice::artifacts(&new, "oled", &opts())[0],
        "sh1107 full frame",
    );
}

/// PAGE ADDRESSING WRAPS ON THIS PART. At column 127 the counter returns to 0
/// with the page unchanged; the SSD1306 model holds it at the last column
/// instead. `addressing.page_wrap` is what states the difference, and this is
/// the test that would go red if the descriptor took the other value.
#[test]
fn sh1107_page_addressing_wraps_the_column_where_the_ssd1306_clamps() {
    let steps = script([
        vec![Step::Start, Step::Write(0x00), Step::Write(0x20)],
        sh1107_cursor(2, 125),
        sh1107_data(&[0x11, 0x22, 0x33, 0x44, 0x55]),
    ]);
    let (old, new, t_old, t_new) = drive_both_sh1107(&steps);
    assert_eq!(t_old, t_new);
    assert_eq!(old.framebuffer(), new.framebuffer());

    let page2 = &new.framebuffer()[2 * 128..3 * 128];
    assert_eq!(
        [page2[125], page2[126], page2[127], page2[0], page2[1]],
        [0x11, 0x22, 0x33, 0x44, 0x55],
        "the column counter must wrap to 0 within page 2"
    );
    // And the neighbouring pages are untouched — a wrap, not a run-on.
    assert!(new.framebuffer()[128..256].iter().all(|&b| b == 0));
    assert!(new.framebuffer()[3 * 128..4 * 128].iter().all(|&b| b == 0));
}

/// Sixteen pages and a seven-bit column: the two geometry facts that separate
/// this part from the SSD1306. The last addressable byte is 2047.
#[test]
fn sh1107_addresses_all_sixteen_pages_and_a_seven_bit_column() {
    let steps = script([sh1107_cursor(15, 127), sh1107_data(&[0xFF])]);
    let (old, new, _, _) = drive_both_sh1107(&steps);
    assert_eq!(old.framebuffer(), new.framebuffer());
    assert_eq!(new.framebuffer()[16 * 128 - 1], 0xFF);
    assert_eq!(
        new.framebuffer().iter().filter(|&&b| b != 0).count(),
        1,
        "exactly one byte was written"
    );
}

/// 0x18..=0x1F address nothing on a seven-bit column. They must be consumed as
/// unknown opcodes, leaving the cursor where it was — not read as a fourth
/// column bit.
#[test]
fn sh1107_high_column_opcodes_stop_at_0x17() {
    let steps = script([
        sh1107_cursor(1, 0x35),
        vec![
            Step::Start,
            Step::Write(0x00),
            Step::Write(0x1B),
            Step::Write(0x1F),
        ],
        sh1107_data(&[0x7E]),
    ]);
    let (old, new, _, _) = drive_both_sh1107(&steps);
    assert_eq!(old.framebuffer(), new.framebuffer());
    assert_eq!(new.framebuffer()[128 + 0x35], 0x7E);
}

/// The artifact is the surface the browser overlay and `inspect` read: sixteen
/// pages of height, the `sh1107_page` format string, the ink counters, and
/// `display_on`. The SSD1306 publishes no `display_on`; this panel always has,
/// which is why `artifact_meta` is per-panel data rather than a format default.
#[test]
fn sh1107_artifact_keeps_its_published_shape() {
    let steps = script([
        vec![Step::Start, Step::Write(0x00), Step::Write(0xAF)],
        sh1107_cursor(0, 0),
        sh1107_data(&[0xFF, 0xFF, 0xFF]),
    ]);
    let (old, new, _, _) = drive_both_sh1107(&steps);
    let art = &I2cDevice::artifacts(&new, "oled", &opts())[0];
    assert_eq!(art.meta["format"], "sh1107_page");
    assert_eq!(art.meta["w"], 128);
    assert_eq!(art.meta["h"], 128);
    assert_eq!(art.meta["ink_bytes"], 3);
    assert_eq!(art.meta["lit_pixels"], 24);
    assert_eq!(art.meta["display_on"], true);
    assert_same_artifact(
        &I2cDevice::artifacts(&old, "oled", &opts())[0],
        art,
        "sh1107 painted artifact",
    );
}

/// A panel that never got DISPLAYON reports it, and an unpainted one reports
/// zero rather than nothing.
#[test]
fn sh1107_unpainted_panel_matches() {
    let (old, new, _, _) = drive_both_sh1107(&[]);
    let art = &I2cDevice::artifacts(&new, "oled", &opts())[0];
    assert_eq!(art.meta["ink_bytes"], 0);
    assert_eq!(art.meta["display_on"], false);
    assert_same_artifact(
        &I2cDevice::artifacts(&old, "oled", &opts())[0],
        art,
        "sh1107 unpainted",
    );
}

// ─── PCD8544 (Nokia 5110) ──────────────────────────────────────────────────

const LCD_CS: &str = "PB6";
const LCD_DC: &str = "PC7";

fn new_pcd8544() -> GenericDisplay {
    labwired_core::peripherals::components::pcd8544(LCD_CS, LCD_DC)
}

fn drive_both_pcd8544(steps: &[Step<'_>]) -> (OldPcd8544, GenericDisplay, Transcript, Transcript) {
    let mut old = OldPcd8544::new(LCD_CS.to_string(), LCD_DC.to_string());
    let mut new = new_pcd8544();
    let t_old = run_spi(&mut old, steps);
    let t_new = run_spi(&mut new, steps);
    (old, new, t_old, t_new)
}

/// Commands: D/C low, one byte each. This panel has no parameterised command,
/// so a command byte is complete in itself.
fn lcd_cmds(bytes: &[u8]) -> Vec<Step<'static>> {
    let mut steps = vec![Step::Dc(false)];
    steps.extend(bytes.iter().map(|&b| Step::TransferByte(b)));
    steps
}

/// The stock Nokia 5110 init every Adafruit-shaped driver sends. Two of these
/// bytes — `0xBF` and `0x14` — are EXTENDED-set commands whose opcodes are
/// "set X address" and nothing at all in the basic set.
fn lcd_init() -> Vec<Step<'static>> {
    lcd_cmds(&[0x21, 0xBF, 0x04, 0x14, 0x20, 0x0C])
}

fn lcd_cursor(x: u8, y: u8) -> Vec<Step<'static>> {
    lcd_cmds(&[0x40 | (y & 0x07), 0x80 | (x & 0x7F)])
}

/// A whole 504-byte DDRAM, so the column wrap and the bank wrap are both
/// exercised rather than assumed.
fn lcd_full_frame() -> Vec<u8> {
    (0..504u32)
        .map(|i| (i.wrapping_mul(53) ^ 0x3C) as u8)
        .collect()
}

#[test]
fn pcd8544_init_and_a_full_frame_are_byte_identical() {
    let frame = lcd_full_frame();
    let steps = script([
        vec![Step::CsSelect],
        lcd_init(),
        lcd_cursor(0, 0),
        dc_data(&frame),
        vec![Step::CsRelease],
    ]);
    let (old, new, t_old, t_new) = drive_both_pcd8544(&steps);

    assert_eq!(t_old, t_new, "wire transcript");
    assert_eq!(old.framebuffer(), new.framebuffer(), "DDRAM differs");
    // Not a tautology against an all-zero buffer: column-first addressing lays
    // byte `i` at bank `i / 84`, column `i % 84`, and all 504 land.
    assert_eq!(
        new.framebuffer(),
        &frame[..],
        "the frame did not land intact"
    );
    assert_same_artifact(
        &SpiDevice::artifacts(&old, "lcd", &opts())[0],
        &SpiDevice::artifacts(&new, "lcd", &opts())[0],
        "pcd8544 full frame",
    );
}

/// ⚠️ THE INSTRUCTION-SET BANK. `0xBF` after `0x21` is SET Vop, not SET X.
///
/// Read as "set X address" it would leave the column pointer at 0x3F, and the
/// first frame would land 63 columns across — a picture, in the wrong place,
/// which is the failure nobody reports as a bug. `when: { var: h, … }` is what
/// keeps both readings of `0x80|n` in one table.
#[test]
fn pcd8544_extended_set_vop_is_not_a_column_move() {
    let steps = script([
        vec![Step::CsSelect],
        lcd_init(),
        dc_data(&[0x5A]),
        vec![Step::CsRelease],
    ]);
    let (old, new, _, _) = drive_both_pcd8544(&steps);
    assert_eq!(old.framebuffer(), new.framebuffer());
    assert_eq!(
        new.framebuffer()[0],
        0x5A,
        "the byte must land at bank 0 column 0, not at column 0x3F"
    );
    assert!(
        new.framebuffer()[1..].iter().all(|&b| b == 0),
        "exactly one byte was written"
    );
}

/// The V bit of the function set. `0x22` is bank-first; the bytes walk DOWN the
/// six banks of a column before moving right.
#[test]
fn pcd8544_vertical_addressing_walks_banks_first() {
    let steps = script([
        vec![Step::CsSelect],
        lcd_cmds(&[0x22, 0x0C]),
        lcd_cursor(3, 0),
        dc_data(&[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77]),
        vec![Step::CsRelease],
    ]);
    let (old, new, _, _) = drive_both_pcd8544(&steps);
    assert_eq!(old.framebuffer(), new.framebuffer());
    let fb = new.framebuffer();
    assert_eq!(
        (0..6).map(|b| fb[b * 84 + 3]).collect::<Vec<_>>(),
        vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
        "six banks of column 3"
    );
    assert_eq!(fb[4], 0x77, "then bank 0 of column 4");
}

/// Banks 6 and 7 and columns 84..127 are out of range. The controller takes the
/// pointer to ZERO rather than clamping it at the last cell, which is why the
/// descriptor spends two extra table entries on them instead of a mask.
#[test]
fn pcd8544_out_of_range_addresses_return_to_zero() {
    for (cmds, label) in [
        (vec![0x40u8 | 6, 0x80 | 20], "bank 6"),
        (vec![0x40 | 2, 0x80 | 100], "column 100"),
    ] {
        let steps = script([
            vec![Step::CsSelect],
            lcd_cmds(&cmds),
            dc_data(&[0xC3]),
            vec![Step::CsRelease],
        ]);
        let (old, new, _, _) = drive_both_pcd8544(&steps);
        assert_eq!(old.framebuffer(), new.framebuffer(), "{label}");
        let at = new
            .framebuffer()
            .iter()
            .position(|&b| b != 0)
            .expect("something was painted");
        let expect = if label == "bank 6" { 20 } else { 2 * 84 };
        assert_eq!(at, expect, "{label}: the out-of-range half reset to 0");
    }
}

/// Every display-control encoding, both flags, through the artifact — because
/// `display_on` and `inverse` are what the browser overlay reads, and on this
/// panel `display_on` means DISPON *and* not powered down *and* supplied.
#[test]
fn pcd8544_display_control_and_power_down_flags_match() {
    for cmd in 0x08u8..=0x0F {
        for func in [0x20u8, 0x24] {
            let steps = script([
                vec![Step::CsSelect],
                lcd_cmds(&[func, cmd]),
                lcd_cursor(0, 0),
                dc_data(&[0xFF]),
                vec![Step::CsRelease],
            ]);
            let (old, new, _, _) = drive_both_pcd8544(&steps);
            assert_same_artifact(
                &SpiDevice::artifacts(&old, "lcd", &opts())[0],
                &SpiDevice::artifacts(&new, "lcd", &opts())[0],
                &format!("pcd8544 function 0x{func:02X} control 0x{cmd:02X}"),
            );
        }
    }
}

/// POWER-ON IS NOT DARK on this part: the display-control D bit and the
/// power-down bit both reset in favour of showing DDRAM. A panel that has been
/// sent nothing at all reports `display_on: true`.
#[test]
fn pcd8544_powers_on_showing_ddram() {
    let (old, new, _, _) = drive_both_pcd8544(&[]);
    let art = &SpiDevice::artifacts(&new, "lcd", &opts())[0];
    assert_eq!(art.meta["format"], "pcd8544_bank");
    assert_eq!(art.meta["w"], 84);
    assert_eq!(art.meta["h"], 48);
    assert_eq!(art.meta["display_on"], true);
    assert_eq!(art.meta["inverse"], false);
    assert_eq!(art.meta["powered"], true);
    assert_eq!(art.meta["ink_bytes"], 0);
    assert_same_artifact(
        &SpiDevice::artifacts(&old, "lcd", &opts())[0],
        art,
        "pcd8544 power-on",
    );
}

/// An unpowered module refuses the bus: DDRAM stays blank and the flags stay at
/// their dark values, by construction rather than by masking at report time.
#[test]
fn pcd8544_unpowered_module_matches() {
    let steps = script([
        vec![Step::CsSelect],
        lcd_init(),
        lcd_cursor(0, 0),
        dc_data(&[0xFF, 0xFF, 0xFF]),
        vec![Step::CsRelease],
    ]);
    let mut old = OldPcd8544::new(LCD_CS.to_string(), LCD_DC.to_string()).with_powered(false);
    let mut new = new_pcd8544();
    new.set_powered(false);
    assert_eq!(run_spi(&mut old, &steps), run_spi(&mut new, &steps));
    assert_eq!(old.framebuffer(), new.framebuffer());
    let art = &SpiDevice::artifacts(&new, "lcd", &opts())[0];
    assert_eq!(art.meta["powered"], false);
    assert_eq!(art.meta["display_on"], false);
    assert_eq!(art.meta["ink_bytes"], 0);
    assert_same_artifact(
        &SpiDevice::artifacts(&old, "lcd", &opts())[0],
        art,
        "pcd8544 unpowered",
    );
}

/// A snapshot carries the PIXELS: the save/restore door the browser's
/// state round-trip uses, kept across the port.
///
/// ⚠️ THE BLOB IS NO LONGER THE FRAME MEMORY. It used to be exactly the
/// framebuffer, and this test compared the two models' blobs byte for byte. A
/// panel now also carries its glass and its refresh counter, so the blob is a
/// TAGGED record — see `DISPLAY_SNAPSHOT_TAG`. What is kept across the port is
/// the CONTRACT, which is what this asserts: the pixels the old model saved are
/// the pixels the new one restores, and the deleted model's untagged blob is
/// refused rather than silently misread.
#[test]
fn pcd8544_runtime_snapshot_round_trips_the_pixels() {
    let frame = lcd_full_frame();
    let steps = script([
        vec![Step::CsSelect],
        lcd_init(),
        lcd_cursor(0, 0),
        dc_data(&frame),
        vec![Step::CsRelease],
    ]);
    let (old, new, _, _) = drive_both_pcd8544(&steps);
    let legacy = SpiDevice::runtime_snapshot(&old);
    let snap = SpiDevice::runtime_snapshot(&new);

    let mut fresh = new_pcd8544();
    SpiDevice::restore_runtime_snapshot(&mut fresh, &snap).expect("restore");
    assert_eq!(fresh.framebuffer(), new.framebuffer());
    assert_eq!(
        fresh.framebuffer(),
        old.framebuffer(),
        "and the restored pixels are the deleted model's pixels",
    );

    assert_eq!(
        legacy,
        old.framebuffer(),
        "the deleted model's blob WAS the frame memory — that is what is refused below",
    );
    SpiDevice::restore_runtime_snapshot(&mut fresh, &legacy)
        .expect_err("an untagged blob must be refused, not restored as if it were tagged");
}

// ─── ILI9341 ───────────────────────────────────────────────────────────────

fn new_ili9341() -> GenericDisplay {
    labwired_core::peripherals::components::ili9341(CS, DC)
}

fn old_ili9341() -> OldIli9341 {
    OldIli9341::new(CS.to_string()).with_dc_pin(DC)
}

fn drive_both_ili9341(steps: &[Step<'_>]) -> (OldIli9341, GenericDisplay, Transcript, Transcript) {
    let mut old = old_ili9341();
    let mut new = new_ili9341();
    let t_old = run_spi(&mut old, steps);
    let t_new = run_spi(&mut new, steps);
    (old, new, t_old, t_new)
}

fn ili_window(cs: u16, ce: u16, rs: u16, re: u16) -> Vec<Step<'static>> {
    script([
        dc_command(
            0x2A,
            &[(cs >> 8) as u8, cs as u8, (ce >> 8) as u8, ce as u8],
        ),
        dc_command(
            0x2B,
            &[(rs >> 8) as u8, rs as u8, (re >> 8) as u8, re as u8],
        ),
    ])
}

fn ili_pixels(px: &[u16]) -> Vec<Step<'static>> {
    let mut bytes = Vec::with_capacity(px.len() * 2);
    for p in px {
        bytes.extend_from_slice(&p.to_be_bytes());
    }
    script([dc_command(0x2C, &[]), dc_data(&bytes)])
}

/// Adafruit's stock ILI9341 init, INCLUDING the undocumented 0xCB whose second
/// parameter is 0x2C. With D/C framing that byte is a parameter and nothing
/// else; an inferring model decodes it as RAMWR and paints the rest of the init
/// sequence into frame memory.
fn ili_init() -> Vec<Step<'static>> {
    script([
        dc_command(0xEF, &[0x03, 0x80, 0x02]),
        dc_command(0xCF, &[0x00, 0xC1, 0x30]),
        dc_command(0xED, &[0x64, 0x03, 0x12, 0x81]),
        dc_command(0xE8, &[0x85, 0x00, 0x78]),
        dc_command(0xCB, &[0x39, 0x2C, 0x00, 0x34, 0x02]),
        dc_command(0xF7, &[0x20]),
        dc_command(0xEA, &[0x00, 0x00]),
        dc_command(0xC0, &[0x23]),
        dc_command(0xC1, &[0x10]),
        dc_command(0xC5, &[0x3E, 0x28]),
        dc_command(0xC7, &[0x86]),
        dc_command(0x36, &[0x48]),
        dc_command(0x3A, &[0x55]),
        dc_command(0xB1, &[0x00, 0x18]),
        dc_command(0xB6, &[0x08, 0x82, 0x27]),
        dc_command(0x11, &[]),
        dc_command(0x29, &[]),
    ])
}

#[test]
fn ili9341_stock_init_and_a_frame_are_byte_identical() {
    // Pseudo-random pixels with ONE colour deliberately dominant. A frame of
    // all-distinct colours would tie every count at 1, and a tie is the one
    // place these two models deliberately disagree — see
    // `ili9341_top_colour_resolves_a_tie_deterministically`.
    let px: Vec<u16> = (0..1024u32)
        .map(|i| {
            if i % 2 == 0 {
                0x07E0
            } else {
                (i.wrapping_mul(2087) ^ 0x1234) as u16 | 0x8000
            }
        })
        .collect();
    let steps = script([
        vec![Step::CsSelect],
        ili_init(),
        ili_window(0, 31, 0, 31),
        ili_pixels(&px),
        vec![Step::CsRelease],
    ]);
    let (old, new, t_old, t_new) = drive_both_ili9341(&steps);
    assert_eq!(t_old, t_new, "wire transcript");
    assert_eq!(
        old.framebuffer(),
        new.framebuffer(),
        "frame memory differs after the stock init and a 32x32 blit"
    );
    // Not a tautology: MADCTL 0x48 sets MX, so the 32x32 block lands mirrored
    // into the right-hand edge of frame memory, and it is really there.
    assert!(
        new.framebuffer().iter().filter(|&&b| b != 0).count() > 1000,
        "the blit landed"
    );
    assert_same_artifact(
        &SpiDevice::artifacts(&old, "tft", &opts())[0],
        &SpiDevice::artifacts(&new, "tft", &opts())[0],
        "ili9341 stock init + frame",
    );
}

/// Seven MADCTL encodings, each with a window that only makes sense in that
/// orientation. MV changes what a legal column IS, so it moves the clamp as
/// well as the pixel map.
#[test]
fn ili9341_madctl_orientation_matches() {
    for madctl in [0x00u8, 0x20, 0x40, 0x60, 0x80, 0xA0, 0xC0] {
        let steps = script([
            vec![Step::CsSelect],
            dc_command(0x29, &[]),
            dc_command(0x36, &[madctl]),
            ili_window(0, 3, 0, 3),
            // Counts 7 / 5 / 3 / 1 — untied, so the dominant colour is the same
            // fact on both models.
            ili_pixels(&[
                0x07E0, 0x07E0, 0x07E0, 0x07E0, 0x07E0, 0x07E0, 0x07E0, 0xF800, 0xF800, 0xF800,
                0xF800, 0xF800, 0x001F, 0x001F, 0x001F, 0xFFFF,
            ]),
            vec![Step::CsRelease],
        ]);
        let (old, new, _, _) = drive_both_ili9341(&steps);
        assert_eq!(
            old.framebuffer(),
            new.framebuffer(),
            "frame memory differs at MADCTL 0x{madctl:02X}"
        );
        assert_same_artifact(
            &SpiDevice::artifacts(&old, "tft", &opts())[0],
            &SpiDevice::artifacts(&new, "tft", &opts())[0],
            &format!("ili9341 MADCTL 0x{madctl:02X}"),
        );
    }
}

/// A landscape window: MV lets CASET legitimately run to 319. Clamping to the
/// physical 239 folds a correct landscape image back into portrait.
#[test]
fn ili9341_landscape_window_is_not_folded_into_portrait() {
    let steps = script([
        vec![Step::CsSelect],
        dc_command(0x29, &[]),
        dc_command(0x36, &[0x20]),
        ili_window(300, 300, 0, 0),
        ili_pixels(&[0x07E0]),
        vec![Step::CsRelease],
    ]);
    let (old, new, _, _) = drive_both_ili9341(&steps);
    assert_eq!(old.framebuffer(), new.framebuffer());
    let at = new
        .framebuffer()
        .iter()
        .position(|&b| b != 0)
        .expect("something was painted");
    assert_eq!(at, 300 * 240 * 2, "logical column 300 is physical row 300");
    let art = &SpiDevice::artifacts(&new, "tft", &opts())[0];
    assert_eq!(art.meta["w"], 320);
    assert_eq!(art.meta["h"], 240);
}

/// RAMWR rewinds the counters to the window origin; RAMWRCONT (0x3C) resumes
/// where the last write stopped. Reading 0x3C as an unknown command meant those
/// pixel bytes were decoded as commands.
#[test]
fn ili9341_ramwr_rewinds_and_ramwrcont_continues() {
    let steps = script([
        vec![Step::CsSelect],
        dc_command(0x29, &[]),
        ili_window(0, 3, 0, 0),
        ili_pixels(&[0x1111, 0x2222]),
        script([dc_command(0x3C, &[]), dc_data(&[0x33, 0x33])]),
        ili_pixels(&[0xAAAA]),
        vec![Step::CsRelease],
    ]);
    let (old, new, _, _) = drive_both_ili9341(&steps);
    assert_eq!(old.framebuffer(), new.framebuffer());
    let fb = new.framebuffer();
    assert_eq!(
        [
            u16::from_be_bytes([fb[0], fb[1]]),
            u16::from_be_bytes([fb[2], fb[3]]),
            u16::from_be_bytes([fb[4], fb[5]]),
        ],
        [0xAAAA, 0x2222, 0x3333],
        "RAMWRCONT continued at pixel 2; the second RAMWR rewound to pixel 0"
    );
}

/// A CS cycle in the middle of a blit. `cs_select: keeps_stream` is what says
/// this panel resumes rather than restarting — a driver that chunks a large
/// blit releases CS between bursts.
#[test]
fn ili9341_a_cs_cycle_does_not_close_the_pixel_stream() {
    let steps = script([
        vec![Step::CsSelect],
        dc_command(0x29, &[]),
        ili_window(0, 3, 0, 0),
        ili_pixels(&[0x1111, 0x2222]),
        vec![Step::CsRelease, Step::CsSelect],
        dc_data(&[0x33, 0x33, 0x44, 0x44]),
        vec![Step::CsRelease],
    ]);
    let (old, new, _, _) = drive_both_ili9341(&steps);
    assert_eq!(old.framebuffer(), new.framebuffer());
    let fb = new.framebuffer();
    assert_eq!(
        [
            u16::from_be_bytes([fb[4], fb[5]]),
            u16::from_be_bytes([fb[6], fb[7]]),
        ],
        [0x3333, 0x4444],
        "the pixel stream survived the CS cycle and resumed at pixel 2"
    );
}

/// SWRESET keeps the picture — §8.2.2, "the Frame Memory contents are
/// unaffected by this command" — and resets the window, MADCTL and DISPON.
#[test]
fn ili9341_swreset_keeps_frame_memory() {
    let steps = script([
        vec![Step::CsSelect],
        dc_command(0x29, &[]),
        ili_window(0, 2, 0, 0),
        ili_pixels(&[0x07E0, 0x07E0, 0xF800]),
        dc_command(0x01, &[]),
        vec![Step::CsRelease],
    ]);
    let (old, new, _, _) = drive_both_ili9341(&steps);
    assert_eq!(old.framebuffer(), new.framebuffer());
    assert_eq!(
        u16::from_be_bytes([new.framebuffer()[0], new.framebuffer()[1]]),
        0x07E0,
        "SWRESET must not clear the picture"
    );
    assert_same_artifact(
        &SpiDevice::artifacts(&old, "tft", &opts())[0],
        &SpiDevice::artifacts(&new, "tft", &opts())[0],
        "ili9341 after SWRESET",
    );
}

/// ⚠️ DELIBERATE DIFFERENCE 1 — SWRESET took its window from the orientation it
/// was about to throw away.
///
/// The old model computed the reset window from `addressable_width()` BEFORE
/// zeroing MADCTL, so a SWRESET issued while landscape left the column window
/// at 0..319 with the panel back in portrait — a window wider than the
/// addressable extent, which nothing on silicon can be in. The descriptor
/// resets the window to the power-on 0..239 / 0..319 and then the orientation,
/// so the two agree afterwards.
///
/// Both halves are asserted: the oracle is pinned to the wrong window so nobody
/// can "fix" the copy and quietly make this vacuous.
#[test]
fn ili9341_swreset_window_follows_the_reset_orientation_and_the_old_model_was_wrong() {
    // Landscape, then SWRESET, then paint a row WITHOUT a new CASET.
    let steps = script([
        vec![Step::CsSelect],
        dc_command(0x36, &[0x20]),
        dc_command(0x01, &[]),
        dc_command(0x29, &[]),
        ili_pixels(&[0x07E0; 260]),
        vec![Step::CsRelease],
    ]);
    let (old, new, _, _) = drive_both_ili9341(&steps);

    // The descriptor: portrait after the reset, so column 239 is the last one
    // and pixel 240 has wrapped onto row 1.
    let fb = new.framebuffer();
    assert_eq!(
        u16::from_be_bytes([fb[239 * 2], fb[239 * 2 + 1]]),
        0x07E0,
        "the last portrait column is painted"
    );
    assert_eq!(
        u16::from_be_bytes([fb[240 * 2], fb[240 * 2 + 1]]),
        0x07E0,
        "pixel 240 wrapped onto row 1, which is what a 0..239 window does"
    );

    // The old model: a 0..319 column window left over from the orientation it
    // had just discarded, so pixels 240..259 fall off the end of each row and
    // are DROPPED instead of wrapping.
    let ofb = old.framebuffer();
    assert_eq!(
        u16::from_be_bytes([ofb[240 * 2], ofb[240 * 2 + 1]]),
        0x0000,
        "the oracle is pinned to the stale-window behaviour; if this fails the \
         oracle was edited and this test measures nothing"
    );
    assert_ne!(
        fb, ofb,
        "the two must differ here — that is the whole point of this test"
    );
}

/// ⚠️ DELIBERATE DIFFERENCE 2 — `top_colour` on a tie.
///
/// The old model counted colours in a `HashMap` and took `max_by_key`, so a tie
/// resolved by hash iteration order: not stable between runs, between native
/// and wasm, or between machines, in a field the browser prints as "dominant
/// colour". A `BTreeMap` resolves a tie to the highest RGB565 value,
/// identically everywhere. Untied frames — every real picture, which has a
/// background — are unchanged, which is what every other test in this section
/// compares. Same change the ST7789 port made.
#[test]
fn ili9341_top_colour_resolves_a_tie_deterministically() {
    let steps = script([
        vec![Step::CsSelect],
        dc_command(0x29, &[]),
        ili_window(0, 3, 0, 0),
        ili_pixels(&[0x07E0, 0x07E0, 0xF800, 0xF800]),
        vec![Step::CsRelease],
    ]);
    let (_, new, _, _) = drive_both_ili9341(&steps);
    let art = &SpiDevice::artifacts(&new, "tft", &opts())[0];
    assert_eq!(
        art.meta["top_colour"], "0xF800",
        "a tie resolves to the highest RGB565 value, on every machine"
    );
    assert_eq!(art.meta["top_colour_pixels"], 2);
}

/// An unpowered module refuses the bus, so DISPON and the pixels stay at their
/// power-on-dark values by construction.
#[test]
fn ili9341_unpowered_module_matches() {
    let steps = script([
        vec![Step::CsSelect],
        ili_init(),
        ili_window(0, 3, 0, 0),
        ili_pixels(&[0xFFFF; 4]),
        vec![Step::CsRelease],
    ]);
    let mut old = old_ili9341().with_powered(false);
    let mut new = new_ili9341();
    new.set_powered(false);
    assert_eq!(run_spi(&mut old, &steps), run_spi(&mut new, &steps));
    assert_eq!(old.framebuffer(), new.framebuffer());
    let art = &SpiDevice::artifacts(&new, "tft", &opts())[0];
    assert_eq!(art.meta["powered"], false);
    assert_eq!(art.meta["display_on"], false);
    assert_eq!(art.meta["painted_bytes"], 0);
    assert_same_artifact(
        &SpiDevice::artifacts(&old, "tft", &opts())[0],
        art,
        "ili9341 unpowered",
    );
}

/// The artifact's published shape: LOGICAL dimensions (so a landscape panel is
/// 320x240, not 240x320), and no `lit` / `awake` — this model does not act on
/// SLPOUT and never has.
#[test]
fn ili9341_artifact_keeps_its_published_shape() {
    let (_, new, _, _) = drive_both_ili9341(&[]);
    let art = &SpiDevice::artifacts(&new, "tft", &opts())[0];
    assert_eq!(art.meta["format"], "rgb565_be");
    assert_eq!(art.meta["w"], 240);
    assert_eq!(art.meta["h"], 320);
    assert_eq!(art.meta["total_bytes"], 240 * 320 * 2);
    assert!(
        art.meta.get("lit").is_none(),
        "this panel publishes no `lit`"
    );
    assert!(art.meta.get("awake").is_none());
}

// ─── RM67162 ───────────────────────────────────────────────────────────────
//
// THE AMOLED. What this panel forced into the primitive is not a command table:
// it is `lit_requires` (a numeric var folded into `lit`), `dc.source: hw_dcx`
// (the controller's own DCX line as an alternative to a firmware GPIO), and
// `artifact_meta` entries that publish a VAR — raw for `brightness`, hex for
// `colmod` and `madctl`. Each has a test below and a negative control in
// `declarative_display.rs`.

fn new_rm67162() -> GenericDisplay {
    labwired_core::peripherals::components::rm67162_hw_dcx(CS)
}

fn old_rm67162() -> OldRm67162 {
    OldRm67162::with_controller_dc(CS)
}

fn drive_both_rm67162(steps: &[Step<'_>]) -> (OldRm67162, GenericDisplay, Transcript, Transcript) {
    let mut old = old_rm67162();
    let mut new = new_rm67162();
    let t_old = run_spi(&mut old, steps);
    let t_new = run_spi(&mut new, steps);
    (old, new, t_old, t_new)
}

fn rm_window(cs: u16, ce: u16, rs: u16, re: u16) -> Vec<Step<'static>> {
    script([
        dc_command(
            0x2A,
            &[(cs >> 8) as u8, cs as u8, (ce >> 8) as u8, ce as u8],
        ),
        dc_command(
            0x2B,
            &[(rs >> 8) as u8, rs as u8, (re >> 8) as u8, re as u8],
        ),
    ])
}

fn rm_pixels(px: &[u16]) -> Vec<Step<'static>> {
    let mut bytes = Vec::with_capacity(px.len() * 2);
    for p in px {
        bytes.extend_from_slice(&p.to_be_bytes());
    }
    script([dc_command(0x2C, &[]), dc_data(&bytes)])
}

/// The init a Lilygo T-Display-S3 AMOLED driver sends, vendor commands and all.
/// 0xFE / 0xC4 / 0x35 / 0x44 are NOT in the command table and take no parameter
/// count: each is consumed, closes the stream, and its parameters then arrive
/// with nothing open. That is exactly what makes an unmodelled init command
/// harmless on a D/C-framed panel.
fn rm_init() -> Vec<Step<'static>> {
    script([
        dc_command(0xFE, &[0x00]),
        dc_command(0xC4, &[0x80]),
        dc_command(0x3A, &[0x55]),
        dc_command(0x35, &[0x00]),
        dc_command(0x44, &[0x01, 0x66]),
        dc_command(0x36, &[0x00]),
        dc_command(0x11, &[]),
        dc_command(0x29, &[]),
    ])
}

#[test]
fn rm67162_stock_init_and_a_frame_are_byte_identical() {
    // One colour deliberately dominant: an all-distinct frame ties every count
    // at 1, and a tie is the one place these two models disagree on purpose —
    // see `rm67162_top_colour_resolves_a_tie_deterministically`.
    let px: Vec<u16> = (0..1024u32)
        .map(|i| {
            if i % 2 == 0 {
                0x07E0
            } else {
                (i.wrapping_mul(2087) ^ 0x1234) as u16 | 0x8000
            }
        })
        .collect();
    let steps = script([
        vec![Step::CsSelect],
        rm_init(),
        dc_command(0x51, &[0xFF]),
        rm_window(0, 31, 0, 31),
        rm_pixels(&px),
        vec![Step::CsRelease],
    ]);
    let (old, new, t_old, t_new) = drive_both_rm67162(&steps);
    assert_eq!(t_old, t_new, "wire transcript");
    assert_eq!(
        old.framebuffer(),
        new.framebuffer(),
        "frame memory differs after the stock init and a 32x32 blit"
    );
    // Not a tautology: the blit really landed.
    assert!(
        new.framebuffer().iter().filter(|&&b| b != 0).count() > 1000,
        "the blit landed"
    );
    assert_same_artifact(
        &SpiDevice::artifacts(&old, "amoled", &opts())[0],
        &SpiDevice::artifacts(&new, "amoled", &opts())[0],
        "rm67162 stock init + frame",
    );
}

/// THE AMOLED ASSERTION, carried across the port.
///
/// A driver ported from a backlit TFT does everything right except write
/// brightness, because on a TFT brightness is a separate backlight pin that is
/// not the controller's business. On an AMOLED that firmware displays nothing.
/// `lit_requires: [{ var: brightness, min: 1 }]` is the descriptor key that
/// says so, and both models must agree that the pixels landed and the glass is
/// still dark.
#[test]
fn rm67162_dispon_without_brightness_is_not_lit() {
    let steps = script([
        vec![Step::CsSelect],
        dc_command(0x11, &[]),
        dc_command(0x29, &[]),
        rm_window(0, 9, 0, 0),
        rm_pixels(&[0xF800; 10]),
        vec![Step::CsRelease],
    ]);
    let (old, new, _, _) = drive_both_rm67162(&steps);
    let a_old = &SpiDevice::artifacts(&old, "amoled", &opts())[0];
    let a_new = &SpiDevice::artifacts(&new, "amoled", &opts())[0];
    assert_eq!(a_new.meta["display_on"], true, "DISPON was sent");
    assert_eq!(a_new.meta["brightness"], 0, "WRDISBV was never written");
    assert_eq!(
        a_new.meta["lit"], false,
        "an emissive panel at brightness 0 shows nothing, whatever DISPON says"
    );
    // Not a case of nothing happening: the pixels really did land.
    assert_ne!(
        new.framebuffer().iter().filter(|&&b| b != 0).count(),
        0,
        "frame memory must still hold what was written"
    );
    assert_same_artifact(a_old, a_new, "rm67162 DISPON without brightness");
}

/// The three states between "on" and "visible", each checked against the old
/// model: awake alone, bright alone, and both plus DISPON.
#[test]
fn rm67162_lit_needs_dispon_awake_and_brightness() {
    for (name, prelude) in [
        ("SLPOUT only", vec![dc_command(0x11, &[])]),
        (
            "SLPOUT + brightness, no DISPON",
            vec![dc_command(0x11, &[]), dc_command(0x51, &[0x7F])],
        ),
        (
            "DISPON + brightness, still asleep",
            vec![dc_command(0x29, &[]), dc_command(0x51, &[0x7F])],
        ),
        (
            "all three",
            vec![
                dc_command(0x11, &[]),
                dc_command(0x51, &[0x01]),
                dc_command(0x29, &[]),
            ],
        ),
    ] {
        let steps = script([vec![Step::CsSelect], script(prelude), vec![Step::CsRelease]]);
        let (old, new, _, _) = drive_both_rm67162(&steps);
        assert_eq!(
            old.is_lit(),
            new.lit(),
            "{name}: the two models disagree about `lit`"
        );
        assert_same_artifact(
            &SpiDevice::artifacts(&old, "amoled", &opts())[0],
            &SpiDevice::artifacts(&new, "amoled", &opts())[0],
            &format!("rm67162 {name}"),
        );
    }
    // Not vacuous: the last case IS lit and the first three are not.
    let all = script([
        vec![Step::CsSelect],
        dc_command(0x11, &[]),
        dc_command(0x51, &[0x01]),
        dc_command(0x29, &[]),
        vec![Step::CsRelease],
    ]);
    let (_, new, _, _) = drive_both_rm67162(&all);
    assert!(new.lit(), "brightness 1 with DISPON and SLPOUT is lit");
}

/// Seven MADCTL encodings, each with a window that only makes sense in that
/// orientation. MV changes what a legal column IS (0..239 vs 0..535), so it
/// moves the clamp as well as the pixel map.
#[test]
fn rm67162_madctl_orientation_matches() {
    for madctl in [0x00u8, 0x20, 0x40, 0x60, 0x80, 0xA0, 0xC0] {
        let steps = script([
            vec![Step::CsSelect],
            dc_command(0x36, &[madctl]),
            dc_command(0x51, &[0xFF]),
            rm_window(0, 15, 0, 15),
            // One colour dominant on purpose: an all-distinct frame ties every
            // count at 1, and a tie is the one thing these two models resolve
            // differently — see
            // `rm67162_top_colour_resolves_a_tie_deterministically`.
            rm_pixels(
                &(0..256u32)
                    .map(|i| {
                        if i % 2 == 0 {
                            0x07E0
                        } else {
                            0x8000 | i as u16
                        }
                    })
                    .collect::<Vec<_>>(),
            ),
            vec![Step::CsRelease],
        ]);
        let (old, new, _, _) = drive_both_rm67162(&steps);
        assert_eq!(
            old.framebuffer(),
            new.framebuffer(),
            "MADCTL 0x{madctl:02X}: frame memory"
        );
        assert_same_artifact(
            &SpiDevice::artifacts(&old, "amoled", &opts())[0],
            &SpiDevice::artifacts(&new, "amoled", &opts())[0],
            &format!("rm67162 MADCTL 0x{madctl:02X}"),
        );
    }
}

/// A landscape window at column 300 is legal only once MV is set — 300 is past
/// the portrait width of 240. Folding it back into portrait would move a whole
/// picture.
#[test]
fn rm67162_landscape_window_is_not_folded_into_portrait() {
    let steps = script([
        vec![Step::CsSelect],
        dc_command(0x36, &[0x20]),
        dc_command(0x51, &[0xFF]),
        rm_window(300, 331, 0, 15),
        rm_pixels(&[0x1F; 512]),
        vec![Step::CsRelease],
    ]);
    let (old, new, _, _) = drive_both_rm67162(&steps);
    assert_eq!(
        old.framebuffer(),
        new.framebuffer(),
        "landscape frame memory"
    );
    assert!(
        new.framebuffer().iter().filter(|&&b| b != 0).count() > 400,
        "the landscape blit landed; a window folded to portrait would clamp it \
         onto one column"
    );
    assert_same_artifact(
        &SpiDevice::artifacts(&old, "amoled", &opts())[0],
        &SpiDevice::artifacts(&new, "amoled", &opts())[0],
        "rm67162 landscape window",
    );
}

/// SWRESET on this controller CLEARS FRAME MEMORY as well as the control state
/// — the opposite of the MIPI reading the ST7789V (§9.1.22) and the ILI9341
/// (§8.2.2) state, where the frame memory is unaffected. That is why
/// `reset_control` and `clear_ram` are two actions in the descriptor and
/// neither implies the other.
#[test]
fn rm67162_swreset_clears_frame_memory_unlike_the_mipi_panels() {
    let steps = script([
        vec![Step::CsSelect],
        rm_init(),
        dc_command(0x51, &[0xFF]),
        rm_window(0, 15, 0, 15),
        rm_pixels(&[0xFFFF; 256]),
        dc_command(0x01, &[]),
        vec![Step::CsRelease],
    ]);
    let (old, new, _, _) = drive_both_rm67162(&steps);
    assert_eq!(
        new.framebuffer().iter().filter(|&&b| b != 0).count(),
        0,
        "SWRESET clears frame memory on the RM67162"
    );
    assert_eq!(old.framebuffer(), new.framebuffer());
    let a_new = &SpiDevice::artifacts(&new, "amoled", &opts())[0];
    assert_eq!(
        a_new.meta["brightness"], 0,
        "SWRESET resets WRDISBV to 0x00"
    );
    assert_eq!(a_new.meta["colmod"], "0x55", "and COLMOD to RGB565");
    assert_eq!(a_new.meta["madctl"], "0x00");
    assert_eq!(a_new.meta["display_on"], false);
    assert_eq!(a_new.meta["asleep"], true);
    assert_same_artifact(
        &SpiDevice::artifacts(&old, "amoled", &opts())[0],
        a_new,
        "rm67162 after SWRESET",
    );
    // The negative half: without the SWRESET the same script leaves a painted,
    // bright, awake panel. Deleting `clear_ram` from the descriptor has to
    // fail this.
    let without = script([
        vec![Step::CsSelect],
        rm_init(),
        dc_command(0x51, &[0xFF]),
        rm_window(0, 15, 0, 15),
        rm_pixels(&[0xFFFF; 256]),
        vec![Step::CsRelease],
    ]);
    let (_, painted, _, _) = drive_both_rm67162(&without);
    assert_eq!(
        painted.framebuffer().iter().filter(|&&b| b != 0).count(),
        512,
        "the same script without SWRESET leaves 256 white pixels"
    );
}

/// The two D/C wirings are both real hardware and the artifact says which one
/// this placement uses. A panel that framed nothing and a panel that was never
/// sent anything are indistinguishable without it.
#[test]
fn rm67162_publishes_which_dc_wiring_drives_it() {
    let (old, new, _, _) = drive_both_rm67162(&[]);
    assert_eq!(
        SpiDevice::artifacts(&new, "amoled", &opts())[0].meta["dc_source"],
        "controller_dcx"
    );
    assert_same_artifact(
        &SpiDevice::artifacts(&old, "amoled", &opts())[0],
        &SpiDevice::artifacts(&new, "amoled", &opts())[0],
        "rm67162 hw dcx",
    );

    let mut old_gpio = OldRm67162::with_gpio_dc(CS, DC);
    let mut new_gpio = labwired_core::peripherals::components::rm67162_gpio_dc(CS, DC);
    let steps = script([
        vec![Step::CsSelect],
        rm_init(),
        dc_command(0x51, &[0x40]),
        rm_window(0, 3, 0, 0),
        rm_pixels(&[0xF81F; 4]),
        vec![Step::CsRelease],
    ]);
    assert_eq!(
        run_spi(&mut old_gpio, &steps),
        run_spi(&mut new_gpio, &steps)
    );
    assert_eq!(old_gpio.framebuffer(), new_gpio.framebuffer());
    let a = &SpiDevice::artifacts(&new_gpio, "amoled", &opts())[0];
    assert_eq!(a.meta["dc_source"], "gpio");
    assert_eq!(a.meta["lit"], true);
    assert_eq!(a.meta["brightness"], 0x40);
    assert_same_artifact(
        &SpiDevice::artifacts(&old_gpio, "amoled", &opts())[0],
        a,
        "rm67162 gpio dc",
    );
}

/// ── A DELIBERATE DIFFERENCE, asserted in both directions ────────────────────
///
/// The old model counted colours in a `HashMap` and resolved a tie by hash
/// iteration order — not stable between runs, between native and wasm, or
/// between machines, in a field the browser prints as the dominant colour. The
/// descriptor counts in a `BTreeMap`, so a tie resolves to the highest RGB565
/// value identically everywhere. Untied frames — every real picture, which has
/// a background — are unchanged, which is why every other test above compares
/// the two artifacts field for field and passes. Same change the ST7789 and
/// ILI9341 ports made.
#[test]
fn rm67162_top_colour_resolves_a_tie_deterministically() {
    let steps = script([
        vec![Step::CsSelect],
        dc_command(0x29, &[]),
        rm_window(0, 3, 0, 0),
        rm_pixels(&[0x07E0, 0x07E0, 0xF800, 0xF800]),
        vec![Step::CsRelease],
    ]);
    let (_, new, _, _) = drive_both_rm67162(&steps);
    let art = &SpiDevice::artifacts(&new, "amoled", &opts())[0];
    assert_eq!(
        art.meta["top_colour"], "0xF800",
        "a tie resolves to the highest RGB565 value, on every machine"
    );
    assert_eq!(art.meta["top_colour_pixels"], 2);
}

/// An unpowered module refuses the bus, so every flag stays at its
/// power-on-dark value by construction rather than by masking at report time.
#[test]
fn rm67162_unpowered_module_matches() {
    let steps = script([
        vec![Step::CsSelect],
        rm_init(),
        dc_command(0x51, &[0xFF]),
        rm_window(0, 3, 0, 0),
        rm_pixels(&[0xFFFF; 4]),
        vec![Step::CsRelease],
    ]);
    let mut old = old_rm67162().with_powered(false);
    let mut new = new_rm67162();
    new.set_powered(false);
    assert_eq!(run_spi(&mut old, &steps), run_spi(&mut new, &steps));
    assert_eq!(old.framebuffer(), new.framebuffer());
    let art = &SpiDevice::artifacts(&new, "amoled", &opts())[0];
    assert_eq!(art.meta["powered"], false);
    assert_eq!(art.meta["lit"], false);
    assert_eq!(
        art.meta["asleep"], true,
        "an unpowered panel was never woken"
    );
    assert_eq!(art.meta["brightness"], 0);
    assert_eq!(art.meta["painted_bytes"], 0);
    assert_same_artifact(
        &SpiDevice::artifacts(&old, "amoled", &opts())[0],
        art,
        "rm67162 unpowered",
    );
}

/// The artifact's published shape, key for key. These names are the contract
/// the browser overlay and the CLI read; a port that renamed `asleep` to
/// `awake` or turned `colmod` from `"0x55"` into `85` would break a consumer
/// silently.
#[test]
fn rm67162_artifact_keeps_its_published_shape() {
    let (old, new, _, _) = drive_both_rm67162(&[]);
    let art = &SpiDevice::artifacts(&new, "amoled", &opts())[0];
    assert_eq!(art.meta["format"], "rgb565_be");
    assert_eq!(art.meta["w"], 240);
    assert_eq!(art.meta["h"], 536);
    assert_eq!(art.meta["total_bytes"], 240 * 536 * 2);
    assert_eq!(
        art.meta["colmod"], "0x55",
        "hex-formatted, not the number 85"
    );
    assert_eq!(art.meta["madctl"], "0x00");
    assert_eq!(art.meta["brightness"], 0, "raw, not hex");
    assert!(
        art.meta.get("awake").is_none(),
        "this panel publishes `asleep`, not `awake`"
    );
    assert!(art.meta.get("inverted").is_none());
    let mut keys: Vec<&String> = art.meta.as_object().expect("object").keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "asleep",
            "brightness",
            "colmod",
            "dc_source",
            "display_on",
            "format",
            "generation",
            "h",
            "lit",
            "madctl",
            "painted_bytes",
            "powered",
            "top_colour",
            "top_colour_pixels",
            "total_bytes",
            "w",
        ]
    );
    assert_same_artifact(
        &SpiDevice::artifacts(&old, "amoled", &opts())[0],
        art,
        "rm67162 power-on artifact",
    );
}

// ─── tri-colour e-paper: SSD1680 and UC8151D ───────────────────────────────
//
// The first panels here where FRAME MEMORY IS NOT THE SCREEN, and the first
// with two RAMs. Everything below drives the descriptor and the deleted model
// through one script and compares the transcript, BOTH PLANES byte for byte,
// the latched screen, and every field of the artifact's `meta`.

use display_oracle::ssd1680_tricolor_290::Ssd1680Tricolor290 as OldSsd1680;
use display_oracle::uc8151d_tricolor_290::Uc8151dTricolor290 as OldUc8151d;

/// 128 px / 8 = 16 bytes per row.
const EPD_ROW_BYTES: usize = 16;
/// One plane of the 2.9" tri-colour glass.
const EPD_PLANE_BYTES: usize = EPD_ROW_BYTES * 296;
/// A D/C output register the bus would have resolved. Its VALUE is irrelevant —
/// what matters is that both models are told a D/C line EXISTS, so both take
/// their wired path and `Step::Dc` frames the script. Without it each takes its
/// own declared unwired cheat, which is a different test (see
/// `ssd1680_with_no_dc_line_infers_framing_in_both_models`).
const EPD_DC_ODR: u64 = 0x4000_0000;

fn new_ssd1680() -> GenericDisplay {
    let mut dev = labwired_core::peripherals::components::ssd1680_tricolor_290(CS);
    dev.set_dc_pin(DC);
    SpiDevice::set_dc_source(&mut dev, EPD_DC_ODR, 0);
    dev
}

fn new_uc8151d() -> GenericDisplay {
    let mut dev = labwired_core::peripherals::components::uc8151d_tricolor_290(CS);
    dev.set_dc_pin(DC);
    SpiDevice::set_dc_source(&mut dev, EPD_DC_ODR, 0);
    dev
}

fn old_ssd1680() -> OldSsd1680 {
    let mut dev = OldSsd1680::new(CS).with_dc_pin(DC);
    SpiDevice::set_dc_source(&mut dev, EPD_DC_ODR, 0);
    dev
}

fn old_uc8151d() -> OldUc8151d {
    let mut dev = OldUc8151d::new(CS).with_dc_pin(DC);
    SpiDevice::set_dc_source(&mut dev, EPD_DC_ODR, 0);
    dev
}

/// The keys the YAML e-paper publishes that the deleted models could not:
/// the ink on THE GLASS, as opposed to the ink in frame memory. Named here so
/// the parity comparison can require exactly these two and no others — a third
/// new key is a contract change and fails the test.
const EPD_NEW_META_KEYS: [&str; 2] = ["screen_black_ink_bytes", "screen_red_ink_bytes"];

/// Compare an e-paper's two artifacts the way [`assert_same_artifact`] compares
/// every other panel's, with ONE named exception: the descriptor publishes the
/// two `screen_*` counts and the deleted model had no screen to count. Every
/// other key must be identical, the payload must be identical, and the new
/// key set must be exactly the old one plus those two.
fn assert_same_epaper_artifact(old: &Artifact, new: &Artifact, what: &str) {
    let (o, n) = (
        old.meta.as_object().expect("old meta is an object"),
        new.meta.as_object().expect("new meta is an object"),
    );
    for (k, v) in o {
        assert_eq!(
            Some(v),
            n.get(k),
            "{what}: artifact meta['{k}'] differs — the Rust model said {v:?}, the descriptor \
             {:?}",
            n.get(k)
        );
    }
    let mut extra: Vec<&str> = n
        .keys()
        .filter(|k| !o.contains_key(*k))
        .map(|k| k.as_str())
        .collect();
    extra.sort_unstable();
    assert_eq!(
        extra, EPD_NEW_META_KEYS,
        "{what}: the descriptor may add exactly the two `of: screen` counts and nothing else"
    );
    assert_eq!(old.kind, new.kind, "{what}: artifact kind");
    assert_eq!(old.id, new.id, "{what}: artifact id");
    assert_eq!(old.bytes, new.bytes, "{what}: artifact payload");
}

/// Both planes, straight off the artifact payload, so the comparison reads the
/// PUBLISHED bytes rather than a private accessor.
fn planes_of(a: &Artifact) -> (&[u8], &[u8]) {
    let bytes = a.bytes.as_ref().expect("artifact carries its bytes");
    assert_eq!(
        bytes.len(),
        2 * EPD_PLANE_BYTES,
        "black plane then red plane"
    );
    bytes.split_at(EPD_PLANE_BYTES)
}

fn epd_art(dev: &dyn SpiDevice) -> Artifact {
    SpiDevice::artifacts(dev, "epd", &opts())
        .into_iter()
        .next()
        .expect("one framebuffer artifact")
}

/// The exact byte sequence `GxEPD2_290_C90c::_InitDisplay()` emits.
fn ssd1680_init() -> Vec<Step<'static>> {
    script([
        dc_command(0x12, &[]),
        dc_command(0x01, &[0x27, 0x01, 0x00]),
        dc_command(0x11, &[0x03]),
        dc_command(0x3C, &[0x05]),
        dc_command(0x18, &[0x80]),
        dc_command(0x21, &[0x00, 0x80]),
        // _setPartialRamArea(0, 0, 128, 296)
        dc_command(0x44, &[0x00, 0x0F]),
        dc_command(0x45, &[0x00, 0x00, 0x27, 0x01]),
        dc_command(0x4E, &[0x00]),
        dc_command(0x4F, &[0x00, 0x00]),
    ])
}

/// `clearScreen(0xFF, 0xFF)`: a white black-plane and — because GxEPD2 writes
/// `~color_value` for red — a fully RED red-plane, then the `_Update_Part`
/// sequence that activates.
fn ssd1680_clear_screen() -> Vec<Step<'static>> {
    script([
        dc_command(0x24, &[]),
        dc_data(&vec![0xFF; EPD_PLANE_BYTES]),
        dc_command(0x26, &[]),
        dc_data(&vec![0x00; EPD_PLANE_BYTES]),
        dc_command(0x22, &[0xF7]),
        dc_command(0x20, &[]),
    ])
}

fn drive_both_ssd1680(steps: &[Step<'_>]) -> (OldSsd1680, GenericDisplay, Transcript, Transcript) {
    let mut old = old_ssd1680();
    let mut new = new_ssd1680();
    let t_old = run_spi(&mut old, steps);
    let t_new = run_spi(&mut new, steps);
    (old, new, t_old, t_new)
}

fn drive_both_uc8151d(steps: &[Step<'_>]) -> (OldUc8151d, GenericDisplay, Transcript, Transcript) {
    let mut old = old_uc8151d();
    let mut new = new_uc8151d();
    let t_old = run_spi(&mut old, steps);
    let t_new = run_spi(&mut new, steps);
    (old, new, t_old, t_new)
}

#[test]
fn ssd1680_init_and_clear_screen_are_byte_identical() {
    let steps = script([ssd1680_init(), ssd1680_clear_screen()]);
    let (old, new, t_old, t_new) = drive_both_ssd1680(&steps);
    assert_eq!(t_old, t_new, "wire transcript");

    let (a_old, a_new) = (epd_art(&old), epd_art(&new));
    assert_same_epaper_artifact(&a_old, &a_new, "ssd1680 init + clearScreen");

    // Not merely "equal to each other": the picture is the one GxEPD2 asked for.
    let (black, red) = planes_of(&a_new);
    assert!(black.iter().all(|&b| b == 0xFF), "black plane all white");
    assert!(
        red.iter().all(|&b| b == 0x00),
        "red plane all red on the wire"
    );
    assert_eq!(a_new.meta["black_ink_bytes"], 0, "0xFF is NO ink");
    assert_eq!(a_new.meta["red_ink_bytes"], EPD_PLANE_BYTES);
    assert_eq!(a_new.meta["refresh_generation"], 1, "0x20 activated once");
    assert_eq!(a_new.meta["plane_bytes"], EPD_PLANE_BYTES);
}

#[test]
fn ssd1680_a_partial_window_writes_the_same_thirty_two_bytes() {
    // 16x16 pixels in the top-left corner: 2 byte-columns x 16 rows.
    let steps = script([
        dc_command(0x44, &[0x00, 0x01]),
        dc_command(0x45, &[0x00, 0x00, 0x0F, 0x00]),
        dc_command(0x4E, &[0x00]),
        dc_command(0x4F, &[0x00, 0x00]),
        dc_command(0x24, &[]),
        dc_data(&[0x55; 32]),
        // The 33rd byte after 0x24 must be a COMMAND again: the window counted
        // the stream out. SWRESET is the one whose effect is visible.
        dc_command(0x12, &[]),
    ]);
    let (old, new, t_old, t_new) = drive_both_ssd1680(&steps);
    assert_eq!(t_old, t_new, "wire transcript");
    let (a_old, a_new) = (epd_art(&old), epd_art(&new));
    assert_same_epaper_artifact(&a_old, &a_new, "ssd1680 partial window");

    let (black, _) = planes_of(&a_new);
    for row in 0..16 {
        assert_eq!(black[row * EPD_ROW_BYTES], 0x55, "row {row} col 0");
        assert_eq!(black[row * EPD_ROW_BYTES + 1], 0x55, "row {row} col 1");
        assert_eq!(
            black[row * EPD_ROW_BYTES + 2],
            0xFF,
            "row {row} outside the window"
        );
    }
    assert_eq!(a_new.meta["black_ink_bytes"], 32, "exactly the window");
}

/// THE BYTE-UNIT X AXIS. 0x44 takes RAM-X as start/8; if the descriptor read it
/// in pixels the window would be eight times too narrow and the rows would land
/// on top of one another. The oracle is the reference for where they land.
#[test]
fn ssd1680_an_offset_byte_window_lands_on_the_same_rows() {
    let steps = script([
        // X bytes 4..=5, Y rows 100..=103.
        dc_command(0x44, &[0x04, 0x05]),
        dc_command(0x45, &[0x64, 0x00, 0x67, 0x00]),
        dc_command(0x4E, &[0x04]),
        dc_command(0x4F, &[0x64, 0x00]),
        dc_command(0x24, &[]),
        dc_data(&[0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7]),
    ]);
    let (old, new, _, _) = drive_both_ssd1680(&steps);
    let (a_old, a_new) = (epd_art(&old), epd_art(&new));
    assert_same_epaper_artifact(&a_old, &a_new, "ssd1680 offset window");
    let (black, _) = planes_of(&a_new);
    assert_eq!(black[100 * EPD_ROW_BYTES + 4], 0xA0);
    assert_eq!(black[100 * EPD_ROW_BYTES + 5], 0xA1);
    assert_eq!(black[103 * EPD_ROW_BYTES + 4], 0xA6);
    assert_eq!(black[103 * EPD_ROW_BYTES + 5], 0xA7);
    assert_eq!(a_new.meta["black_ink_bytes"], 8);
}

/// 0x22 IS A SEQUENCE SELECTOR, which is what the per-action `when` guard is
/// for: 0xF8 powers the booster on, 0x83 powers it off, 0xF7 does neither.
#[test]
fn ssd1680_power_on_and_off_track_the_0x22_parameter() {
    for (param, expect) in [(0xF8u8, true), (0x83, false)] {
        let steps = script([dc_command(0x22, &[param]), dc_command(0x20, &[])]);
        let (old, new, _, _) = drive_both_ssd1680(&steps);
        let (a_old, a_new) = (epd_art(&old), epd_art(&new));
        assert_same_epaper_artifact(&a_old, &a_new, &format!("ssd1680 0x22 {param:#04X}"));
        assert_eq!(a_new.meta["power_on"], expect, "0x22 {param:#04X}");
    }
    // 0xF7 — the full-update selector GxEPD2 sends — changes no power state.
    let steps = script([
        dc_command(0x22, &[0xF8]),
        dc_command(0x20, &[]),
        dc_command(0x22, &[0xF7]),
        dc_command(0x20, &[]),
    ]);
    let (old, new, _, _) = drive_both_ssd1680(&steps);
    let (a_old, a_new) = (epd_art(&old), epd_art(&new));
    assert_same_epaper_artifact(&a_old, &a_new, "ssd1680 0x22 0xF7");
    assert_eq!(a_new.meta["power_on"], true, "0xF7 must not power off");
    assert_eq!(a_new.meta["refresh_generation"], 2, "two activations");
}

#[test]
fn ssd1680_deep_sleep_drops_the_booster_only_when_the_enter_bit_is_set() {
    for (param, expect) in [(0x01u8, false), (0x00, true)] {
        let steps = script([
            dc_command(0x22, &[0xF8]),
            dc_command(0x20, &[]),
            dc_command(0x10, &[param]),
        ]);
        let (old, new, _, _) = drive_both_ssd1680(&steps);
        let (a_old, a_new) = (epd_art(&old), epd_art(&new));
        assert_same_epaper_artifact(&a_old, &a_new, &format!("ssd1680 0x10 {param:#04X}"));
        assert_eq!(a_new.meta["power_on"], expect, "0x10 param {param:#04X}");
    }
}

/// THE SUPPLY GATE, both directions. The positive control matters: "unpowered
/// stays blank" also passes on a model that never inks anything.
#[test]
fn ssd1680_an_unpowered_panel_is_blank_in_both_models() {
    let steps = script([
        ssd1680_init(),
        dc_command(0x24, &[]),
        dc_data(&vec![0x00; EPD_PLANE_BYTES]),
        dc_command(0x22, &[0xF7]),
        dc_command(0x20, &[]),
    ]);

    let (old, new, _, _) = drive_both_ssd1680(&steps);
    let (a_old, a_new) = (epd_art(&old), epd_art(&new));
    assert_same_epaper_artifact(&a_old, &a_new, "ssd1680 powered control");
    assert_eq!(
        a_new.meta["black_ink_bytes"], EPD_PLANE_BYTES,
        "positive control"
    );
    assert_eq!(a_new.meta["refresh_generation"], 1);

    let mut old = old_ssd1680().with_powered(false);
    let mut new = new_ssd1680();
    new.set_powered(false);
    run_spi(&mut old, &steps);
    run_spi(&mut new, &steps);
    let (a_old, a_new) = (epd_art(&old), epd_art(&new));
    assert_same_epaper_artifact(&a_old, &a_new, "ssd1680 unpowered");
    assert_eq!(a_new.meta["black_ink_bytes"], 0, "no supply, no ink");
    assert_eq!(a_new.meta["refresh_generation"], 0);
    assert_eq!(a_new.meta["powered"], false, "the artifact must say WHY");
}

/// NO D/C LINE IS ALSO A BOARD. The ESP32 e-paper lab wires CS and nothing
/// else, and the deleted model inferred framing there. The descriptor's
/// `dc.unwired: infer` is that same cheat, stated.
#[test]
fn ssd1680_with_no_dc_line_infers_framing_in_both_models() {
    // The same bytes, with NO `Step::Dc` anywhere and no `dc_source` on either
    // device: pure byte stream, exactly what the lab's SPI peripheral clocks.
    let mut bytes: Vec<u8> = vec![
        0x12, 0x01, 0x27, 0x01, 0x00, 0x11, 0x03, 0x44, 0x00, 0x0F, 0x45, 0x00, 0x00, 0x27, 0x01,
        0x4E, 0x00, 0x4F, 0x00, 0x00, 0x24,
    ];
    bytes.extend(std::iter::repeat_n(0x00u8, EPD_PLANE_BYTES));
    bytes.extend([0x22, 0xF7, 0x20]);
    let steps = vec![Step::CsSelect, Step::Transfer(&bytes), Step::CsRelease];

    let mut old = OldSsd1680::new(CS);
    let mut new = labwired_core::peripherals::components::ssd1680_tricolor_290(CS);
    assert_eq!(run_spi(&mut old, &steps), run_spi(&mut new, &steps));
    let (a_old, a_new) = (epd_art(&old), epd_art(&new));
    assert_same_epaper_artifact(&a_old, &a_new, "ssd1680 unwired D/C");
    assert_eq!(
        a_new.meta["black_ink_bytes"], EPD_PLANE_BYTES,
        "the inference must terminate at the window end and let 0x22 decode",
    );
    assert_eq!(
        a_new.meta["refresh_generation"], 1,
        "0x20 decoded as a command"
    );
}

// ─── UC8151D ───────────────────────────────────────────────────────────────

/// `GxEPD2_290_Z13c`-shaped drive: power on, both planes, refresh.
fn uc8151d_frame(black: u8, red: u8) -> Vec<Step<'static>> {
    script([
        dc_command(0x00, &[0x0F]),
        dc_command(0x61, &[0x80, 0x01, 0x28]),
        dc_command(0x50, &[0x77]),
        dc_command(0x04, &[]),
        dc_command(0x10, &[]),
        dc_data(&vec![black; EPD_PLANE_BYTES]),
        dc_command(0x13, &[]),
        dc_data(&vec![red; EPD_PLANE_BYTES]),
        dc_command(0x12, &[]),
    ])
}

#[test]
fn uc8151d_a_full_frame_is_byte_identical() {
    let steps = uc8151d_frame(0x00, 0xFF);
    let (old, new, t_old, t_new) = drive_both_uc8151d(&steps);
    assert_eq!(t_old, t_new, "wire transcript");
    let (a_old, a_new) = (epd_art(&old), epd_art(&new));
    assert_same_epaper_artifact(&a_old, &a_new, "uc8151d full frame");

    let (black, red) = planes_of(&a_new);
    assert!(black.iter().all(|&b| b == 0x00), "black plane all ink");
    assert!(red.iter().all(|&b| b == 0xFF), "red plane blank");
    assert_eq!(a_new.meta["black_ink_bytes"], EPD_PLANE_BYTES);
    assert_eq!(a_new.meta["red_ink_bytes"], 0);
    assert_eq!(a_new.meta["power_on"], true, "PON");
    assert_eq!(a_new.meta["refresh_generation"], 1, "DRF");
}

#[test]
fn uc8151d_pon_and_pof_move_the_booster() {
    let steps = script([dc_command(0x04, &[]), dc_command(0x02, &[])]);
    let (old, new, _, _) = drive_both_uc8151d(&steps);
    let (a_old, a_new) = (epd_art(&old), epd_art(&new));
    assert_same_epaper_artifact(&a_old, &a_new, "uc8151d PON then POF");
    assert_eq!(a_new.meta["power_on"], false);
}

/// The LUT commands carry 42 and 44 parameters. They must be COUNTED, or the
/// byte after a LUT decodes as an opcode and the next plane stream never opens.
#[test]
fn uc8151d_a_forty_four_byte_lut_does_not_desynchronise_either_model() {
    let steps = script([
        dc_command(0x20, &[0x11; 44]),
        dc_command(0x21, &[0x22; 42]),
        dc_command(0x04, &[]),
        dc_command(0x10, &[]),
        dc_data(&[0x0F; 8]),
        dc_command(0x12, &[]),
    ]);
    let (old, new, _, _) = drive_both_uc8151d(&steps);
    let (a_old, a_new) = (epd_art(&old), epd_art(&new));
    assert_same_epaper_artifact(&a_old, &a_new, "uc8151d LUTs");
    let (black, _) = planes_of(&a_new);
    assert_eq!(&black[..8], &[0x0F; 8], "the stream after the LUTs painted");
    assert_eq!(a_new.meta["power_on"], true);
    assert_eq!(a_new.meta["refresh_generation"], 1);
}

/// ⚠️ THE ONE DELIBERATE DIFFERENCE, pinned from BOTH sides so neither can be
/// "fixed" into the other by accident.
///
/// A DTM1 stream longer than one plane: the deleted model CLIPPED (its cursor
/// stopped at 4736 and further bytes were dropped), the descriptor's address
/// counters WRAP to the window origin — which is what every other panel here
/// does and what the counters of this family are described as doing. GxEPD2
/// sends exactly one plane, so no firmware in this tree reaches it.
#[test]
fn uc8151d_an_over_long_plane_stream_clips_in_the_old_model_and_wraps_in_the_new() {
    let mut data = vec![0x0Fu8; EPD_PLANE_BYTES];
    data.extend([0xA5, 0xA5, 0xA5, 0xA5]);
    let steps = script([dc_command(0x10, &[]), dc_data(&data)]);

    let mut old = old_uc8151d();
    let mut new = new_uc8151d();
    run_spi(&mut old, &steps);
    run_spi(&mut new, &steps);

    let a_old = epd_art(&old);
    let (black_old, _) = planes_of(&a_old);
    assert_eq!(
        &black_old[..4],
        &[0x0F; 4],
        "the deleted model DROPPED the four trailing bytes",
    );
    let a_new = epd_art(&new);
    let (black_new, _) = planes_of(&a_new);
    assert_eq!(
        &black_new[..4],
        &[0xA5; 4],
        "the descriptor's counters WRAPPED and rewrote the first four bytes",
    );
    assert_eq!(
        &black_new[4..8],
        &[0x0F; 4],
        "and only the four bytes that were re-sent moved",
    );
}

/// WITH NO D/C LINE THE UC8151D CANNOT INFER, and both models say so the same
/// way: every byte is data, nothing decodes, nothing paints. An honest blank
/// rather than a plausible wrong picture.
#[test]
fn uc8151d_with_no_dc_line_paints_nothing_in_both_models() {
    let mut bytes = vec![0x04u8, 0x10];
    bytes.extend(std::iter::repeat_n(0x00u8, EPD_PLANE_BYTES));
    bytes.push(0x12);
    let steps = vec![Step::CsSelect, Step::Transfer(&bytes), Step::CsRelease];

    let mut old = OldUc8151d::new(CS);
    let mut new = labwired_core::peripherals::components::uc8151d_tricolor_290(CS);
    assert_eq!(run_spi(&mut old, &steps), run_spi(&mut new, &steps));
    let (a_old, a_new) = (epd_art(&old), epd_art(&new));
    assert_same_epaper_artifact(&a_old, &a_new, "uc8151d unwired D/C");
    assert_eq!(
        a_new.meta["black_ink_bytes"], 0,
        "nothing decoded, nothing painted"
    );
    assert_eq!(a_new.meta["refresh_generation"], 0);
    assert_eq!(a_new.meta["power_on"], false);
}

// ─── what the deleted models could not say: the glass ──────────────────────

/// FRAME MEMORY IS NOT THE SCREEN. A frame written and never activated changes
/// `black_ink_bytes` and leaves `screen_black_ink_bytes` where it was. Nothing
/// in either deleted model could tell the two apart, which is why these are the
/// only two keys the port adds.
#[test]
fn a_frame_written_after_the_refresh_is_in_ram_and_not_on_the_glass() {
    let mut new = new_ssd1680();
    run_spi(
        &mut new,
        &script([
            ssd1680_init(),
            dc_command(0x24, &[]),
            dc_data(&vec![0x00; EPD_PLANE_BYTES]),
            dc_command(0x20, &[]),
        ]),
    );
    let a = epd_art(&new);
    assert_eq!(a.meta["black_ink_bytes"], EPD_PLANE_BYTES);
    assert_eq!(
        a.meta["screen_black_ink_bytes"], EPD_PLANE_BYTES,
        "activated"
    );
    assert_eq!(a.meta["refresh_generation"], 1);

    // A second frame, NEVER activated: RAM goes blank, the glass does not.
    run_spi(
        &mut new,
        &script([
            dc_command(0x44, &[0x00, 0x0F]),
            dc_command(0x45, &[0x00, 0x00, 0x27, 0x01]),
            dc_command(0x24, &[]),
            dc_data(&vec![0xFF; EPD_PLANE_BYTES]),
        ]),
    );
    let a = epd_art(&new);
    assert_eq!(a.meta["black_ink_bytes"], 0, "frame memory was erased");
    assert_eq!(
        a.meta["screen_black_ink_bytes"], EPD_PLANE_BYTES,
        "the glass still holds the activated frame — that is what e-paper does",
    );
    assert_eq!(a.meta["refresh_generation"], 1, "no second activation");
}

/// The runtime snapshot round-trips the PICTURE — both planes, the glass and
/// the refresh counter — and an untagged capture is REFUSED rather than
/// restored into a panel whose shape has changed.
#[test]
fn an_epaper_runtime_snapshot_round_trips_and_an_untagged_one_is_refused() {
    let mut src = new_ssd1680();
    run_spi(
        &mut src,
        &script([
            ssd1680_init(),
            dc_command(0x24, &[]),
            dc_data(&vec![0x0F; EPD_PLANE_BYTES]),
            dc_command(0x20, &[]),
            // Written after the activation, so RAM and the glass DISAGREE and a
            // snapshot that carried only RAM would restore the wrong picture.
            dc_command(0x44, &[0x00, 0x0F]),
            dc_command(0x45, &[0x00, 0x00, 0x27, 0x01]),
            dc_command(0x24, &[]),
            dc_data(&vec![0xFF; EPD_PLANE_BYTES]),
        ]),
    );
    let blob = SpiDevice::runtime_snapshot(&src);

    let mut dst = new_ssd1680();
    SpiDevice::restore_runtime_snapshot(&mut dst, &blob).expect("a tagged snapshot restores");
    assert_eq!(
        epd_art(&dst).meta,
        epd_art(&src).meta,
        "every published fact survives the round trip",
    );
    assert_eq!(epd_art(&dst).bytes, epd_art(&src).bytes, "and every byte");
    assert_eq!(
        epd_art(&dst).meta["screen_black_ink_bytes"],
        EPD_PLANE_BYTES
    );
    assert_eq!(epd_art(&dst).meta["black_ink_bytes"], 0);

    // The pre-versioning format was RAW FRAME MEMORY with no header. It must be
    // refused, and the message must say what to do.
    let legacy = vec![0xFFu8; 2 * EPD_PLANE_BYTES];
    let err = SpiDevice::restore_runtime_snapshot(&mut dst, &legacy)
        .expect_err("an untagged snapshot must be refused");
    let msg = err.to_string();
    assert!(
        msg.contains("retake"),
        "the error must say what to do: {msg}"
    );
}
