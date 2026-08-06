// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The contract, stated without reference to how the engine is built:
//!
//! > A panel that was painted reports an artifact saying so. A panel that was
//! > NOT painted reports zero — not absence.
//!
//! Absence and zero are different findings, and only one of them is checkable.
//! `labwired_verify`'s display oracle resolves its `panel` clause against the
//! artifact `inspect` emits; a panel that emits nothing makes
//! `{painted: true, min_ink_bytes: N}` and `min_refresh_generation`
//! unresolvable no matter how correct the firmware is. Four shipped labs — IMAX
//! Console (SH1107), Weather Station and Stats Display (SSD1680 tricolor), and
//! any UC8151D lab — were unverifiable for exactly that reason, while the same
//! panels rendered perfectly in the browser: the renderer and the evidence
//! layer were two systems that did not know about each other.
//!
//! Every test here drives a panel over its OWN wire protocol — the same
//! `I2cDevice` / `SpiDevice` calls the controller makes — and then reads
//! `Machine::inspect`. Nothing downcasts to a panel type, nothing reaches into
//! a model's buffer, and every expected number is arithmetic on the bytes this
//! file sent. The two panels that already had evidence (SSD1306, ILI9341) are
//! pinned too, so a change that adds the missing ones by disturbing the
//! existing ones cannot pass.

use labwired_config::{ChipDescriptor, ExternalDevice, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::inspect::{Artifact, DeviceInspect, InspectOpts};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Machine;
use std::collections::HashMap;
use std::path::PathBuf;

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// A one-device rig on an STM32F103: the smallest thing that can hold a panel
/// on a real controller.
fn rig(device_type: &str, connection: &str, config: &[(&str, serde_yaml::Value)]) -> SystemBus {
    let chip_path = repo("configs/chips/stm32f103.yaml");
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
        name: "panel-rig".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        external_devices: vec![ExternalDevice {
            id: "panel".to_string(),
            r#type: device_type.to_string(),
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

/// Send bytes to the single I²C slave on `controller`, through the device's own
/// `I2cDevice` surface — the same calls the controller's transaction engine
/// makes on the wire.
fn i2c_write(bus: &mut SystemBus, controller: &str, bytes: &[u8]) {
    let idx = bus
        .find_peripheral_index_by_name(controller)
        .unwrap_or_else(|| panic!("{controller} registered"));
    let any = bus.peripherals[idx].dev.as_any_mut().expect("downcastable");
    let i2c = any
        .downcast_mut::<labwired_core::peripherals::i2c::I2c>()
        .expect("generic I2c controller");
    let cell = i2c.attached_devices().first().expect("a slave is attached");
    let mut dev = cell.borrow_mut();
    dev.start();
    for &b in bytes {
        dev.write(b);
    }
    dev.stop();
}

/// Clock `(dc_level, byte)` frames into the single SPI device on `controller`,
/// through its own `SpiDevice` surface. `false` = command, `true` = data, which
/// is what the D/C line carries on real silicon.
fn spi_write(bus: &mut SystemBus, controller: &str, frames: &[(bool, u8)]) {
    use labwired_core::peripherals::spi::SpiDevice;
    let idx = bus
        .find_peripheral_index_by_name(controller)
        .unwrap_or_else(|| panic!("{controller} registered"));
    let any = bus.peripherals[idx].dev.as_any_mut().expect("downcastable");
    let spi = any
        .downcast_mut::<labwired_core::peripherals::spi::Spi>()
        .expect("generic Spi controller");
    let dev: &mut Box<dyn SpiDevice> = spi
        .attached_devices
        .first_mut()
        .expect("a device is attached");
    dev.cs_select();
    for &(dc, b) in frames {
        dev.set_dc_level(dc);
        dev.transfer(b);
    }
    dev.cs_release();
}

/// Finish the rig and read the panel's device record out of `inspect`.
fn inspect_panel(mut bus: SystemBus) -> DeviceInspect {
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    bus.refresh_peripheral_index();
    let machine = Machine::new(cpu, bus);
    machine
        .inspect(None, &InspectOpts::default())
        .devices
        .into_iter()
        .find(|d| d.id == "panel")
        .expect("the declared panel is a device")
}

/// The panel's one evidence artifact, or a panic naming what came out instead.
fn evidence(device: &DeviceInspect) -> &Artifact {
    device.artifacts.first().unwrap_or_else(|| {
        panic!(
            "panel '{}' ({:?}) produced NO artifact; the display oracle has \
             nothing to resolve against, so no lab using this panel can be \
             verified however correct its firmware is",
            device.id, device.device_type
        )
    })
}

// ─── OLEDs ──────────────────────────────────────────────────────────────────

/// Control byte 0x00 opens a command stream, 0x40 a data stream — the SSD1306 /
/// SH1107 convention.
fn oled_paint(bus: &mut SystemBus, commands: &[u8], data: &[u8]) {
    let mut cmd = vec![0x00u8];
    cmd.extend_from_slice(commands);
    i2c_write(bus, "i2c1", &cmd);
    let mut px = vec![0x40u8];
    px.extend_from_slice(data);
    i2c_write(bus, "i2c1", &px);
}

/// SSD1306's payload is unchanged: same keys, same definitions, same counts.
#[test]
fn ssd1306_evidence_is_unchanged() {
    const PATTERN: [u8; 3] = [0xFF, 0x01, 0x80];
    let lit: usize = PATTERN.iter().map(|b| b.count_ones() as usize).sum();
    let mut bus = rig("oled-ssd1306", "i2c1", &[("i2c_address", 0x3Cu64.into())]);
    oled_paint(&mut bus, &[0xAF], &PATTERN);

    let device = inspect_panel(bus);
    let art = evidence(&device);
    assert_eq!(art.kind, "framebuffer");
    assert_eq!(art.id, "panel", "artifact is addressed by the device's id");
    assert_eq!(art.meta["format"], "ssd1306_page");
    assert_eq!(art.meta["ink_bytes"], PATTERN.len());
    assert_eq!(art.meta["lit_pixels"], lit);
    assert!(art.meta.get("w").is_some() && art.meta.get("h").is_some());
    assert!(art.bytes.is_none(), "summary mode omits the payload");
}

/// The IMAX Console's panel. Same 1-bpp page geometry as the SSD1306, and it
/// must report the same way.
#[test]
fn sh1107_that_painted_reports_its_ink() {
    const PATTERN: [u8; 4] = [0x01, 0xFF, 0x0F, 0x81];
    let lit: usize = PATTERN.iter().map(|b| b.count_ones() as usize).sum();
    // Display on, page 0, column 0, then the pixel stream.
    let mut bus = rig("oled-sh1107", "i2c1", &[("i2c_address", 0x3Cu64.into())]);
    oled_paint(&mut bus, &[0xAF, 0xB0, 0x00, 0x10], &PATTERN);

    let device = inspect_panel(bus);
    let art = evidence(&device);
    assert_eq!(art.meta["ink_bytes"], PATTERN.len());
    assert_eq!(art.meta["lit_pixels"], lit);
    assert_eq!(art.meta["display_on"], true, "0xAF was sent");
    assert_eq!(art.meta["w"], 128);
}

/// A panel nobody drove reports zero, not an absent artifact. "Nothing was
/// painted" is a finding and must be legible as one.
#[test]
fn unpainted_sh1107_reports_zero_rather_than_nothing() {
    let bus = rig("oled-sh1107", "i2c1", &[("i2c_address", 0x3Cu64.into())]);
    let device = inspect_panel(bus);
    let art = evidence(&device);
    assert_eq!(art.meta["ink_bytes"], 0);
    assert_eq!(art.meta["lit_pixels"], 0);
    assert_eq!(art.meta["display_on"], false);
}

// ─── tri-color e-paper ──────────────────────────────────────────────────────

/// GxEPD2's SSD1680 sequence: window + counters, the black stream (0x24), the
/// red stream (0x26), then the power/master-activation handshake that puts the
/// image on the glass.
fn ssd1680_paint(black: &[u8], red: &[u8]) -> Vec<(bool, u8)> {
    let mut f: Vec<(bool, u8)> = Vec::new();
    let cmd = |f: &mut Vec<(bool, u8)>, b: u8| f.push((false, b));
    let data = |f: &mut Vec<(bool, u8)>, b: u8| f.push((true, b));
    cmd(&mut f, 0x12); // SWRESET
    cmd(&mut f, 0x11); // data entry mode
    data(&mut f, 0x03);
    cmd(&mut f, 0x44); // RAM-X window (start/8, end/8)
    data(&mut f, 0x00);
    data(&mut f, 0x00);
    cmd(&mut f, 0x45); // RAM-Y window
    data(&mut f, 0x00);
    data(&mut f, 0x00);
    data(&mut f, (black.len() - 1) as u8);
    data(&mut f, 0x00);
    cmd(&mut f, 0x4E);
    data(&mut f, 0x00);
    cmd(&mut f, 0x4F);
    data(&mut f, 0x00);
    data(&mut f, 0x00);
    cmd(&mut f, 0x24); // black plane
    for &b in black {
        data(&mut f, b);
    }
    cmd(&mut f, 0x4E);
    data(&mut f, 0x00);
    cmd(&mut f, 0x4F);
    data(&mut f, 0x00);
    data(&mut f, 0x00);
    cmd(&mut f, 0x26); // red plane
    for &b in red {
        data(&mut f, b);
    }
    cmd(&mut f, 0x22); // GxEPD2 _PowerOn selector
    data(&mut f, 0xF8);
    cmd(&mut f, 0x20); // master activation → power on
    cmd(&mut f, 0x22); // full-update sequence selector
    data(&mut f, 0xF7);
    cmd(&mut f, 0x20); // master activation → refresh
    f
}

/// A tri-color e-paper that was streamed and refreshed reports BOTH planes and
/// the refresh that made them visible.
///
/// An e-paper plane is erased to 0xFF (a set bit is "no ink"), so an inked cell
/// is any byte that is not 0xFF — the same count the CLI's
/// `black-plane non-FF bytes=` line prints. Two of the three black bytes and one
/// of the three red bytes differ from the erased value.
#[test]
fn ssd1680_that_refreshed_reports_both_planes_and_the_refresh() {
    const BLACK: [u8; 3] = [0x00, 0xFF, 0xAA];
    const RED: [u8; 3] = [0xFF, 0xFF, 0x0F];
    let black_ink = BLACK.iter().filter(|&&b| b != 0xFF).count();
    let red_ink = RED.iter().filter(|&&b| b != 0xFF).count();

    let mut bus = rig(
        "ssd1680_tricolor_290",
        "spi1",
        &[("cs_pin", "PA4".into()), ("dc_pin", "PC7".into())],
    );
    spi_write(&mut bus, "spi1", &ssd1680_paint(&BLACK, &RED));

    let device = inspect_panel(bus);
    let art = evidence(&device);
    assert_eq!(art.meta["w"], 128);
    assert_eq!(art.meta["h"], 296);
    assert_eq!(art.meta["black_ink_bytes"], black_ink);
    assert_eq!(art.meta["red_ink_bytes"], red_ink);
    assert_eq!(
        art.meta["refresh_generation"], 2,
        "one master activation is one refresh — the only thing that \
         distinguishes 'RAM was written' from 'the image is on the glass'; \
         GxEPD2 sends two (power-on, then the update sequence)"
    );
    assert_eq!(art.meta["power_on"], true);
}

/// An e-paper nobody drove reports zero ink and generation zero — not absence,
/// and not a plausible-looking number.
#[test]
fn unrefreshed_ssd1680_reports_zero_rather_than_nothing() {
    let bus = rig(
        "ssd1680_tricolor_290",
        "spi1",
        &[("cs_pin", "PA4".into()), ("dc_pin", "PC7".into())],
    );
    let device = inspect_panel(bus);
    let art = evidence(&device);
    assert_eq!(art.meta["black_ink_bytes"], 0);
    assert_eq!(art.meta["red_ink_bytes"], 0);
    assert_eq!(art.meta["refresh_generation"], 0);
    assert_eq!(art.meta["power_on"], false);
}

/// The UC8151D's own datasheet sequence: 0x04 powers on, 0x10 opens the black
/// (B/W) stream, 0x13 the red stream, 0x12 triggers the refresh.
#[test]
fn uc8151d_that_refreshed_reports_both_planes_and_the_refresh() {
    const BLACK: [u8; 4] = [0x00, 0xFF, 0xF0, 0xFF];
    const RED: [u8; 4] = [0xFF, 0x00, 0xFF, 0xFF];
    let black_ink = BLACK.iter().filter(|&&b| b != 0xFF).count();
    let red_ink = RED.iter().filter(|&&b| b != 0xFF).count();

    let mut frames = vec![(false, 0x04u8), (false, 0x10)];
    frames.extend(BLACK.iter().map(|&b| (true, b)));
    frames.push((false, 0x13));
    frames.extend(RED.iter().map(|&b| (true, b)));
    frames.push((false, 0x12));

    let mut bus = rig(
        "uc8151d_tricolor_290",
        "spi1",
        &[("cs_pin", "PA4".into()), ("dc_pin", "PC7".into())],
    );
    spi_write(&mut bus, "spi1", &frames);

    let device = inspect_panel(bus);
    let art = evidence(&device);
    assert_eq!(art.meta["black_ink_bytes"], black_ink);
    assert_eq!(art.meta["red_ink_bytes"], red_ink);
    assert_eq!(art.meta["refresh_generation"], 1);
    assert_eq!(art.meta["power_on"], true);
}

// ─── the rest of what the browser renders ───────────────────────────────────

/// ILI9341's payload is unchanged, including the `painted_bytes` definition —
/// deliberately the same count the CLI's `painted bytes=` line prints, so the
/// two agree by construction rather than by coincidence.
#[test]
fn ili9341_evidence_is_unchanged() {
    const PIXELS: usize = 100;
    const HI: u8 = 0x07;
    const LO: u8 = 0xE0; // RGB565 green: both bytes non-zero.
    let mut frames = vec![(false, 0x29u8), (false, 0x2C)];
    for _ in 0..PIXELS {
        frames.push((true, HI));
        frames.push((true, LO));
    }
    let mut bus = rig(
        "ili9341",
        "spi1",
        &[("cs_pin", "PA4".into()), ("dc_pin", "PC7".into())],
    );
    spi_write(&mut bus, "spi1", &frames);

    let device = inspect_panel(bus);
    let art = evidence(&device);
    assert_eq!(art.meta["format"], "rgb565_be");
    assert_eq!(art.meta["display_on"], true);
    assert_eq!(art.meta["painted_bytes"], PIXELS * 2);
    assert_eq!(art.meta["top_colour"], "0x07E0");
    assert_eq!(art.meta["top_colour_pixels"], PIXELS);
}

/// The Nokia 5110's controller: bank-addressed 1-bpp, D/C framed.
#[test]
fn pcd8544_that_painted_reports_its_ink() {
    const PATTERN: [u8; 5] = [0xFF, 0x01, 0x00, 0x3C, 0x81];
    let ink = PATTERN.iter().filter(|&&b| b != 0).count();
    let lit: usize = PATTERN.iter().map(|b| b.count_ones() as usize).sum();

    // Function set (basic), display normal, then the pixel stream.
    let mut frames = vec![(false, 0x20u8), (false, 0x0C)];
    frames.extend(PATTERN.iter().map(|&b| (true, b)));
    let mut bus = rig(
        "pcd8544",
        "spi1",
        &[("cs_pin", "PA4".into()), ("dc_pin", "PC7".into())],
    );
    spi_write(&mut bus, "spi1", &frames);

    let device = inspect_panel(bus);
    let art = evidence(&device);
    assert_eq!(art.meta["ink_bytes"], ink);
    assert_eq!(art.meta["lit_pixels"], lit);
}

/// An 8×8 LED matrix module: one register write per row, one bit per LED.
#[test]
fn max7219_that_was_written_reports_its_lit_leds() {
    // Registers 0x01..0x08 are the eight digit/row registers.
    const ROWS: [u8; 3] = [0xFF, 0x81, 0x18];
    let lit: usize = ROWS.iter().map(|b| b.count_ones() as usize).sum();
    let mut frames: Vec<(bool, u8)> = vec![(false, 0x0C), (false, 0x01)]; // shutdown reg = normal
    for (i, &row) in ROWS.iter().enumerate() {
        frames.push((false, (i + 1) as u8));
        frames.push((false, row));
    }
    let mut bus = rig("led-matrix", "spi1", &[("cs_pin", "PA4".into())]);
    spi_write(&mut bus, "spi1", &frames);

    let device = inspect_panel(bus);
    let art = evidence(&device);
    assert_eq!(art.meta["lit_pixels"], lit);
    assert_eq!(art.meta["shutdown"], false);
}

/// A character LCD holds text, not pixels — so its evidence is the decoded
/// DDRAM text, and its `kind` says so rather than pretending to be a
/// framebuffer.
#[test]
fn lcd1602_that_was_written_reports_its_text() {
    let bus = rig("lcd1602", "i2c1", &[("i2c_address", 0x27u64.into())]);
    let device = inspect_panel(bus);
    let art = evidence(&device);
    assert_eq!(art.kind, "text_display");
    assert!(
        art.meta.get("text").is_some(),
        "the decoded DDRAM text is the evidence"
    );
}

// ─── the ratchet ────────────────────────────────────────────────────────────

/// A panel the browser can render but `inspect` cannot report is the defect
/// this file exists for, so it is a build failure rather than a comment.
///
/// The list is derived from the OTHER system — the wasm renderer's own
/// `get_<panel>_framebuffer` accessors in `crates/wasm/src/inspect.rs`. That is
/// deliberate: a hand-written list here would be a third source of truth about
/// what a display is, and the kit registry cannot supply one (its `Category`
/// distinguishes transports, not displays). Deriving from the renderer encodes
/// exactly the invariant that broke — the two systems must agree.
///
/// Adding a `get_foo_framebuffer` accessor without giving `foo`'s model an
/// `artifacts` impl fails here.
#[test]
fn every_panel_the_browser_renders_reports_evidence_to_inspect() {
    let wasm = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../wasm/src/inspect.rs"),
    )
    .expect("read the wasm renderer");

    // `get_<panel>_framebuffer` / `get_<panel>_text` → the panel's model module.
    let mut panels: Vec<String> = Vec::new();
    for line in wasm.lines() {
        let line = line.trim();
        for suffix in ["_framebuffer(", "_text("] {
            if let Some(rest) = line.strip_prefix("pub fn get_") {
                if let Some(name) = rest.split(suffix).next() {
                    if rest.contains(suffix) && !panels.iter().any(|p| p == name) {
                        panels.push(name.to_string());
                    }
                }
            }
        }
    }
    assert!(
        panels.len() >= 8,
        "renderer scan found only {panels:?} — the scan is broken, so this test \
         would have passed by measuring nothing"
    );

    // The renderer's accessor name is the model's module name, except where it
    // names the part rather than the controller chip.
    let module_of = |panel: &str| match panel {
        "led_matrix" => "max7219".to_string(),
        "tm1637" => "tm1637_7seg".to_string(),
        "ssd1680" => "ssd1680_tricolor_290".to_string(),
        "uc8151d" => "uc8151d_tricolor_290".to_string(),
        other => other.to_string(),
    };

    let components = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/peripherals/components");
    for panel in &panels {
        let path = components.join(format!("{}.rs", module_of(panel)));
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("the browser renders '{panel}' but {path:?}: {e}"));
        assert!(
            src.contains("fn artifacts("),
            "the browser renders '{panel}' but its model reports no artifacts to \
             inspect — every oracle clause about that panel is unresolvable. \
             Add an `artifacts` impl next to its buffers in {path:?}."
        );
    }
}
