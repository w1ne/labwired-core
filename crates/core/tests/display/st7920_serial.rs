// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ST7920 128x64 graphic LCD controller in serial (PSB = 0) mode, driven over
//! its own wire framing and read back through the panel's inspect artifact.
//!
//! Every expectation here comes from the Sitronix ST7920 datasheet (cited by
//! section; the section names are those of the v4.0 English and C2.0c
//! Chinese editions, which share their tables):
//!
//! * "Serial Interface": a start byte of five 1s, RW, RS, 0 (`0xF8 | RW << 2 |
//!   RS << 1`), then every 8-bit instruction or datum split into two bytes,
//!   upper nibble first, each nibble in D7..D4 with D3..D0 zero. CS is active
//!   HIGH; CS low resets the serial counter. There is no read in serial mode.
//! * "Instruction set" tables 1 (RE = 0) and 2 (RE = 1), and "Register flag
//!   initial values" for power-on state.
//! * "DDRAM": 16-bit words, high byte first, AC increments after the second
//!   byte; 02H..7FH select the 16x8 half-width HCGROM font, 0000H/0002H/0004H/
//!   0006H select a CGRAM glyph (bits 4..15 zero, bits 1..2 the glyph), A1H and
//!   above start a two-byte 16x16 CGROM code.
//! * "CGRAM" (Table 5): address bits 4..5 select the glyph, bits 0..3 the row,
//!   data bit 15 is the leftmost pixel.
//! * "GDRAM" (Table 8) and "Set graphic RAM address": vertical address first,
//!   then horizontal; only the horizontal address auto-increments, wrapping
//!   0FH -> 00H with no vertical carry. Bit 15 of a word is the leftmost pixel.
//!   On a 128x64 module the 256x32 address space is folded: horizontal words
//!   0..7 are rows 0..31, words 8..15 are rows 32..63. DDRAM folds the same way
//!   (80H line on rows 0..15, 90H on 16..31, 88H on 32..47, 98H on 48..63).
//!
//! Nothing here downcasts to the model: bytes go in through `SpiDevice`, pixels
//! come out of the `framebuffer` artifact the display oracle reads.

use labwired_config::{ChipDescriptor, ExternalDevice, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::inspect::{artifact_region_ink, Artifact, InspectOpts, PixelRegion};
use labwired_core::peripherals::spi::SpiDevice;
use labwired_core::Bus;
use std::collections::HashMap;
use std::path::PathBuf;

const W: usize = 128;
const H: usize = 64;

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn manifest(chip: &str, connection: &str, config: &[(&str, serde_yaml::Value)]) -> SystemBus {
    let chip_path = repo(chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    let mut cfg = HashMap::new();
    for (k, v) in config {
        cfg.insert(k.to_string(), v.clone());
    }
    let manifest = SystemManifest {
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "st7920-rig".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        cpu_hz: None,
        external_devices: vec![ExternalDevice {
            id: "lcd".to_string(),
            r#type: "st7920".to_string(),
            connection: connection.to_string(),
            channel: None,
            route: Default::default(),
            config: cfg,
        }],
        board_io: vec![],
        debug_uart: None,
        wifi_ap: None,
        peripherals: vec![],
        parts: Default::default(),
        memory_overrides: Default::default(),
    };
    SystemBus::from_config(&chip, &manifest).expect("build bus")
}

/// The panel on an STM32F103 SPI1, CS on PA4.
fn rig() -> SystemBus {
    manifest(
        "configs/chips/stm32f103.yaml",
        "spi1",
        &[("cs_pin", "PA4".into())],
    )
}

fn device(bus: &mut SystemBus) -> &mut Box<dyn SpiDevice> {
    let idx = bus
        .find_peripheral_index_by_name("spi1")
        .expect("spi1 registered");
    bus.peripherals[idx]
        .dev
        .spi_attached_devices_mut()
        .expect("spi1 hosts SPI devices")
        .first_mut()
        .expect("the ST7920 is attached")
}

/// One CS-high window carrying `wire` bytes, as a host SPI clocks them.
fn send(bus: &mut SystemBus, wire: &[u8]) {
    let dev = device(bus);
    dev.cs_select();
    for &b in wire {
        dev.transfer(b);
    }
    dev.cs_release();
}

/// Serial-mode instruction write: sync with RW = 0, RS = 0, then the split
/// nibbles.
fn cmd(wire: &mut Vec<u8>, b: u8) {
    wire.extend([0xF8, b & 0xF0, b << 4]);
}

/// Serial-mode data write: one sync with RS = 1, then each byte as two nibble
/// bytes.
fn data(wire: &mut Vec<u8>, bytes: &[u8]) {
    wire.push(0xFA);
    for &b in bytes {
        wire.extend([b & 0xF0, b << 4]);
    }
}

fn cmds(seq: &[u8]) -> Vec<u8> {
    let mut w = Vec::new();
    for &b in seq {
        cmd(&mut w, b);
    }
    w
}

fn artifact(bus: &SystemBus) -> Artifact {
    let opts = InspectOpts {
        include_bytes: true,
        peripheral: None,
    };
    bus.display_artifact("lcd", &opts)
        .expect("the ST7920 publishes a display artifact")
}

/// Pixel (x, y) of the artifact, decoded by the format it declares.
fn px(art: &Artifact, x: usize, y: usize) -> bool {
    let (ink, _) = ink(art, x, y, 1, 1);
    ink == 1
}

fn ink(art: &Artifact, x: usize, y: usize, w: usize, h: usize) -> (usize, usize) {
    let format = art.meta["format"].as_str().expect("format");
    artifact_region_ink(
        format,
        &art.meta,
        art.bytes.as_deref().expect("bytes requested"),
        PixelRegion { x, y, w, h },
    )
    .expect("the region decodes")
}

fn text_line(art: &Artifact, n: usize) -> String {
    art.meta["text"][n].as_str().expect("text line").to_string()
}

/// Power-on: display off, basic set, graphics off ("Register flag initial
/// values"). An undriven panel reports zero ink, not a missing artifact.
#[test]
fn power_on_panel_reports_dark_and_unpainted() {
    let bus = rig();
    let art = artifact(&bus);
    assert_eq!(art.kind, "framebuffer");
    assert_eq!(art.meta["w"], W);
    assert_eq!(art.meta["h"], H);
    assert_eq!(art.meta["ink_bytes"], 0);
    assert_eq!(art.meta["lit_pixels"], 0);
    assert_eq!(art.meta["display_on"], false);
    assert_eq!(art.meta["graphics_on"], false);
    assert_eq!(art.meta["extended"], false);
}

/// "Serial Interface": sync 11111 RW RS 0, then upper and lower nibble each in
/// D7..D4. Display ON/OFF control 0000_1DCB with D = 1 is 0x0C.
#[test]
fn serial_instruction_frame_turns_display_on() {
    let mut bus = rig();
    send(&mut bus, &cmds(&[0x0C]));
    assert_eq!(artifact(&bus).meta["display_on"], true);
    send(&mut bus, &cmds(&[0x08]));
    assert_eq!(
        artifact(&bus).meta["display_on"],
        false,
        "D = 0 turns the display off"
    );
}

/// "Serial Interface": "When CS is low, the serial counter and data are
/// reset". A frame cut by CS is discarded rather than completed by the next
/// window's bytes.
#[test]
fn cs_low_resets_the_serial_counter() {
    let mut bus = rig();
    // Sync + upper nibble of 0x0C, then CS drops.
    send(&mut bus, &[0xF8, 0x00]);
    // The lower nibble alone in a new window is not a frame.
    send(&mut bus, &[0xC0]);
    assert_eq!(artifact(&bus).meta["display_on"], false);
}

/// The nibble split is real: an instruction sent as one unsplit byte after the
/// sync is not decoded as that instruction.
#[test]
fn unsplit_byte_is_not_an_instruction() {
    let mut bus = rig();
    send(&mut bus, &[0xF8, 0x0C, 0x00]);
    assert_eq!(artifact(&bus).meta["display_on"], false);
}

/// "DDRAM" + "Set DDRAM address": half-width codes 02H..7FH render the 16x8
/// HCGROM font, two characters per 16-bit address. On the 128x64 module the
/// four text lines start at 80H, 90H, 88H and 98H.
#[test]
fn half_width_text_lands_on_the_folded_four_lines() {
    let mut bus = rig();
    let mut w = cmds(&[0x30, 0x0C, 0x01]);
    for (addr, s) in [(0x80u8, b"AB"), (0x90, b"CD"), (0x88, b"EF"), (0x98, b"GH")] {
        cmd(&mut w, addr);
        data(&mut w, s);
    }
    send(&mut bus, &w);
    let art = artifact(&bus);
    for (n, want) in ["AB", "CD", "EF", "GH"].iter().enumerate() {
        let line = text_line(&art, n);
        assert_eq!(&line[..2], *want, "text line {n}: {line:?}");
        assert!(
            line[2..].trim().is_empty(),
            "rest of line {n} blank: {line:?}"
        );
        let (lit, _) = ink(&art, 0, n * 16, 16, 16);
        assert!(
            lit > 0,
            "line {n} glyphs are on rows {}..{}",
            n * 16,
            n * 16 + 16
        );
        let (rest, _) = ink(&art, 16, n * 16, W - 16, 16);
        assert_eq!(rest, 0, "nothing else on line {n}");
    }
    // Each half-width glyph sits in its own 8-pixel cell.
    assert!(ink(&art, 0, 0, 8, 16).0 > 0, "'A' in x 0..8");
    assert!(ink(&art, 8, 0, 8, 16).0 > 0, "'B' in x 8..16");
    assert!(art.meta["ink_bytes"].as_u64().unwrap() > 0);
}

/// "Write data to RAM": AC increments after the SECOND byte of a word, so a
/// third character opens the next address. Entry mode I/D = 0 decrements.
#[test]
fn address_counter_steps_per_word_and_follows_entry_mode() {
    let mut bus = rig();
    let mut w = cmds(&[0x30, 0x0C, 0x01, 0x80]);
    data(&mut w, b"ABC");
    send(&mut bus, &w);
    assert_eq!(&text_line(&artifact(&bus), 0)[..3], "ABC");

    let mut bus = rig();
    // I/D = 0, start at address 3, write two words: they land at 3 then 2.
    let mut w = cmds(&[0x30, 0x0C, 0x01, 0x04, 0x83]);
    data(&mut w, b"WXYZ");
    send(&mut bus, &w);
    let line = text_line(&artifact(&bus), 0);
    assert_eq!(&line[4..8], "YZWX", "I/D = 0 decrements AC: {line:?}");
}

/// "Display clear": fills DDRAM with 20H, sets AC to 00H and I/D to 1.
#[test]
fn clear_blanks_ddram_and_homes_address() {
    let mut bus = rig();
    let mut w = cmds(&[0x30, 0x0C, 0x04, 0x84]);
    data(&mut w, b"HELLO!");
    cmd(&mut w, 0x01);
    data(&mut w, b"QRST");
    send(&mut bus, &w);
    let art = artifact(&bus);
    let line = text_line(&art, 0);
    assert_eq!(
        line.trim_end(),
        "QRST",
        "clear homed AC and restored I/D = 1"
    );
    assert_eq!(ink(&art, 32, 0, W - 32, 64).0, 0, "old text erased");
}

/// "Return home": AC to 00H, DDRAM untouched.
#[test]
fn return_home_keeps_ddram() {
    let mut bus = rig();
    let mut w = cmds(&[0x30, 0x0C, 0x01, 0x80]);
    data(&mut w, b"ABCD");
    cmd(&mut w, 0x02);
    data(&mut w, b"XY");
    send(&mut bus, &w);
    assert_eq!(text_line(&artifact(&bus), 0).trim_end(), "XYCD");
}

/// "Cursor/display shift" with S/C = 0 moves AC by one word without writing.
#[test]
fn cursor_shift_moves_address_counter() {
    let mut bus = rig();
    let mut w = cmds(&[0x30, 0x0C, 0x01, 0x80, 0x14]);
    data(&mut w, b"AB");
    send(&mut bus, &w);
    assert_eq!(&text_line(&artifact(&bus), 0)[..4], "  AB");
}

/// "CGRAM" Table 5: address bits 4..5 pick the glyph, 0..3 the row, data bit
/// 15 is the leftmost pixel; DDRAM code 0002H shows glyph 1, and bits 0 and 3
/// of the code are don't-care.
#[test]
fn cgram_glyph_shows_through_its_ddram_code() {
    let mut bus = rig();
    let mut w = cmds(&[0x30, 0x0C, 0x01, 0x40 | (1 << 4)]);
    // Row 0 = 0x8001, row 1 = 0xFFFF, rows 2..15 = 0.
    let mut rows = vec![0x80, 0x01, 0xFF, 0xFF];
    rows.resize(32, 0);
    data(&mut w, &rows);
    cmd(&mut w, 0x80);
    data(&mut w, &[0x00, 0x02]);
    // The same glyph again at address 1 with the don't-care bits set: 0x0003
    // has bit 0 set, bits 1..2 still select glyph 1.
    data(&mut w, &[0x00, 0x03]);
    send(&mut bus, &w);
    let art = artifact(&bus);
    for base in [0, 16] {
        assert!(
            px(&art, base, 0),
            "bit 15 = leftmost pixel (cell at {base})"
        );
        assert!(px(&art, base + 15, 0), "bit 0 = rightmost pixel");
        assert!(!px(&art, base + 1, 0));
        assert_eq!(ink(&art, base, 1, 16, 1).0, 16, "row 1 solid");
        assert_eq!(ink(&art, base, 2, 16, 14).0, 0, "rows 2..15 clear");
    }
    assert_eq!(art.meta["lit_pixels"], 2 * (2 + 16));
}

/// Extended function set 0011_x1G0: RE = 1 switches 0x80 to "Set graphic RAM
/// address" (vertical, then horizontal). G = 1 shows GDRAM. Table 8: bit 15
/// is the leftmost pixel of a 16-pixel word.
#[test]
fn gdram_word_msb_is_leftmost_pixel() {
    let mut bus = rig();
    let mut w = cmds(&[0x30, 0x0C, 0x34, 0x36, 0x80 | 5, 0x80 | 2]);
    data(&mut w, &[0x80, 0x01]);
    send(&mut bus, &w);
    let art = artifact(&bus);
    assert_eq!(art.meta["graphics_on"], true);
    assert_eq!(art.meta["extended"], true);
    assert!(px(&art, 32, 5), "word x=2 starts at pixel 32");
    assert!(px(&art, 47, 5), "bit 0 is pixel 47");
    assert!(!px(&art, 33, 5));
    assert_eq!(art.meta["lit_pixels"], 2);
}

/// Table 8 on a 128x64 module: horizontal words 8..15 are the lower half of
/// the glass, vertical address 0..31 within it.
#[test]
fn gdram_horizontal_8_to_15_folds_to_rows_32_to_63() {
    let mut bus = rig();
    let mut w = cmds(&[0x30, 0x0C, 0x34, 0x36, 0x80 | 3, 0x80 | 8]);
    data(&mut w, &[0xFF, 0xFF]);
    cmd(&mut w, 0x80 | 31);
    cmd(&mut w, 0x80 | 15);
    data(&mut w, &[0xFF, 0xFF]);
    send(&mut bus, &w);
    let art = artifact(&bus);
    assert_eq!(ink(&art, 0, 35, 16, 1).0, 16, "x=8,y=3 -> row 35, px 0..16");
    assert_eq!(
        ink(&art, 112, 63, 16, 1).0,
        16,
        "x=15,y=31 -> row 63, px 112..128"
    );
    assert_eq!(ink(&art, 0, 0, W, 32).0, 0, "top half untouched");
    assert_eq!(art.meta["lit_pixels"], 32);
}

/// "Set graphic RAM address": only the horizontal address auto-increments,
/// and 0FH wraps to 00H without carrying into the vertical address.
#[test]
fn gdram_horizontal_address_wraps_without_vertical_carry() {
    let mut bus = rig();
    let mut w = cmds(&[0x30, 0x0C, 0x34, 0x36, 0x80 | 7, 0x80 | 15]);
    data(&mut w, &[0xFF, 0xFF, 0xFF, 0xFF]);
    send(&mut bus, &w);
    let art = artifact(&bus);
    assert_eq!(ink(&art, 112, 39, 16, 1).0, 16, "x=15,y=7 -> row 39");
    assert_eq!(ink(&art, 0, 7, 16, 1).0, 16, "wrapped to x=0 on the same y");
    assert_eq!(ink(&art, 0, 8, 16, 1).0, 0, "no vertical carry");
}

/// G = 0 keeps GDRAM off the glass; writing it is still remembered and shows
/// once G = 1.
#[test]
fn gdram_is_hidden_until_graphics_display_on() {
    let mut bus = rig();
    let mut w = cmds(&[0x30, 0x0C, 0x34, 0x80, 0x80]);
    data(&mut w, &[0xFF, 0xFF]);
    send(&mut bus, &w);
    assert_eq!(artifact(&bus).meta["lit_pixels"], 0);
    send(&mut bus, &cmds(&[0x36]));
    assert_eq!(artifact(&bus).meta["lit_pixels"], 16);
}

/// Marlin's RepRapDiscount driver (`ultralcd_st7920_u8glib_rrd_AVR.cpp`)
/// initialises with 0x20, 0x08 (display off), 0x01, 0x24, 0x26 (extended set,
/// G = 1), clears GDRAM and ends with 0x0C. With RE = 1 that last byte is the
/// extended "sleep mode" instruction (0000_1SLxx, SL = 1 leaves sleep), not
/// display on/off, so D stays 0 (Sitronix ST7920 datasheet V4.x, instruction
/// tables: display status 0000_1DCB in the basic set, sleep mode 0000_1SLxx
/// in the extended set). The datasheet describes D as the whole-display
/// switch ("D = 0: display off, DDRAM unchanged"), yet RRD panels on this
/// init show Marlin's status screen; Marlin's HAL driver
/// (`u8g_dev_st7920_128x64_HAL.cpp`, `clear_graphics_DRAM`) relies on the same
/// 0x08 / 0x3E / 0x0C order. The model therefore takes the graphic layer as
/// shown whenever G = 1 (and the part is neither asleep nor in standby),
/// independent of D, and reports the panel lit.
#[test]
fn marlin_rrd_init_shows_gdram_with_display_bit_clear() {
    let mut bus = rig();
    let mut w = cmds(&[0x20, 0x08, 0x01, 0x24, 0x26, 0x80, 0x80]);
    data(&mut w, &[0xFF, 0xFF]);
    w.extend(cmds(&[0x0C]));
    send(&mut bus, &w);
    let art = artifact(&bus);
    assert_eq!(art.meta["display_on"], false, "0x0C under RE = 1 is SL");
    assert_eq!(art.meta["sleep"], false);
    assert_eq!(art.meta["graphics_on"], true);
    assert_eq!(art.meta["lit_pixels"], 16);
    assert_eq!(art.meta["lit"], true, "GDRAM with G = 1 is on the glass");

    // Standby (extended 0x01) still blanks it.
    send(&mut bus, &cmds(&[0x01]));
    assert_eq!(artifact(&bus).meta["lit"], false);
}

/// RE selects which table an opcode is read from: after returning to the
/// basic set (0x30), 0x80 is "Set DDRAM address" again and data is text.
#[test]
fn basic_set_after_extended_reads_0x80_as_ddram_address() {
    let mut bus = rig();
    let mut w = cmds(&[0x34, 0x36, 0x30, 0x0C, 0x80]);
    data(&mut w, b"OK");
    send(&mut bus, &w);
    let art = artifact(&bus);
    assert_eq!(art.meta["extended"], false);
    assert_eq!(&text_line(&art, 0)[..2], "OK");
}

/// A full-width code (A1H and above, two bytes) selects the 16x16 CGROM. The
/// model carries no CGROM bitmaps, so those pixels are reported as UNKNOWN
/// rather than blank: a region over them cannot be measured.
#[test]
fn full_width_cgrom_cell_is_unknown_not_blank() {
    let mut bus = rig();
    let mut w = cmds(&[0x30, 0x0C, 0x01, 0x80]);
    data(&mut w, &[0xB0, 0xA1]);
    send(&mut bus, &w);
    let art = artifact(&bus);
    assert_eq!(art.meta["unknown_pixels"], 256);
    let err = artifact_region_ink(
        art.meta["format"].as_str().unwrap(),
        &art.meta,
        art.bytes.as_deref().unwrap(),
        PixelRegion {
            x: 0,
            y: 0,
            w: 16,
            h: 16,
        },
    );
    assert!(err.is_err(), "an undefined glyph must not read as blank");
}

/// Mixed text and graphics: the two layers are combined per pixel, so a solid
/// GDRAM band over a character cell inverts the glyph (a text pixel and a
/// graphics pixel at the same place cancel).
#[test]
fn text_and_graphics_layers_combine_exclusively() {
    let mut bus = rig();
    let mut w = cmds(&[0x30, 0x0C, 0x01, 0x80]);
    data(&mut w, b"A ");
    send(&mut bus, &w);
    let glyph_px = artifact(&bus).meta["lit_pixels"].as_u64().unwrap() as usize;
    assert!(glyph_px > 0);

    // Fill rows 0..15 of the first word with graphics.
    let mut w = cmds(&[0x34, 0x36]);
    for y in 0..16u8 {
        cmd(&mut w, 0x80 | y);
        cmd(&mut w, 0x80);
        data(&mut w, &[0xFF, 0xFF]);
    }
    send(&mut bus, &w);
    let art = artifact(&bus);
    assert_eq!(
        art.meta["lit_pixels"].as_u64().unwrap() as usize,
        256 - glyph_px,
        "glyph pixels cancel against the solid graphics band"
    );
}

/// Data bytes after one RS = 1 sync keep flowing as data until the next sync:
/// a long GDRAM row needs a single start byte.
#[test]
fn one_data_sync_carries_many_bytes() {
    let mut bus = rig();
    let mut w = cmds(&[0x30, 0x0C, 0x34, 0x36, 0x80, 0x80]);
    data(&mut w, &[0xFF; 16]); // 8 words = one full top-half row
    send(&mut bus, &w);
    assert_eq!(ink(&artifact(&bus), 0, 0, W, 1).0, W);
}

/// The display oracle's `display: {painted: true}` reads `ink_bytes` from a
/// `framebuffer` artifact; a painted ST7920 must satisfy it.
#[test]
fn painted_panel_satisfies_the_display_oracle_fields() {
    let mut bus = rig();
    let mut w = cmds(&[0x30, 0x0C, 0x01, 0x80]);
    data(&mut w, b"Hi");
    send(&mut bus, &w);
    let art = artifact(&bus);
    assert!(labwired_core::inspect::is_display_artifact(&art));
    assert!(art.meta["ink_bytes"].as_u64().unwrap() >= 1);
    assert_eq!(art.meta["lit"], true);
}

/// Serial-mode CS is active HIGH ("Serial Interface": SCLK is accepted while
/// CS is high). On a controller whose chip selects the bus syncs from GPIO
/// (the parking SPI of an AVR), driving the CS pin HIGH must select the panel.
#[test]
fn cs_pin_is_active_high_on_a_gpio_synced_bus() {
    let mut bus = manifest(
        "configs/chips/atmega328p.yaml",
        "spi",
        &[("cs_pin", "PB2".into())],
    );
    let portb = 0x0001_0023u64; // PINB; DDRB = +1, PORTB = +2
    bus.write_u8(portb + 1, 1 << 2).unwrap();
    bus.write_u8(portb + 2, 1 << 2).unwrap(); // CS high
    bus.sync_parking_spi_cs();
    for b in cmds(&[0x0C]) {
        bus.clock_parking_spi(b);
    }
    assert_eq!(
        artifact(&bus).meta["display_on"],
        true,
        "CS high selects the ST7920"
    );

    bus.write_u8(portb + 2, 0).unwrap(); // CS low
    bus.sync_parking_spi_cs();
    for b in cmds(&[0x08]) {
        bus.clock_parking_spi(b);
    }
    assert_eq!(
        artifact(&bus).meta["display_on"],
        true,
        "CS low deselects it; the display-off frame is not received"
    );
}
