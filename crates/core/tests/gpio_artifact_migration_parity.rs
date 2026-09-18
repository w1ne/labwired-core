// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **TM1637 and the bare 7-segment digit: the descriptors against the models
//! they replace, on a real bus, driven by real GPIO stores.**
//!
//! `components/tm1637_7seg.rs` and `components/seven_segment.rs` are DELETED.
//! Both are kept below as ORACLES — the decode half copied verbatim, including
//! the artifact each published — and every test drives the SAME stimulus
//! through both implementations and compares every artifact byte and every
//! `meta` field.
//!
//! ## What is actually under test
//!
//! Not "a rule list can shift bits". A unit test over the rules would show that
//! and would pass with the part wired to nothing. Three claims:
//!
//! 1. **A `gpio_device` PUBLISHES AN ARTIFACT.** Until the `artifact:` key
//!    existed a ported display would simulate perfectly and inspect as NOTHING
//!    — no text, no panel, no evidence — which is what #1176 named as the thing
//!    blocking both of these ports. Every assertion here reads the artifact out
//!    of `Machine::inspect`, never out of the device.
//! 2. **A store that moves BOTH lines decodes as one event.** The engine
//!    resamples every observed pad and installs the whole snapshot before
//!    raising anything. [`a_bsrr_store_moving_both_lines_raises_no_phantom_edge`]
//!    is the measurement, with the decomposed decode spelled out as the thing
//!    that must not happen.
//! 3. **Byte parity with the deleted models**, on the full protocol, not on a
//!    sample of it.
//!
//! ## The stimulus is MMIO, never a method call
//!
//! Every CLK/DIO transition and every segment pattern below is a store to a
//! GPIO register through the `Bus` trait — the same path CPU MMIO takes — with
//! no `advance` between stores. A bit-bang loop is many stores inside one
//! peripheral tick, so a part serviced only on the tick would see one edge per
//! interval or none; [`the_frame_decodes_with_no_tick_at_all`] is the negative
//! control for that.

use labwired_config::{ChipDescriptor, ExternalDevice, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::inspect::{Artifact, DeviceInspect, InspectOpts};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::{Bus, Machine};
use std::collections::HashMap;
use std::path::PathBuf;

// ─── the oracles: the deleted models, verbatim ─────────────────────────────

/// The shared 7-segment font, copied verbatim from
/// `components/seven_seg_font.rs`.
///
/// ⚠️ COPIED, NOT IMPORTED, and that is the whole point of an oracle. Importing
/// the engine's font would make every `text` assertion below compare the
/// engine against itself: a corrupted table would change both sides equally and
/// every test would stay green.
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

const GRIDS: usize = 6;

/// `components/tm1637_7seg.rs`'s decode half, verbatim.
struct Tm1637Oracle {
    prev_clk: bool,
    prev_dio: bool,
    in_transaction: bool,
    expecting_ack: bool,
    bit_buf: u8,
    bit_count: u8,
    txn_command: Option<u8>,
    auto_increment: bool,
    addr_pointer: u8,
    display_on: bool,
    brightness: u8,
    grids: [u8; GRIDS],
}

impl Tm1637Oracle {
    fn new() -> Self {
        Self {
            prev_clk: true,
            prev_dio: true,
            in_transaction: false,
            expecting_ack: false,
            bit_buf: 0,
            bit_count: 0,
            txn_command: None,
            auto_increment: true,
            addr_pointer: 0,
            display_on: false,
            brightness: 0,
            grids: [0; GRIDS],
        }
    }

    fn observe_lines(&mut self, clk: bool, dio: bool) {
        let clk_rose = !self.prev_clk && clk;
        let clk_steady_high = self.prev_clk && clk;

        if clk_steady_high && dio != self.prev_dio {
            if self.prev_dio && !dio {
                self.begin_transaction();
            } else {
                self.in_transaction = false;
            }
        } else if clk_rose && self.in_transaction {
            if self.expecting_ack {
                self.expecting_ack = false;
                self.bit_buf = 0;
                self.bit_count = 0;
            } else {
                if dio {
                    self.bit_buf |= 1 << self.bit_count;
                }
                self.bit_count += 1;
                if self.bit_count == 8 {
                    let byte = self.bit_buf;
                    self.handle_byte(byte);
                    self.expecting_ack = true;
                }
            }
        }

        self.prev_clk = clk;
        self.prev_dio = dio;
    }

    fn begin_transaction(&mut self) {
        self.in_transaction = true;
        self.expecting_ack = false;
        self.bit_buf = 0;
        self.bit_count = 0;
        self.txn_command = None;
    }

    fn handle_byte(&mut self, byte: u8) {
        match self.txn_command {
            None => {
                self.txn_command = Some(byte);
                match byte & 0xC0 {
                    0x40 => self.auto_increment = byte & 0x04 == 0,
                    0xC0 => self.addr_pointer = byte & 0x07,
                    0x80 => {
                        self.display_on = byte & 0x08 != 0;
                        self.brightness = byte & 0x07;
                    }
                    _ => {}
                }
            }
            Some(_) => {
                let slot = (self.addr_pointer as usize) % GRIDS;
                self.grids[slot] = byte;
                if self.auto_increment {
                    self.addr_pointer = self.addr_pointer.wrapping_add(1) % GRIDS as u8;
                }
            }
        }
    }

    fn colon(&self) -> bool {
        self.grids[1] & 0x80 != 0
    }

    fn text(&self) -> String {
        (0..4).map(|i| oracle_font::decode(self.grids[i])).collect()
    }

    /// The artifact the deleted model published, field for field.
    fn artifact(&self) -> serde_json::Value {
        let grids = [self.grids[0], self.grids[1], self.grids[2], self.grids[3]];
        serde_json::json!({
            "format": "tm1637_grid",
            "generation": labwired_core::inspect::artifact_generation(&grids),
            "text": self.text(),
            "lit_segments": grids.iter().map(|g| g.count_ones() as usize).sum::<usize>(),
            "display_on": self.display_on,
            "brightness": self.brightness,
            "colon": self.colon(),
        })
    }
}

const SEGMENTS: usize = 8;

/// `components/seven_segment.rs`'s decode half, verbatim.
struct SevenSegmentOracle {
    lit: u8,
}

impl SevenSegmentOracle {
    fn new() -> Self {
        Self { lit: 0 }
    }

    fn observe_levels(&mut self, seg_levels: [bool; SEGMENTS], com: bool) {
        let mut mask = 0u8;
        for (i, level) in seg_levels.iter().enumerate() {
            let lit = if com { !*level } else { *level };
            if lit {
                mask |= 1 << i;
            }
        }
        self.lit = mask;
    }

    fn artifact(&self) -> serde_json::Value {
        serde_json::json!({
            "format": "seven_segment_mask",
            "generation": labwired_core::inspect::artifact_generation(&[self.lit]),
            "text": oracle_font::decode(self.lit).to_string(),
            "segments": self.lit,
            "lit_segments": self.lit.count_ones(),
            "decimal_point": self.lit & 0x80 != 0,
        })
    }
}

// ─── the rig ───────────────────────────────────────────────────────────────

fn repo(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// STM32F103 GPIOA: ODR at +0x0C, BSRR at +0x10. The same chip
/// `panel_artifact_evidence` uses for its one-device rigs.
const GPIOA: u64 = 0x4001_0800;
const BSRR: u64 = GPIOA + 0x10;
/// RCC APB2ENR — bit 2 ungates GPIOA. Without it the port's register file is
/// dead and every test below would measure the clock gate.
const RCC_APB2ENR: u64 = 0x4002_1018;

fn rig(device_type: &str, config: &[(&str, &str)]) -> SystemBus {
    let chip_path = repo("configs/chips/stm32f103.yaml");
    let chip = ChipDescriptor::from_file(&chip_path).expect("load chip descriptor");
    let mut cfg = HashMap::new();
    for (k, v) in config {
        cfg.insert(k.to_string(), serde_yaml::Value::from(*v));
    }
    let manifest = SystemManifest {
        cosim_models: Vec::new(),
        motor_models: Vec::new(),
        walk_deleted: Some(false),
        schema_version: "1.0".to_string(),
        name: "segment-rig".to_string(),
        chip: chip_path.to_string_lossy().to_string(),
        cpu_hz: None,
        external_devices: vec![ExternalDevice {
            id: "panel".to_string(),
            r#type: device_type.to_string(),
            connection: "gpio".to_string(),
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
    bus.write_u32(RCC_APB2ENR, 0xFFFF).expect("ungate GPIOA");
    bus
}

/// One BSRR store. `set` bits go high, `clear` bits go low — in ONE write, the
/// way `HAL_GPIO_WritePin` and every bit-bang loop in the wild do it.
fn bsrr(bus: &mut SystemBus, set: u32, clear: u32) {
    bus.write_u32(BSRR, set | (clear << 16))
        .expect("BSRR store");
}

/// The panel's one artifact, read out of `Machine::inspect` — never off the
/// device. Consumes the bus, so it is the last thing a test does.
fn inspect_artifact(mut bus: SystemBus) -> Artifact {
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    bus.refresh_peripheral_index();
    let machine = Machine::new(cpu, bus);
    let device: DeviceInspect = machine
        .inspect(None, &InspectOpts::default())
        .devices
        .into_iter()
        .find(|d| d.id == "panel")
        .expect("the declared panel is a device");
    device.artifacts.into_iter().next().unwrap_or_else(|| {
        panic!(
            "the panel produced NO artifact. A ported display that simulates \
             perfectly and inspects as nothing is exactly the state #1176 named \
             as blocking this port: the display oracle has nothing to resolve \
             against, and the browser paints an empty panel."
        )
    })
}

/// Compare an artifact against the oracle's, key by key, naming the first
/// disagreement. `assert_eq!` on two `Value`s names neither.
fn assert_same_artifact(got: &Artifact, want: &serde_json::Value, kind: &str, what: &str) {
    assert_eq!(got.kind, kind, "{what}: artifact kind");
    assert_eq!(got.id, "panel", "{what}: artifact id");
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
    // Every key, both ways: a descriptor that publishes MORE than the model did
    // is also a change, and one the browser would show.
    let mut extra: Vec<&String> = have.keys().filter(|k| !want.contains_key(*k)).collect();
    extra.sort();
    assert!(
        extra.is_empty(),
        "{what}: the descriptor publishes meta keys the deleted model did not: {extra:?}"
    );
}

// ─── TM1637 ────────────────────────────────────────────────────────────────

const CLK: u8 = 8; // PA8
const DIO: u8 = 9; // PA9

/// A TM1637 stimulus script, replayed identically into the bus and the oracle.
///
/// Each step is the `(clk, dio)` level pair the firmware puts on the pads with
/// ONE store — which is exactly what the oracle's `observe_lines` was handed,
/// so there is nothing to translate between the two sides.
struct Tm1637Script(Vec<(bool, bool)>);

impl Tm1637Script {
    fn new() -> Self {
        // The bus is idle high before anything happens. Both sides start there:
        // the oracle by construction, the descriptor by its first store.
        Self(vec![(true, true)])
    }
    fn set(&mut self, clk: bool, dio: bool) {
        self.0.push((clk, dio));
    }
    fn start(&mut self) {
        self.set(true, true);
        self.set(true, false); // DIO ↓ while CLK high
        self.set(false, false);
    }
    fn stop(&mut self) {
        self.set(false, false);
        self.set(true, false);
        self.set(true, true); // DIO ↑ while CLK high
    }
    fn byte(&mut self, b: u8) {
        for i in 0..8 {
            let bit = (b >> i) & 1 != 0;
            self.set(false, bit);
            self.set(true, bit);
        }
        self.set(false, true); // the ACK clock
        self.set(true, true);
        self.set(false, true);
    }

    /// Replay into a bus through BSRR, one store per step.
    fn replay(&self, bus: &mut SystemBus) {
        for &(clk, dio) in &self.0 {
            let mut set = 0u32;
            let mut clear = 0u32;
            if clk {
                set |= 1 << CLK;
            } else {
                clear |= 1 << CLK;
            }
            if dio {
                set |= 1 << DIO;
            } else {
                clear |= 1 << DIO;
            }
            bsrr(bus, set, clear);
        }
    }

    /// Replay into the oracle.
    fn oracle(&self) -> Tm1637Oracle {
        let mut o = Tm1637Oracle::new();
        for &(clk, dio) in &self.0 {
            o.observe_lines(clk, dio);
        }
        o
    }
}

fn tm1637_rig() -> SystemBus {
    rig("tm1637-7seg", &[("clk_pin", "PA8"), ("dio_pin", "PA9")])
}

/// The full write sequence a TM1637 driver performs: data command, address
/// command plus four grids, display control. Every byte, every ACK clock, every
/// framing edge — compared field for field against the deleted model.
#[test]
fn a_full_write_sequence_matches_the_deleted_model_field_for_field() {
    let mut s = Tm1637Script::new();
    s.start();
    s.byte(0x40); // data command, auto-increment
    s.stop();
    s.start();
    s.byte(0xC0); // address command, GRID 0
    s.byte(0x06); // '1'
    s.byte(0x5B); // '2'
    s.byte(0x4F); // '3'
    s.byte(0x66); // '4'
    s.stop();
    s.start();
    s.byte(0x8F); // display on, brightness 7
    s.stop();

    let oracle = s.oracle();
    assert_eq!(
        oracle.text(),
        "1234",
        "the oracle itself decoded the script"
    );
    assert!(oracle.display_on);
    assert_eq!(oracle.brightness, 7);

    let mut bus = tm1637_rig();
    s.replay(&mut bus);
    let got = inspect_artifact(bus);
    assert_same_artifact(&got, &oracle.artifact(), "text_display", "full write");
    // Stated rather than left to the oracle comparison, so a corrupted oracle
    // cannot make this pass by agreeing with a corrupted descriptor.
    assert_eq!(got.meta["text"], "1234");
    assert_eq!(got.meta["brightness"], 7);
    assert_eq!(got.meta["display_on"], serde_json::Value::Bool(true));
}

/// Fixed-address mode (0x44) writes ONE grid and does not advance the pointer.
/// The pointer is persistent state across transactions, so this is the case a
/// stateless decode would get wrong.
#[test]
fn fixed_address_mode_matches_the_deleted_model() {
    let mut s = Tm1637Script::new();
    s.start();
    s.byte(0x44); // fixed address
    s.stop();
    s.start();
    s.byte(0xC2); // GRID 2
    s.byte(0x3F); // '0'
    s.byte(0x3F); // …and again: fixed address means it lands in GRID 2 twice
    s.stop();

    let oracle = s.oracle();
    assert_eq!(oracle.text(), "  0 ", "GRID 2 only");

    let mut bus = tm1637_rig();
    s.replay(&mut bus);
    assert_same_artifact(
        &inspect_artifact(bus),
        &oracle.artifact(),
        "text_display",
        "fixed address",
    );
}

/// The colon on the clock module is the decimal point of GRID 1, and it is a
/// `meta` field of its own because the browser draws it separately.
#[test]
fn the_colon_bit_matches_the_deleted_model() {
    let mut s = Tm1637Script::new();
    s.start();
    s.byte(0x40);
    s.stop();
    s.start();
    s.byte(0xC0);
    s.byte(0x06); // '1'
    s.byte(0x5B | 0x80); // '2' + colon
    s.stop();

    let oracle = s.oracle();
    assert!(oracle.colon());

    let mut bus = tm1637_rig();
    s.replay(&mut bus);
    let got = inspect_artifact(bus);
    assert_same_artifact(&got, &oracle.artifact(), "text_display", "colon");
    assert_eq!(got.meta["colon"], serde_json::Value::Bool(true));
}

/// The pointer wraps modulo SIX — the part has six GRID registers — while the
/// four-digit module wires four. A write past GRID 3 must land, must not
/// corrupt a shown digit, and must NOT move `generation`.
#[test]
fn the_two_unwired_grids_are_kept_but_not_shown() {
    let mut s = Tm1637Script::new();
    s.start();
    s.byte(0x40);
    s.stop();
    s.start();
    s.byte(0xC4); // GRID 4 — past the four the module wires
    s.byte(0x7F); // '8', all segments
    s.stop();

    let oracle = s.oracle();
    assert_eq!(oracle.grids[4], 0x7F, "the oracle latched GRID 4");
    assert_eq!(oracle.text(), "    ", "and shows nothing for it");

    let mut bus = tm1637_rig();
    s.replay(&mut bus);
    let got = inspect_artifact(bus);
    assert_same_artifact(&got, &oracle.artifact(), "text_display", "grid 4");
    assert_eq!(
        got.meta["lit_segments"], 0,
        "a byte in an unwired GRID lights nothing the module has"
    );
}

/// ⚠️ **THE SIMULTANEOUS-PAD MEASUREMENT.**
///
/// A store that moves BOTH lines at once. Decomposed into two per-pad edges —
/// which is what `on: { pin: X }` delivers, in `observed` order — the DIO
/// falling edge is raised while CLK still reads its OLD level, so this store
/// would decode as a START. It is not one: CLK fell in the same instruction,
/// and a TM1637 sees one transition of the pair, not two.
///
/// The oracle is the authority on what the answer is, because it was handed
/// both levels together. It says: no transaction opened.
#[test]
fn a_bsrr_store_moving_both_lines_raises_no_phantom_edge() {
    let mut s = Tm1637Script::new();
    // Idle high, then ONE store that clears CLK and DIO together.
    s.set(false, false);
    // …and then a complete, legitimate write, so the test also proves the part
    // is not simply inert after the ambiguous store.
    s.start();
    s.byte(0x40);
    s.stop();
    s.start();
    s.byte(0xC0);
    s.byte(0x6D); // '5'
    s.stop();

    let oracle = s.oracle();
    assert!(
        !oracle.in_transaction,
        "the oracle closed the transaction at the STOP"
    );
    assert_eq!(oracle.text(), "5   ");

    let mut bus = tm1637_rig();
    s.replay(&mut bus);
    let got = inspect_artifact(bus);
    assert_same_artifact(&got, &oracle.artifact(), "text_display", "both lines");
    assert_eq!(
        got.meta["text"], "5   ",
        "a phantom START from the both-lines store would have opened a \
         transaction that swallowed the data command, and the digit would be blank"
    );
}

/// The negative control for the phantom edge: the ambiguous store ALONE, with
/// nothing after it, must leave the panel exactly as it was.
///
/// Without this, the test above could pass on a build that did open a phantom
/// transaction and then recovered at the next real START.
#[test]
fn the_both_lines_store_alone_changes_nothing() {
    let mut s = Tm1637Script::new();
    s.set(false, false);
    let oracle = s.oracle();
    assert!(!oracle.in_transaction, "no transaction opened");

    let mut bus = tm1637_rig();
    s.replay(&mut bus);
    let got = inspect_artifact(bus);
    assert_same_artifact(&got, &oracle.artifact(), "text_display", "ambiguous alone");
    assert_eq!(got.meta["text"], "    ");
    assert_eq!(got.meta["lit_segments"], 0);
}

/// The whole frame is delivered with the machine never advanced a single cycle.
///
/// This is the claim `edge_service_addrs` exists for. A part serviced only on
/// the peripheral tick would sample the pads after the entire bit-bang loop and
/// see one level, not ninety-odd edges — so a green decode above would be
/// measuring the tick pass rather than the write path.
#[test]
fn the_frame_decodes_with_no_tick_at_all() {
    let mut s = Tm1637Script::new();
    s.start();
    s.byte(0x40);
    s.stop();
    s.start();
    s.byte(0xC0);
    s.byte(0x77); // 'A'
    s.stop();
    assert!(
        s.0.len() > 40,
        "the script is {} stores — a tick pass could not deliver them",
        s.0.len()
    );

    let mut bus = tm1637_rig();
    s.replay(&mut bus);
    // No `Machine::advance` anywhere: `inspect_artifact` builds the machine and
    // reads it without running a cycle.
    assert_eq!(inspect_artifact(bus).meta["text"], "A   ");
}

// ─── the bare 7-segment digit ──────────────────────────────────────────────

/// Segments A..DP on PA0..PA7, COM on PA8 — all one port, which is the wiring
/// the deleted model's own bus test used and the one that exercises the
/// dedupe in `edge_service_addrs`.
fn seven_segment_rig() -> SystemBus {
    rig(
        "seven-segment",
        &[
            ("a_pin", "PA0"),
            ("b_pin", "PA1"),
            ("c_pin", "PA2"),
            ("d_pin", "PA3"),
            ("e_pin", "PA4"),
            ("f_pin", "PA5"),
            ("g_pin", "PA6"),
            ("dp_pin", "PA7"),
            ("com_pin", "PA8"),
        ],
    )
}

/// Drive the nine pads with ONE BSRR store — which is how firmware drives a
/// digit, and the case a per-pad recomputation gets wrong eight times out of
/// nine.
fn drive_digit(bus: &mut SystemBus, segs: u8, com: bool) {
    let mut set = 0u32;
    let mut clear = 0u32;
    for i in 0..8u8 {
        if (segs >> i) & 1 != 0 {
            set |= 1 << i;
        } else {
            clear |= 1 << i;
        }
    }
    if com {
        set |= 1 << 8;
    } else {
        clear |= 1 << 8;
    }
    bsrr(bus, set, clear);
}

fn seven_segment_oracle(steps: &[(u8, bool)]) -> SevenSegmentOracle {
    let mut o = SevenSegmentOracle::new();
    for &(segs, com) in steps {
        o.observe_levels(std::array::from_fn(|i| (segs >> i) & 1 != 0), com);
    }
    o
}

/// Common cathode: COM low, a segment lights when its pin is driven HIGH.
#[test]
fn common_cathode_matches_the_deleted_model() {
    let steps = [(0x3Fu8, false)];
    let oracle = seven_segment_oracle(&steps);
    assert_eq!(oracle.lit, 0x3F);

    let mut bus = seven_segment_rig();
    for &(segs, com) in &steps {
        drive_digit(&mut bus, segs, com);
    }
    let got = inspect_artifact(bus);
    assert_same_artifact(&got, &oracle.artifact(), "text_display", "common cathode");
    assert_eq!(got.meta["text"], "0");
    assert_eq!(got.meta["segments"], 0x3F);
    assert_eq!(got.meta["lit_segments"], 6);
}

/// Common anode: COM high, the SAME two segments lit by the INVERTED drive.
/// The polarity fold is what makes both report the same glyph, and getting it
/// backwards is the failure that lights every segment of a blank digit.
#[test]
fn common_anode_matches_the_deleted_model() {
    let steps = [(!0x06u8, true)];
    let oracle = seven_segment_oracle(&steps);
    assert_eq!(oracle.lit, 0x06, "'1' through an inverted drive");

    let mut bus = seven_segment_rig();
    for &(segs, com) in &steps {
        drive_digit(&mut bus, segs, com);
    }
    let got = inspect_artifact(bus);
    assert_same_artifact(&got, &oracle.artifact(), "text_display", "common anode");
    assert_eq!(got.meta["text"], "1");
}

/// The decimal point is its own `meta` field and must NOT change the glyph —
/// the font masks bit 7 before matching.
#[test]
fn the_decimal_point_matches_the_deleted_model() {
    let steps = [(0x3Fu8 | 0x80, false)];
    let oracle = seven_segment_oracle(&steps);

    let mut bus = seven_segment_rig();
    for &(segs, com) in &steps {
        drive_digit(&mut bus, segs, com);
    }
    let got = inspect_artifact(bus);
    assert_same_artifact(&got, &oracle.artifact(), "text_display", "decimal point");
    assert_eq!(got.meta["decimal_point"], serde_json::Value::Bool(true));
    assert_eq!(
        got.meta["text"], "0",
        "dp is masked off before the glyph match"
    );
}

/// Sampling is COMBINATIONAL: the new levels win outright, with no latching and
/// no history. Three patterns in a row, each read against the oracle's answer
/// for the same three.
#[test]
fn sampling_is_combinational_across_stores() {
    for steps in [
        vec![(0x3Fu8, false)],
        vec![(0x3Fu8, false), (0x7F, false)],
        vec![(0x3Fu8, false), (0x7F, false), (0x06, false)],
    ] {
        let oracle = seven_segment_oracle(&steps);
        let mut bus = seven_segment_rig();
        for &(segs, com) in &steps {
            drive_digit(&mut bus, segs, com);
        }
        assert_same_artifact(
            &inspect_artifact(bus),
            &oracle.artifact(),
            "text_display",
            "combinational",
        );
    }
}

/// A blank digit reads blank in BOTH wirings, and reports zero rather than
/// absence — the contract `panel_artifact_evidence` states.
#[test]
fn a_blank_digit_reports_zero_in_both_wirings() {
    for (segs, com) in [(0x00u8, false), (0xFFu8, true)] {
        let oracle = seven_segment_oracle(&[(segs, com)]);
        assert_eq!(oracle.lit, 0);
        let mut bus = seven_segment_rig();
        drive_digit(&mut bus, segs, com);
        let got = inspect_artifact(bus);
        assert_same_artifact(&got, &oracle.artifact(), "text_display", "blank");
        assert_eq!(got.meta["text"], " ");
        assert_eq!(got.meta["lit_segments"], 0);
    }
}

/// ⚠️ **THE NINE-PAD SIMULTANEITY MEASUREMENT.**
///
/// Nine pads move in one store, and the mask must be computed from the levels
/// that store left — all nine of them. Decomposed into nine edge events, the
/// mask would be recomputed nine times, and eight of those recomputations read
/// at least one pad at a level the store has already changed.
///
/// The case is chosen so the decomposition is VISIBLE rather than coincidental:
/// COM flips at the same time as the segments, so an order that recomputes
/// before COM has been resampled folds the OLD polarity over the NEW segments
/// and reports the complement.
#[test]
fn nine_pads_moving_in_one_store_fold_one_polarity() {
    // From '0' on a common-cathode wiring to '1' on a common-anode one, in a
    // single store.
    let steps = [(0x3Fu8, false), (!0x06u8, true)];
    let oracle = seven_segment_oracle(&steps);
    assert_eq!(oracle.lit, 0x06);

    let mut bus = seven_segment_rig();
    for &(segs, com) in &steps {
        drive_digit(&mut bus, segs, com);
    }
    let got = inspect_artifact(bus);
    assert_same_artifact(&got, &oracle.artifact(), "text_display", "nine pads");
    assert_eq!(
        got.meta["segments"], 0x06,
        "the complement (0x{:02X}) is what a stale-COM fold reports",
        !0x06u8
    );
}
