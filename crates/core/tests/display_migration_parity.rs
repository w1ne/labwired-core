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
