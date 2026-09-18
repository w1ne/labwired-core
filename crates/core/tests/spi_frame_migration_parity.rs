// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **The three shift-register display drivers: the descriptors against the
//! models they replace, on a real bus, over a real SPI controller.**
//!
//! `components/max7219.rs`, `components/hc595.rs` and
//! `components/hc595_7seg.rs` are DELETED. All three are kept below as ORACLES,
//! copied verbatim (the MAX7219's artifact included), and every test drives the
//! SAME byte stream through both implementations and compares every artifact
//! byte, every `meta` field and every pad level.
//!
//! ## What is actually under test
//!
//! Not "a rule list can store a byte". Four claims, each of which was a
//! measured gap before this change:
//!
//! 1. **A rule can read a frame's ADDRESS byte.** `on: frame` hands a rule
//!    `written` — one byte, the one that CLOSED the frame. A MAX7219
//!    transaction is `[address, data]`, so until now the address was
//!    unreachable and all thirteen registers with it. #1181 measured that
//!    `frames.opcode_byte` was a declared key with exactly one hit in
//!    `crates/`: the struct field. Every MAX7219 test here is a test of that,
//!    because every guard in `max7219.yaml` reads `var(opcode)` and every data
//!    write reads `frame_byte(1)` — a build where either is unwired paints
//!    nothing and fails here.
//! 2. **`powered:` is honoured by `GenericSpiDevice`.** It was not, at all
//!    (`grep -rn powered declarative_spi.rs` was empty), while the deleted
//!    models each gated on it — so porting without this would have silently
//!    lost the "module has no rail" finding.
//! 3. **An SPI part can DRIVE PADS.** The deleted `Hc595` drove none: its
//!    `outputs()` was a field nothing on the board could see.
//!    [`a_latched_byte_reaches_the_pads`] reads the levels back out of the
//!    GPIO input register, through the tick, never off the device.
//! 4. **Byte parity with the deleted models**, plus two DELIBERATE differences
//!    that are measured rather than left to be rediscovered — the 74HC595 latch
//!    EDGE and the dual-595 module's byte-order SEARCH. Each has its own test
//!    naming what changed and why.
//!
//! ## The stimulus goes through the controller's device surface
//!
//! Every byte below is a `SpiDevice::transfer` on the device the real attach
//! path put on the real `Spi` controller — the same call the controller's
//! transaction engine makes — and every readback is `Machine::inspect` or the
//! GPIO input register. Nothing downcasts to a model type and nothing reaches
//! into a buffer.

use labwired_config::{ChipDescriptor, ExternalDevice, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::inspect::{Artifact, DeviceInspect, InspectOpts};
use labwired_core::peripherals::spi::SpiDevice;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Machine};
use std::collections::HashMap;
use std::path::PathBuf;

// ─── the oracles: the deleted models, verbatim ─────────────────────────────

/// The shared 7-segment font, copied verbatim from
/// `components/seven_seg_font.rs`.
///
/// ⚠️ COPIED, NOT IMPORTED, for the reason `gpio_artifact_migration_parity`
/// states: importing the engine's table would make every `text` assertion
/// compare the engine against itself.
// ⚠️ `#![allow(dead_code)]` on the oracles only: they are COPIES, and trimming
// an unread field (each model's `cs_pin`, the 595's `component_id`) would make
// them something other than what was deleted.
#[allow(dead_code)]
mod oracle_font {
    pub const FONT: &[(u8, char)] = &[
        (0x3F, '0'),
        (0x06, '1'),
        (0x5B, '2'),
        (0x4F, '3'),
        (0x66, '4'),
        (0x6D, '5'),
        (0x7D, '6'),
        (0x07, '7'),
        (0x7F, '8'),
        (0x6F, '9'),
        (0x77, 'A'),
        (0x7C, 'b'),
        (0x39, 'C'),
        (0x5E, 'd'),
        (0x79, 'E'),
        (0x71, 'F'),
        (0x40, '-'),
        (0x00, ' '),
    ];

    pub fn decode(seg: u8) -> char {
        let glyph = seg & 0x7F;
        FONT.iter()
            .find(|(pattern, _)| *pattern == glyph)
            .map(|(_, ch)| *ch)
            .unwrap_or('?')
    }
}

// ── MAX7219 ──
/// Rows on the matrix — equivalently, the number of digit registers driven.
const ROWS: usize = 8;

// Register addresses (datasheet Table 2).
const REG_NOOP: u8 = 0x00;
const REG_DIGIT0: u8 = 0x01;
const REG_DIGIT7: u8 = 0x08;
const REG_DECODE_MODE: u8 = 0x09;
const REG_INTENSITY: u8 = 0x0A;
const REG_SCAN_LIMIT: u8 = 0x0B;
const REG_SHUTDOWN: u8 = 0x0C;
const REG_DISPLAY_TEST: u8 = 0x0F;

/// Simulated MAX7219-driven 8×8 LED matrix.
#[allow(dead_code)]
#[derive(Debug)]
pub struct Max7219 {
    /// `CS`/`LOAD` line, wired to the GPIO used as SPI chip-select.
    cs_pin: String,
    /// Whether the module's supply pins (VCC, GND) are connected in the design.
    ///
    /// ⚠️ NOT the MAX7219's own `shutdown` register below. Shutdown is a mode
    /// the firmware selects over a bus that works; this is whether the module
    /// has a rail at all. A diagram wiring only DIN/CLK/CS used to run the
    /// whole init and report lit LEDs; on a bench the matrix is dark.
    ///
    /// ⚠️ DEFAULTS TO `true`. Only an explicit `powered: false` in the compiled
    /// manifest darkens it — see [`crate::peripherals::components::supply`].
    powered: bool,
    /// Two-byte shift accumulator for the 16-bit write currently clocking in.
    shift: [u8; 2],
    /// Number of bytes clocked into `shift` since the last latch.
    shift_len: usize,
    /// Digit RAM: one byte per row, index 0 = digit 0 (register `0x01`).
    framebuffer: [u8; ROWS],
    /// Decode-mode register (`0x09`). `0x00` = no decode, as an 8×8 matrix needs.
    decode_mode: u8,
    /// Intensity register (`0x0A`), 0…15 in the low nibble.
    intensity: u8,
    /// Scan-limit register (`0x0B`): how many digits are multiplexed, 0…7.
    scan_limit: u8,
    /// True while the driver is in shutdown mode (register `0x0C` data bit 0 clear).
    shutdown: bool,
    /// True while display test is active (register `0x0F` data bit 0 set).
    display_test: bool,
}

impl Max7219 {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        Self {
            cs_pin: cs_pin.into(),
            // Absent supply information means "powered" — see the field's note.
            powered: true,
            shift: [0; 2],
            shift_len: 0,
            framebuffer: [0; ROWS],
            // Power-on defaults per datasheet: all registers cleared, which
            // leaves the part shut down with display test off.
            decode_mode: 0,
            intensity: 0,
            scan_limit: 0,
            shutdown: true,
            display_test: false,
        }
    }

    /// The eight row bytes the panel is currently lighting, row 0 first.
    ///
    /// Display test floods every LED and shutdown blanks the panel — both
    /// without disturbing digit RAM, so the stored rows reappear untouched when
    /// the mode is cleared. Reporting the raw RAM in those states would render a
    /// picture the real panel is not showing.
    pub fn framebuffer(&self) -> [u8; ROWS] {
        // No supply, no LEDs — ahead of display test, which on silicon
        // overrides shutdown but cannot override the absence of a rail.
        if !self.powered {
            return [0x00; ROWS];
        }
        if self.display_test {
            // Display test overrides shutdown (datasheet: "display-test mode
            // overrides shutdown mode").
            return [0xFF; ROWS];
        }
        if self.shutdown {
            return [0x00; ROWS];
        }
        self.framebuffer
    }

    /// Digit RAM as written by the firmware, ignoring shutdown / display test.
    pub fn digit_ram(&self) -> [u8; ROWS] {
        self.framebuffer
    }

    /// Latched decode-mode register (`0x09`).
    pub fn decode_mode(&self) -> u8 {
        self.decode_mode
    }

    /// Latched intensity register (`0x0A`), 0…15 in the low nibble.
    pub fn intensity(&self) -> u8 {
        self.intensity
    }

    /// Latched scan-limit register (`0x0B`), 0…7.
    pub fn scan_limit(&self) -> u8 {
        self.scan_limit
    }

    /// True while the driver is in shutdown mode. A module with no supply is
    /// reported as shut down as well: `transfer` refuses the bus, so the part
    /// stays at its power-on-shutdown default by construction.
    pub fn is_shutdown(&self) -> bool {
        self.shutdown
    }

    /// Declare whether the module's supply is connected. See the `powered`
    /// field. Only ever called with `false`, from `attach`, when the compiled
    /// manifest explicitly says the supply pins are on no net.
    pub fn with_powered(mut self, powered: bool) -> Self {
        self.powered = powered;
        self
    }

    /// True when the module has a supply. See the `powered` field.
    pub fn powered(&self) -> bool {
        self.powered
    }

    /// True while display test is active.
    pub fn is_display_test(&self) -> bool {
        self.display_test
    }

    /// Apply a fully clocked-in 16-bit write: `shift[0]` is the address byte,
    /// `shift[1]` the data byte. Addresses outside the documented map are
    /// ignored, matching a part that simply decodes nothing for them.
    fn latch_frame(&mut self) {
        let (addr, data) = (self.shift[0], self.shift[1]);
        // Only the low nibble of the address byte is decoded; the upper bits are
        // don't-care on real silicon.
        match addr & 0x0F {
            REG_NOOP => {}
            REG_DIGIT0..=REG_DIGIT7 => {
                self.framebuffer[(addr & 0x0F) as usize - 1] = data;
            }
            REG_DECODE_MODE => self.decode_mode = data,
            REG_INTENSITY => self.intensity = data,
            REG_SCAN_LIMIT => self.scan_limit = data,
            // Data bit 0: 0 = shutdown, 1 = normal operation.
            REG_SHUTDOWN => self.shutdown = data & 1 == 0,
            REG_DISPLAY_TEST => self.display_test = data & 1 != 0,
            _ => {}
        }
    }

    fn push_byte(&mut self, byte: u8) {
        let idx = self.shift_len % 2;
        self.shift[idx] = byte;
        self.shift_len += 1;
        if self.shift_len == 2 {
            self.latch_frame();
            self.shift_len = 0;
        }
    }
}

impl Max7219 {
    /// The artifact the deleted model published, as its `meta` object.
    ///
    /// Copied from the model's `artifacts` impl verbatim except for the two
    /// engine calls it made: `artifact_format::MAX7219_ROWS` is inlined as the
    /// literal it is, and `artifact_generation` is called on the engine because
    /// it is a CONTENT HASH — the test's claim is that both sides hash the same
    /// bytes, and a copied hash function would pass even if they did not.
    fn artifact_meta(&self) -> serde_json::Value {
        let fb = self.framebuffer();
        serde_json::json!({
            "w": 8,
            "h": fb.len(),
            "format": "max7219_rows",
            "generation": labwired_core::inspect::artifact_generation(&fb),
            "ink_bytes": fb.iter().filter(|&&b| b != 0).count(),
            "lit_pixels": fb.iter().map(|b| b.count_ones() as usize).sum::<usize>(),
            "shutdown": self.is_shutdown(),
            "powered": self.powered,
            "intensity": self.intensity(),
            "scan_limit": self.scan_limit(),
        })
    }

    fn cs_select(&mut self) {
        // CS asserted → start of a fresh 16-bit write. Discard any partial frame
        // so a byte-misaligned burst can't shift the address/data pairing.
        self.shift_len = 0;
    }

    fn cs_release(&mut self) {
        // Each 16-bit write is latched in `push_byte`; nothing extra to do on
        // release. Reset the partial counter for the next assertion.
        self.shift_len = 0;
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        // THE ONE GATE THAT MAKES AN UNPOWERED MODULE BEHAVE LIKE ONE. Every
        // state change this model has — digit RAM, intensity, scan limit,
        // shutdown, display test — arrives through `transfer` as a 16-bit
        // register write. Refusing the bus here leaves the part at its
        // power-on defaults (shut down, RAM clear) by construction rather than
        // masking the readback at report time.
        if !self.powered {
            return 0;
        }
        self.push_byte(mosi);
        // DOUT is the delayed DIN used for cascading; a single (uncascaded)
        // module presents nothing meaningful on MISO.
        0
    }
}

// ── HC595 ──
/// Simulated 74HC595 8-bit serial-in / parallel-out shift register.
#[allow(dead_code)]
#[derive(Debug)]
pub struct Hc595 {
    /// Latch line (`RCLK`), wired to the GPIO used as SPI chip-select.
    cs_pin: String,
    /// Bits clocked in but not yet latched. bit 7 = QH end, bit 0 = QA end.
    shift_reg: u8,
    /// Latched value currently driven on the parallel outputs QA..QH.
    output_latch: u8,
    /// system.yaml `external_devices` id, stamped at attach if the bus wires
    /// one (see [`crate::sim_input::SimInput::component_id`]). Retained for
    /// readback identity; a pure output register serves no `SimInput` channel.
    component_id: Option<String>,
}

impl Hc595 {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        Self {
            cs_pin: cs_pin.into(),
            shift_reg: 0,
            output_latch: 0,
            component_id: None,
        }
    }

    /// Read back the latched parallel outputs: bit 0 = QA … bit 7 = QH.
    pub fn outputs(&self) -> u8 {
        self.output_latch
    }
}

impl Hc595 {
    fn cs_select(&mut self) {
        // RCLK rising edge → latch the shift register into the output pins.
        self.output_latch = self.shift_reg;
    }

    fn cs_release(&mut self) {}

    fn transfer(&mut self, mosi: u8) -> u8 {
        // A single 8-bit stage: the transferred byte becomes the new shift
        // register contents (MSB-first → bit 7 = QH, bit 0 = QA). Outputs are
        // untouched until the next latch (`cs_select`).
        self.shift_reg = mosi;
        0 // 595 has no MISO (QH' daisy-chain out is not modelled here).
    }
}

// ── HC595 7SEG ──
const DIGITS: usize = 4;

/// Simulated dual-74HC595 4-digit 7-segment LED display.
#[allow(dead_code)]
#[derive(Debug)]
pub struct Hc5957Seg {
    /// Latch line (`RCLK`), wired to the GPIO used as SPI chip-select.
    cs_pin: String,
    /// Whether the module's supply pins (VCC, GND) are connected in the design.
    ///
    /// The LED segments draw their current from the rail, so a diagram wiring
    /// only SER/SRCLK/RCLK produces a module that on a bench shows nothing
    /// while the twin reported decoded digits.
    ///
    /// ⚠️ DEFAULTS TO `true`. Only an explicit `powered: false` in the compiled
    /// manifest darkens it — see [`crate::peripherals::components::supply`].
    powered: bool,
    /// Two-byte shift accumulator for the frame currently being clocked in.
    shift: [u8; 2],
    /// Number of bytes clocked into `shift` since the last latch.
    shift_len: u8,
    /// Latched segment byte per digit (index 0 = leftmost digit).
    segments: [u8; DIGITS],
}

impl Hc5957Seg {
    pub fn new(cs_pin: impl Into<String>) -> Self {
        Self {
            cs_pin: cs_pin.into(),
            // Absent supply information means "powered" — see the field's note.
            powered: true,
            shift: [0; 2],
            shift_len: 0,
            segments: [0; DIGITS],
        }
    }

    /// Declare whether the module's supply is connected. See the `powered`
    /// field. Only ever called with `false`, from `attach`, when the compiled
    /// manifest explicitly says the supply pins are on no net.
    pub fn with_powered(mut self, powered: bool) -> Self {
        self.powered = powered;
        self
    }

    /// True when the module has a supply. See the `powered` field.
    pub fn powered(&self) -> bool {
        self.powered
    }

    /// Latched raw segment byte for `digit` (0..4), `0b0gfedcba` (dp = bit 7).
    pub fn segment_byte(&self, digit: usize) -> u8 {
        self.segments.get(digit).copied().unwrap_or(0)
    }

    /// Decode a latched segment byte to the character it displays. The decimal
    /// point (bit 7) is ignored for the glyph match; unknown patterns render as
    /// `?` so a mis-driven panel is visible rather than silently blank.
    fn decode(seg: u8) -> char {
        oracle_font::decode(seg)
    }

    /// The four decoded characters, leftmost digit first.
    pub fn chars(&self) -> [char; DIGITS] {
        let mut out = [' '; DIGITS];
        for (i, seg) in self.segments.iter().enumerate() {
            out[i] = Self::decode(*seg);
        }
        out
    }

    /// True when digit `i` has its decimal-point segment (bit 7) lit.
    pub fn decimal_point(&self, digit: usize) -> bool {
        self.segments.get(digit).is_some_and(|s| s & 0x80 != 0)
    }

    /// The whole panel as a 4-char string (for logs / assertions / the bridge).
    pub fn text(&self) -> String {
        self.chars().iter().collect()
    }

    /// Return the one-hot digit index encoded by `byte`, if it selects exactly
    /// one of the four common lines — active-high (`0x1/0x2/0x4/0x8`) or the
    /// active-low complement. Returns `None` when the byte is not a valid
    /// one-hot digit select (so the caller knows it must be the segment byte).
    fn digit_select_index(byte: u8) -> Option<usize> {
        // Active-high select is one of 0x01/0x02/0x04/0x08; active-low is the
        // bitwise complement (0xFE/0xFD/0xFB/0xF7). In both cases exactly one of
        // the four low bits is the "selected" line and the high nibble carries
        // no digit lines. No standard 7-segment glyph is a single low-nibble
        // bit, so this never mistakes segment data for a digit select.
        for candidate in [byte, !byte] {
            if candidate & 0xF0 == 0 && (candidate & 0x0F).count_ones() == 1 {
                return Some((candidate & 0x0F).trailing_zeros() as usize);
            }
        }
        None
    }

    /// Process a fully clocked-in 16-bit frame: identify which byte is the
    /// digit select and which is the segments, then latch the segments into
    /// that digit.
    fn latch_frame(&mut self) {
        let (b0, b1) = (self.shift[0], self.shift[1]);
        // Try each byte as the digit-select; the other is the segment data.
        let resolved = Self::digit_select_index(b1)
            .map(|d| (d, b0))
            .or_else(|| Self::digit_select_index(b0).map(|d| (d, b1)));
        if let Some((digit, seg)) = resolved {
            if digit < DIGITS {
                self.segments[digit] = seg;
            }
        }
    }

    fn push_byte(&mut self, byte: u8) {
        let idx = (self.shift_len as usize) % 2;
        self.shift[idx] = byte;
        self.shift_len += 1;
        if self.shift_len == 2 {
            self.latch_frame();
            self.shift_len = 0;
        }
    }
}

impl Hc5957Seg {
    fn cs_select(&mut self) {
        // RCLK asserted → start of a fresh 16-bit shift. Discard any partial
        // frame so a byte-misaligned burst can't smear across digits.
        self.shift_len = 0;
    }

    fn cs_release(&mut self) {
        // A frame is latched every two bytes in `push_byte`; nothing extra to
        // do on release. Reset the partial counter for the next assertion.
        self.shift_len = 0;
    }

    fn transfer(&mut self, mosi: u8) -> u8 {
        // THE ONE GATE THAT MAKES AN UNPOWERED MODULE BEHAVE LIKE ONE. Both
        // 74HC595s latch only what was shifted through `transfer`, so refusing
        // the bus here leaves every segment byte at zero — a blank display —
        // by construction rather than by blanking the readback.
        if !self.powered {
            return 0;
        }
        self.push_byte(mosi);
        0 // shift registers have no meaningful MISO on this module.
    }
}

// ─── the rig ───────────────────────────────────────────────────────────────

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// STM32F103 GPIOA: IDR at +0x08. The eight 74HC595 outputs are bound to
/// PA0..PA7 so one register read is the whole parallel port.
const GPIOA: u64 = 0x4001_0800;
const GPIOA_IDR: u64 = GPIOA + 0x08;
/// RCC APB2ENR — ungates GPIOA/GPIOB/GPIOC and SPI1. Without it the ports'
/// register files are dead and every pad assertion would measure the clock gate.
const RCC_APB2ENR: u64 = 0x4002_1018;

/// A one-device rig on an STM32F103, the same shape `panel_artifact_evidence`
/// uses — built through `SystemBus::from_config`, i.e. the real kit registry
/// and the real attach path, so a descriptor that fails to register fails here.
fn rig(device_type: &str, config: &[(&str, serde_yaml::Value)]) -> SystemBus {
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
        name: "shift-register-rig".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        cpu_hz: None,
        external_devices: vec![ExternalDevice {
            id: "panel".to_string(),
            r#type: device_type.to_string(),
            connection: "spi1".to_string(),
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
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    bus.write_u32(RCC_APB2ENR, 0xFFFF)
        .expect("ungate GPIO + SPI");
    bus
}

fn s(v: &str) -> serde_yaml::Value {
    serde_yaml::Value::from(v)
}

/// Clock one CS-framed transaction into the single SPI device, through its own
/// `SpiDevice` surface: CS↓, the bytes, CS↑.
fn spi_txn(bus: &mut SystemBus, bytes: &[u8]) -> Vec<u8> {
    let idx = bus
        .find_peripheral_index_by_name("spi1")
        .expect("spi1 registered");
    let any = bus.peripherals[idx].dev.as_any_mut().expect("downcastable");
    let spi = any
        .downcast_mut::<labwired_core::peripherals::spi::Spi>()
        .expect("generic Spi controller");
    let dev: &mut Box<dyn SpiDevice> = spi
        .attached_devices
        .first_mut()
        .expect("a device is attached");
    dev.cs_select();
    let miso: Vec<u8> = bytes.iter().map(|b| dev.transfer(*b)).collect();
    dev.cs_release();
    miso
}

/// One peripheral tick, which is where `service_device_pin_drives` drains what
/// the rules queued onto the pads.
fn tick(bus: &mut SystemBus) {
    let _ = bus.tick_peripherals_fully();
}

/// The eight pads PA0..PA7 as one byte, read out of the GPIO INPUT register —
/// which is what `digitalRead` resolves through.
fn pads(bus: &mut SystemBus) -> u8 {
    (bus.read_u32(GPIOA_IDR).expect("GPIOA IDR readable") & 0xFF) as u8
}

/// Finish the rig and read the panel's one artifact out of `Machine::inspect`.
/// Consumes the bus, so it is the last thing a test does.
fn inspect_artifact(mut bus: SystemBus) -> Artifact {
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    bus.refresh_peripheral_index();
    let machine = Machine::new(cpu, bus);
    let device: DeviceInspect = machine
        .inspect(
            None,
            &InspectOpts {
                include_bytes: true,
                ..Default::default()
            },
        )
        .devices
        .into_iter()
        .find(|d| d.id == "panel")
        .expect("the declared panel is a device");
    device.artifacts.into_iter().next().unwrap_or_else(|| {
        panic!(
            "the part produced NO artifact. A ported display that simulates \
             perfectly and inspects as nothing is the exact state the \
             `artifact:` key exists to end."
        )
    })
}

/// Compare an artifact against the oracle's, key by key BOTH WAYS, naming the
/// first disagreement. `assert_eq!` on two `Value`s names neither.
fn assert_same_meta(got: &Artifact, want: &serde_json::Value, what: &str) {
    let want = want.as_object().expect("oracle meta is an object");
    let have = got.meta.as_object().expect("artifact meta is an object");
    for (key, value) in want {
        assert_eq!(
            have.get(key),
            Some(value),
            "{what}: meta['{key}'] — the descriptor says {:?}, the deleted model said {value:?}",
            have.get(key)
        );
    }
    let mut extra: Vec<&String> = have.keys().filter(|k| !want.contains_key(*k)).collect();
    extra.sort();
    assert!(
        extra.is_empty(),
        "{what}: the descriptor publishes meta keys the deleted model did not: {extra:?}"
    );
}

// ─── MAX7219 ───────────────────────────────────────────────────────────────

fn max7219_rig(powered: bool) -> SystemBus {
    if powered {
        rig("led-matrix", &[("cs_pin", s("PC13"))])
    } else {
        rig(
            "led-matrix",
            &[
                ("cs_pin", s("PC13")),
                ("powered", serde_yaml::Value::from(false)),
            ],
        )
    }
}

/// One 16-bit register write, into BOTH implementations.
fn max_write(bus: &mut SystemBus, oracle: &mut Max7219, addr: u8, data: u8) {
    spi_txn(bus, &[addr, data]);
    oracle.cs_select();
    oracle.transfer(addr);
    oracle.transfer(data);
    oracle.cs_release();
}

/// The init every MAX7219 driver does, then three rows of a pattern.
const INIT_AND_ROWS: &[(u8, u8)] = &[
    (0x0C, 0x01), // shutdown register = normal operation
    (0x09, 0x00), // decode mode = none (a matrix, not digits)
    (0x0A, 0x07), // intensity
    (0x0B, 0x07), // scan limit = all eight
    (0x01, 0xFF),
    (0x02, 0x81),
    (0x03, 0x18),
    (0x08, 0x5A),
];

/// The whole register map, both implementations, every published field.
#[test]
fn a_full_register_sweep_matches_the_deleted_model_field_for_field() {
    let mut bus = max7219_rig(true);
    let mut oracle = Max7219::new("PC13");
    for &(addr, data) in INIT_AND_ROWS {
        max_write(&mut bus, &mut oracle, addr, data);
    }
    let art = inspect_artifact(bus);
    assert_eq!(art.kind, "framebuffer");
    assert_eq!(art.id, "panel");
    assert_same_meta(&art, &oracle.artifact_meta(), "full register sweep");
    assert_eq!(
        art.bytes.as_deref(),
        Some(&oracle.framebuffer()[..]),
        "the eight row bytes themselves must match, not only the derived counts",
    );
    // …and the numbers are what this file SENT, so a build where both sides are
    // equally blank cannot pass.
    assert_eq!(art.meta["lit_pixels"], 8 + 2 + 2 + 4);
    assert_eq!(art.meta["intensity"], 7);
    assert_eq!(art.meta["shutdown"], false);
}

/// ⚠️ THE ADDRESS BYTE IS THE TEST. Eight rows written to eight different
/// register addresses in one sweep: a build where `frames.opcode_byte` or
/// `frame_byte(N)` is unwired writes every frame to the same place (or to
/// none), and the row bytes come back wrong rather than merely blank.
#[test]
fn each_digit_register_addresses_its_own_row() {
    let mut bus = max7219_rig(true);
    let mut oracle = Max7219::new("PC13");
    max_write(&mut bus, &mut oracle, 0x0C, 0x01);
    for row in 0..8u8 {
        max_write(&mut bus, &mut oracle, 0x01 + row, 0x10 + row);
    }
    let art = inspect_artifact(bus);
    assert_eq!(
        art.bytes.as_deref(),
        Some(&[0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17][..]),
        "each address must land in its OWN row, in order",
    );
    assert_same_meta(&art, &oracle.artifact_meta(), "per-register rows");
}

/// Shutdown blanks the panel WITHOUT disturbing digit RAM, so the stored
/// pattern reappears when the mode is cleared. That is what
/// `artifact.blank_when` renders and what a shadow-RAM rule could not.
#[test]
fn shutdown_blanks_the_panel_and_the_pattern_returns() {
    let mut bus = max7219_rig(true);
    let mut oracle = Max7219::new("PC13");
    max_write(&mut bus, &mut oracle, 0x0C, 0x01);
    max_write(&mut bus, &mut oracle, 0x01, 0xAA);
    max_write(&mut bus, &mut oracle, 0x08, 0x55);
    // Shutdown: data bit 0 clear.
    max_write(&mut bus, &mut oracle, 0x0C, 0x00);
    {
        let art = inspect_artifact(max7219_replay(INIT_SHUTDOWN_SCRIPT));
        assert_eq!(
            art.bytes.as_deref(),
            Some(&[0u8; 8][..]),
            "panel reads blank"
        );
        assert_eq!(art.meta["shutdown"], true);
        assert_eq!(art.meta["lit_pixels"], 0);
    }
    // Back to normal operation: the stored rows reappear untouched.
    max_write(&mut bus, &mut oracle, 0x0C, 0x01);
    let art = inspect_artifact(bus);
    assert_eq!(oracle.framebuffer()[0], 0xAA, "oracle control");
    assert_same_meta(&art, &oracle.artifact_meta(), "shutdown then resume");
    assert_eq!(art.bytes.as_deref(), Some(&oracle.framebuffer()[..]));
}

/// The script the blanking assertion above replays into a fresh rig, so the
/// "blank" reading is taken on a part that is actually in shutdown rather than
/// on one the test has already brought back out of it.
const INIT_SHUTDOWN_SCRIPT: &[(u8, u8)] = &[(0x0C, 0x01), (0x01, 0xAA), (0x08, 0x55), (0x0C, 0x00)];

fn max7219_replay(script: &[(u8, u8)]) -> SystemBus {
    let mut bus = max7219_rig(true);
    for &(addr, data) in script {
        spi_txn(&mut bus, &[addr, data]);
    }
    bus
}

/// ⚠️ DISPLAY TEST OVERRIDES SHUTDOWN (datasheet Table 10), which is why
/// `fill_when` is evaluated BEFORE `blank_when`. Checked the other way round,
/// a display-test write on a shut-down panel would be invisible.
#[test]
fn display_test_floods_and_overrides_shutdown() {
    let mut bus = max7219_rig(true);
    let mut oracle = Max7219::new("PC13");
    max_write(&mut bus, &mut oracle, 0x0C, 0x01);
    max_write(&mut bus, &mut oracle, 0x02, 0x01);
    max_write(&mut bus, &mut oracle, 0x0F, 0x01); // display test on
    max_write(&mut bus, &mut oracle, 0x0C, 0x00); // …and shut down as well
    let art = inspect_artifact(bus);
    assert_eq!(oracle.framebuffer(), [0xFF; 8], "oracle control: flooded");
    assert_eq!(art.bytes.as_deref(), Some(&[0xFFu8; 8][..]));
    assert_eq!(art.meta["lit_pixels"], 64);
    assert_eq!(
        art.meta["shutdown"], true,
        "the shutdown REGISTER is still reported — a flooded panel that is also \
         shut down is a different finding from a flooded one that is not",
    );
    assert_same_meta(&art, &oracle.artifact_meta(), "display test over shutdown");
}

/// The no-op register and the two undocumented addresses decode nothing — so no
/// rule matches, and the panel is untouched. A descriptor whose guards were
/// wrong would write a data byte somewhere for one of these.
#[test]
fn the_noop_and_undocumented_addresses_change_nothing() {
    let mut bus = max7219_rig(true);
    let mut oracle = Max7219::new("PC13");
    max_write(&mut bus, &mut oracle, 0x0C, 0x01);
    max_write(&mut bus, &mut oracle, 0x01, 0x11);
    for addr in [0x00u8, 0x0D, 0x0E] {
        max_write(&mut bus, &mut oracle, addr, 0xFF);
    }
    let art = inspect_artifact(bus);
    assert_eq!(art.bytes.as_deref(), Some(&oracle.framebuffer()[..]));
    assert_eq!(art.meta["lit_pixels"], 2, "only the 0x11 row is lit");
    assert_eq!(
        art.meta["shutdown"], false,
        "0x00 is NOT the shutdown register"
    );
    assert_same_meta(&art, &oracle.artifact_meta(), "noop and undocumented");
}

/// ⚠️ A 16-BIT WRITE IS LATCHED OR IT NEVER HAPPENED. An odd byte left in the
/// shifter must not become the next transaction's address byte, and must not be
/// delivered as a one-byte frame whose low nibble decodes as a register.
///
/// This is `frames.discard_partial`, and the oracle's `cs_select` reset is what
/// it reproduces. 0x03 alone would otherwise address digit register 3 with a
/// data byte of 0 and blank a row the firmware lit.
#[test]
fn an_orphan_byte_is_discarded_rather_than_decoded() {
    let mut bus = max7219_rig(true);
    let mut oracle = Max7219::new("PC13");
    max_write(&mut bus, &mut oracle, 0x0C, 0x01);
    max_write(&mut bus, &mut oracle, 0x03, 0x3C);

    // One byte, then CS↑: an address with no data.
    spi_txn(&mut bus, &[0x03]);
    oracle.cs_select();
    oracle.transfer(0x03);
    oracle.cs_release();

    max_write(&mut bus, &mut oracle, 0x05, 0x99);
    let art = inspect_artifact(bus);
    assert_eq!(
        art.bytes.as_deref().map(|b| b[2]),
        Some(0x3C),
        "row 3 must still hold what firmware put there — the orphan 0x03 must \
         not have paired with a zero data byte",
    );
    assert_eq!(art.bytes.as_deref().map(|b| b[4]), Some(0x99));
    assert_same_meta(&art, &oracle.artifact_meta(), "orphan byte");
}

// ── the supply gate ────────────────────────────────────────────────────────

/// The POSITIVE control. Without it, "unpowered is dark" would also pass on a
/// build that never lights anything.
#[test]
fn a_powered_matrix_driven_this_way_lights_its_leds() {
    let mut bus = max7219_rig(true);
    for &(addr, data) in INIT_AND_ROWS {
        spi_txn(&mut bus, &[addr, data]);
    }
    let art = inspect_artifact(bus);
    assert_eq!(
        art.meta["powered"], true,
        "no supply key at all must mean POWERED"
    );
    assert_eq!(art.meta["shutdown"], false);
    assert!(art.meta["lit_pixels"].as_u64().unwrap() > 0);
}

/// The FIX, end to end through a real manifest: identical drive, supply
/// declared absent, every field the bug reported goes dark — and the artifact
/// says WHY.
#[test]
fn an_unpowered_matrix_reports_dark_on_every_field() {
    let mut bus = max7219_rig(false);
    let mut oracle = Max7219::new("PC13").with_powered(false);
    for &(addr, data) in INIT_AND_ROWS {
        spi_txn(&mut bus, &[addr, data]);
        oracle.cs_select();
        oracle.transfer(addr);
        oracle.transfer(data);
        oracle.cs_release();
    }
    let art = inspect_artifact(bus);
    assert_eq!(
        art.meta["powered"], false,
        "the artifact must say why it is dark"
    );
    assert_eq!(art.meta["lit_pixels"], 0, "no supply, no light");
    assert_eq!(art.meta["ink_bytes"], 0);
    assert_eq!(
        art.meta["intensity"], 0,
        "the intensity write never latched"
    );
    assert_eq!(art.meta["scan_limit"], 0);
    assert_eq!(
        art.meta["shutdown"], true,
        "an unpowered driver cannot honour the shutdown-clear write",
    );
    assert_eq!(
        art.bytes.as_deref(),
        Some(&[0u8; 8][..]),
        "digit RAM must be UNTOUCHED, not merely blanked at readback",
    );
    assert_same_meta(&art, &oracle.artifact_meta(), "unpowered");
}

/// Display test cannot light a module with no rail. A gate placed only on the
/// artifact's `fill_when` would flood an unpowered panel with 64 lit LEDs.
#[test]
fn display_test_cannot_light_an_unpowered_matrix() {
    let mut bus = max7219_rig(false);
    spi_txn(&mut bus, &[0x0F, 0x01]);
    let art = inspect_artifact(bus);
    assert_eq!(art.bytes.as_deref(), Some(&[0u8; 8][..]));
    assert_eq!(art.meta["lit_pixels"], 0);
}

/// ⚠️ A NAMED DIFFERENCE. The deleted models answered `0x00` on MISO whether
/// they had a rail or not. A chip with no supply drives NOTHING, so the master
/// clocks in the idle bus — reported here, as everywhere else in this engine
/// that means "nothing is answering", as all ones.
#[test]
fn an_unpowered_part_clocks_out_the_idle_bus_not_zero() {
    let mut powered = max7219_rig(true);
    assert_eq!(
        spi_txn(&mut powered, &[0x01, 0xFF]),
        vec![0x00, 0x00],
        "positive control: a powered frame-only part presents 0x00, exactly as \
         the deleted model did",
    );
    let mut dark = max7219_rig(false);
    assert_eq!(
        spi_txn(&mut dark, &[0x01, 0xFF]),
        vec![0xFF, 0xFF],
        "the one deliberate wire difference: Hi-Z reads as the idle bus",
    );
}

// ─── 74HC595: an SPI part that drives PADS ─────────────────────────────────

/// All eight parallel outputs bound to PA0..PA7, latch on PC13.
fn hc595_rig() -> SystemBus {
    rig(
        "74hc595",
        &[
            ("cs_pin", s("PC13")),
            ("qa_pin", s("PA0")),
            ("qb_pin", s("PA1")),
            ("qc_pin", s("PA2")),
            ("qd_pin", s("PA3")),
            ("qe_pin", s("PA4")),
            ("qf_pin", s("PA5")),
            ("qg_pin", s("PA6")),
            ("qh_pin", s("PA7")),
        ],
    )
}

/// ⚠️ THE WHOLE POINT OF THIS PART. The deleted model drove NO pad: `outputs()`
/// was a private field, so a board wired to a 74HC595 saw nothing on QA..QH
/// however correct the shift was. The level is read back out of the GPIO INPUT
/// register, after a tick, which is what `digitalRead` resolves through.
#[test]
fn a_latched_byte_reaches_the_pads() {
    let mut bus = hc595_rig();
    assert_eq!(pads(&mut bus), 0x00, "nothing driven yet");

    spi_txn(&mut bus, &[0xA5]);
    tick(&mut bus);
    assert_eq!(
        pads(&mut bus),
        0xA5,
        "QA..QH must carry the latched byte — bit 0 = QA, bit 7 = QH",
    );

    // …and the deleted model's own readback agrees about the WORD, which is the
    // half of it that was ever observable.
    let mut oracle = Hc595::new("PC13");
    oracle.transfer(0xA5);
    oracle.cs_select();
    assert_eq!(oracle.outputs(), 0xA5, "oracle control");
}

/// The NEGATIVE control for the test above: a shift with no latch must leave
/// the pads exactly where they were. Without it, "the byte reaches the pads"
/// would also pass on a build that drove the pads straight from MOSI.
#[test]
fn a_shift_without_a_latch_leaves_the_pads_alone() {
    let mut bus = hc595_rig();
    spi_txn(&mut bus, &[0x0F]);
    tick(&mut bus);
    assert_eq!(pads(&mut bus), 0x0F);

    // Shift a new value in WITHOUT completing the transaction: CS stays low, so
    // RCLK never rises.
    {
        let idx = bus.find_peripheral_index_by_name("spi1").expect("spi1");
        let any = bus.peripherals[idx].dev.as_any_mut().expect("downcastable");
        let spi = any
            .downcast_mut::<labwired_core::peripherals::spi::Spi>()
            .expect("generic Spi");
        let dev = spi.attached_devices.first_mut().expect("attached");
        dev.cs_select();
        dev.transfer(0xF0);
    }
    tick(&mut bus);
    assert_eq!(
        pads(&mut bus),
        0x0F,
        "the parallel outputs must not change before the RCLK latch",
    );
}

/// ⚠️ A NAMED DIFFERENCE: THE LATCH EDGE.
///
/// The deleted model latched in `cs_select` — CS going LOW — so the pins
/// carried the byte of the PREVIOUS transaction. A 74HC595 latches on the
/// RISING edge of RCLK, which with RCLK wired to an active-low chip select is
/// CS going HIGH. Measured here rather than left to be rediscovered: after ONE
/// transaction the descriptor shows the byte and the old model still shows
/// nothing.
#[test]
fn the_latch_edge_is_cs_rising_not_cs_falling() {
    let mut bus = hc595_rig();
    let mut oracle = Hc595::new("PC13");

    spi_txn(&mut bus, &[0x3C]);
    oracle.cs_select();
    oracle.transfer(0x3C);
    oracle.cs_release();
    tick(&mut bus);

    assert_eq!(pads(&mut bus), 0x3C, "the descriptor latches on CS↑");
    assert_eq!(
        oracle.outputs(),
        0x00,
        "the deleted model had not latched anything yet — a value written once \
         and never rewritten never appeared on its pins at all",
    );

    // A second transaction is where the old model catches up, one behind.
    spi_txn(&mut bus, &[0x5A]);
    oracle.cs_select();
    oracle.transfer(0x5A);
    oracle.cs_release();
    tick(&mut bus);
    assert_eq!(pads(&mut bus), 0x5A);
    assert_eq!(oracle.outputs(), 0x3C, "exactly one transaction behind");
}

/// Bit order, against the deleted model: SPI is MSB-first, so the first bit
/// clocked in ends at QH and the last at QA.
#[test]
fn bit_order_matches_the_deleted_model() {
    for byte in [0x01u8, 0x80, 0x55, 0xFF, 0x00] {
        let mut bus = hc595_rig();
        spi_txn(&mut bus, &[byte]);
        tick(&mut bus);
        let mut oracle = Hc595::new("PC13");
        oracle.transfer(byte);
        oracle.cs_select();
        assert_eq!(
            pads(&mut bus),
            oracle.outputs(),
            "QA..QH for {byte:#04x} must match the deleted model's word",
        );
    }
}

/// ⚠️ AN `outputs:` ROLE THE PLACEMENT DID NOT WIRE IS SKIPPED, NOT AN ERROR.
/// A board using this part with QD unconnected is an ordinary board — and it is
/// also every placement written before this descriptor existed, which set
/// `cs_pin` and nothing else.
#[test]
fn an_unwired_output_role_is_skipped_rather_than_refused() {
    let mut bus = rig(
        "74hc595",
        &[
            ("cs_pin", s("PC13")),
            ("qa_pin", s("PA0")),
            ("qh_pin", s("PA7")),
        ],
    );
    spi_txn(&mut bus, &[0xFF]);
    tick(&mut bus);
    assert_eq!(
        pads(&mut bus),
        0x81,
        "only the two wired pads move; the six unwired roles go nowhere",
    );

    // The pre-existing shape: signals only.
    let mut bare = rig("74hc595", &[("cs_pin", s("PC13"))]);
    spi_txn(&mut bare, &[0xFF]);
    tick(&mut bare);
    assert_eq!(
        pads(&mut bare),
        0x00,
        "nothing bound, nothing driven, no error"
    );
}

/// The pin queue carries TRANSITIONS, so re-latching the same word costs no bus
/// write — which is what makes a multiplexing loop cheap. Observable as: the
/// pads hold their level across a repeat.
#[test]
fn relatching_the_same_word_holds_the_pads() {
    let mut bus = hc595_rig();
    for _ in 0..5 {
        spi_txn(&mut bus, &[0x99]);
        tick(&mut bus);
        assert_eq!(pads(&mut bus), 0x99);
    }
}

// ─── the dual-74HC595 4-digit module ───────────────────────────────────────

fn hc595_7seg_rig(powered: bool) -> SystemBus {
    if powered {
        rig("hc595-7seg", &[("cs_pin", s("PC13"))])
    } else {
        rig(
            "hc595-7seg",
            &[
                ("cs_pin", s("PC13")),
                ("powered", serde_yaml::Value::from(false)),
            ],
        )
    }
}

/// Shift `segments` then a one-hot digit select, the way the module's driver
/// multiplexes one digit — into both implementations.
fn seg_write(bus: &mut SystemBus, oracle: &mut Hc5957Seg, segments: u8, select: u8) {
    spi_txn(bus, &[segments, select]);
    oracle.cs_select();
    oracle.transfer(segments);
    oracle.transfer(select);
    oracle.cs_release();
}

/// The ordinary case, both implementations: four digits, active-high select.
#[test]
fn four_multiplexed_digits_match_the_deleted_model() {
    let mut bus = hc595_7seg_rig(true);
    let mut oracle = Hc5957Seg::new("PC13");
    for (i, seg) in [0x06u8, 0x5B, 0x4F, 0x66].iter().enumerate() {
        seg_write(&mut bus, &mut oracle, *seg, 1 << i);
    }
    assert_eq!(oracle.text(), "1234", "oracle control");
    let art = inspect_artifact(bus);
    assert_eq!(art.kind, "text_display");
    assert_eq!(art.meta["format"], "hc595_7seg_digits");
    assert_eq!(
        art.meta["text"], "1234",
        "the descriptor must decode the same four characters the deleted model did",
    );
    assert_eq!(art.meta["lit_segments"], 2 + 5 + 5 + 4);
}

/// Polarity is DATA, not a search: the four common lines may be driven
/// active-low, and the same digit is selected.
#[test]
fn an_active_low_digit_select_matches_the_deleted_model() {
    let mut bus = hc595_7seg_rig(true);
    let mut oracle = Hc5957Seg::new("PC13");
    seg_write(&mut bus, &mut oracle, 0x3F, !(1u8 << 2)); // '0' on digit 2, active low
    assert_eq!(oracle.chars()[2], '0', "oracle control");
    let art = inspect_artifact(bus);
    assert_eq!(art.meta["text"], "  0 ");
}

/// The decimal point is bit 7 of a digit's segment byte, on both.
#[test]
fn the_decimal_point_bit_matches_the_deleted_model() {
    let mut bus = hc595_7seg_rig(true);
    let mut oracle = Hc5957Seg::new("PC13");
    seg_write(&mut bus, &mut oracle, 0x3F | 0x80, 1 << 1);
    assert!(oracle.decimal_point(1), "oracle control");
    let art = inspect_artifact(bus);
    assert_eq!(art.meta["text"], " 0  ", "the dp does not change the glyph");
    assert_eq!(art.meta["decimal_points"], 0b0010);
}

/// An unrecognised segment pattern renders as `?` on both, so a mis-driven
/// panel is visible rather than silently blank.
#[test]
fn an_unknown_pattern_renders_as_a_question_mark_on_both() {
    let mut bus = hc595_7seg_rig(true);
    let mut oracle = Hc5957Seg::new("PC13");
    seg_write(&mut bus, &mut oracle, 0x2A, 1);
    assert_eq!(oracle.chars()[0], '?', "oracle control");
    assert_eq!(inspect_artifact(bus).meta["text"], "?   ");
}

/// ⚠️ A NAMED DIFFERENCE: THE BYTE-ORDER SEARCH IS GONE.
///
/// The deleted model tested byte 1 for a one-hot low nibble and FELL BACK to
/// byte 0 — a search over candidates, not a statement of how the board is
/// wired. The chain order of two 74HC595s is fixed by the traces, and this
/// descriptor states it: byte 0 is segments, byte 1 selects.
///
/// Measured both ways, so the change is a fact rather than a claim.
#[test]
fn the_reversed_byte_order_is_no_longer_auto_detected() {
    let mut bus = hc595_7seg_rig(true);
    let mut oracle = Hc5957Seg::new("PC13");
    // Digit-select byte FIRST, segments second — the "reversed chain".
    seg_write(&mut bus, &mut oracle, 1 << 2, 0x3F);
    assert_eq!(
        oracle.chars()[2],
        '0',
        "the deleted model searched and found it",
    );
    let art = inspect_artifact(bus);
    assert_eq!(
        art.meta["text"], "    ",
        "the descriptor does not search: 0x3F selects no common line, so the \
         frame lit nothing — which is what the module does",
    );
}

/// ⚠️ …AND WHAT THE SEARCH COST. Any segment byte that is one-hot in the low
/// nibble is indistinguishable from a digit select, so a frame whose SELECT
/// byte deselects everything resolves the wrong way round: the deleted model
/// reads the segment byte as the digit and writes the select byte in as
/// segments. No standard glyph is such a byte, which is why the search
/// survived; a firmware lighting one segment deliberately is not glyphs.
#[test]
fn a_one_hot_segment_byte_no_longer_resolves_the_frame_backwards() {
    let mut bus = hc595_7seg_rig(true);
    let mut oracle = Hc5957Seg::new("PC13");
    // Light segment A alone on a digit, with every common line deselected.
    seg_write(&mut bus, &mut oracle, 0x01, 0x00);
    assert_eq!(
        oracle.segment_byte(0),
        0x00,
        "the deleted model resolved this backwards: it took the SEGMENT byte as \
         the digit select and latched the select byte as segments",
    );
    // …and the wiring it IS meant for still works, in the same rig, so this
    // cannot pass on a build that latches nothing at all.
    seg_write(&mut bus, &mut oracle, 0x01, 0x02);
    let art = inspect_artifact(bus);
    assert_eq!(
        art.meta["lit_segments"], 1,
        "segment A alone on digit 1 — and NOTHING from the backwards frame \
         before it, which selected no common line",
    );
}

/// ⚠️ NEW EVIDENCE, NOT PORTED EVIDENCE. The deleted model had `text()` and NO
/// `artifacts` impl at all, so the module simulated and inspected as nothing.
#[test]
fn the_module_publishes_evidence_the_deleted_model_never_did() {
    let mut bus = hc595_7seg_rig(true);
    spi_txn(&mut bus, &[0x3F, 0x01]);
    let art = inspect_artifact(bus);
    assert_eq!(art.id, "panel");
    assert_eq!(art.kind, "text_display");
    assert!(art.meta.get("text").is_some());
}

/// The supply gate, against the deleted model's own.
#[test]
fn an_unpowered_module_shows_nothing_and_says_why() {
    let mut powered = hc595_7seg_rig(true);
    spi_txn(&mut powered, &[0x3F, 0x01]);
    spi_txn(&mut powered, &[0x06, 0x02]);
    let art = inspect_artifact(powered);
    assert_eq!(art.meta["text"], "01  ", "positive control");
    assert_eq!(art.meta["powered"], true);

    let mut dark = hc595_7seg_rig(false);
    let mut oracle = Hc5957Seg::new("PC13").with_powered(false);
    for (seg, sel) in [(0x3Fu8, 0x01u8), (0x06, 0x02)] {
        seg_write(&mut dark, &mut oracle, seg, sel);
    }
    assert_eq!(oracle.text(), "    ", "oracle control");
    let art = inspect_artifact(dark);
    assert_eq!(art.meta["text"], "    ", "no supply, no light");
    assert_eq!(
        art.meta["lit_segments"], 0,
        "and no latched segments either"
    );
    assert_eq!(art.meta["powered"], false, "the artifact must say why");
}
