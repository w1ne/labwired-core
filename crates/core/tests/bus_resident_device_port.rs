// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! A bus-resident device is bound to three pad operations, not to the machine.
//!
//! `BusResidentDevice::service` used to take `&mut SystemBus`. Every off-chip
//! stimulus model in the tree — a push button, a 4×4 keypad, a rotary encoder,
//! a DHT22 — was therefore typed against the whole engine, while using three
//! operations at most. That is the coupling the C-1 ledger row is about: it is
//! not rebuild time, it is that a device cannot be written, moved or exercised
//! without the entire bus in scope.
//!
//! Two tests here, and they check different things:
//!
//! 1. [`a_keypad_scans_with_no_bus_in_sight`] is the *behavioural* proof. It
//!    services a real `Keypad` against a fifty-line fake port that owns nothing
//!    but a register map. This test could not have been written before the
//!    narrowing — there was no way to call `service` without constructing a
//!    `SystemBus` — so its mere existence is the measurement.
//! 2. [`resident_device_port_stays_narrow`] is the *structural* guard. The
//!    narrowing is enforced by the type system today, so no future edit can
//!    widen `service` itself without breaking the build. What CAN happen
//!    silently is a new `DevicePins` method that hands the bus (or any other
//!    engine type) back through the port — a one-line change that compiles, and
//!    that undoes all of this. That test reads the trait's own body and fails
//!    on it.

use labwired_core::bus::DevicePins;
use labwired_core::peripherals::components::keypad::{Keypad, COLS, ROWS};
use std::collections::HashMap;
use std::path::PathBuf;

/// A `DevicePins` that is not a bus: a flat `(addr, bit) -> level` map for the
/// MCU's output pins, and a log of everything the device drove back.
///
/// This is the whole world a bus-resident device gets to see. If a device needs
/// more than this, the port is wrong — and it will not compile, which is the
/// point.
#[derive(Default)]
struct FakePins {
    /// What the MCU is driving out, by output-register word address.
    out: HashMap<u64, u32>,
    /// Every `drive_idr_bit` the device performed, in order.
    idr_writes: Vec<(u64, u8, bool)>,
    /// Every `drive_input_bit` the device performed, in order.
    input_writes: Vec<(u64, u8, bool)>,
}

impl FakePins {
    fn drive_out(&mut self, addr: u64, bit: u8, high: bool) {
        let w = self.out.entry(addr).or_insert(0);
        if high {
            *w |= 1 << bit;
        } else {
            *w &= !(1 << bit);
        }
    }
}

impl DevicePins for FakePins {
    fn output_bit(&self, addr: u64, bit: u8) -> Option<bool> {
        self.out.get(&addr).map(|w| (w >> bit) & 1 != 0)
    }

    fn drive_idr_bit(&mut self, addr: u64, bit: u8, high: bool) {
        self.idr_writes.push((addr, bit, high));
    }

    fn drive_input_bit(&mut self, addr: u64, bit: u8, high: bool) -> bool {
        self.input_writes.push((addr, bit, high));
        true
    }
}

const ROW_ADDR: u64 = 0x4001_0014;
const COL_ADDR: u64 = 0x4001_0010;

fn wired_keypad() -> Keypad {
    Keypad::new(
        "pad".to_string(),
        std::array::from_fn(|r| (ROW_ADDR, r as u8)),
        std::array::from_fn(|c| (COL_ADDR, c as u8)),
    )
}

/// A real keypad, scanned end to end, with no `SystemBus` anywhere in the test.
///
/// The scan is the genuine one: the firmware drives one row LOW, the model
/// recomputes the four column levels, and the pressed key's column follows the
/// row that bridges it. Everything else stays high.
#[test]
fn a_keypad_scans_with_no_bus_in_sight() {
    // UFCS on purpose: `Keypad` also has an INHERENT `service`, which wins
    // method resolution. The trait method is the one under test.
    use labwired_core::bus::BusResidentDevice;

    let mut pins = FakePins::default();
    let mut pad = wired_keypad();

    // Idle: every row released high. First service settles all four columns
    // high (their pull-ups), because the fake IDR starts at an unknown level.
    for r in 0..ROWS {
        pins.drive_out(ROW_ADDR, r as u8, true);
    }
    BusResidentDevice::service(&mut pad, &mut pins, 0);
    assert_eq!(
        pins.idr_writes.len(),
        COLS,
        "the first pass settles every column at its idle level: {:?}",
        pins.idr_writes
    );
    assert!(
        pins.idr_writes.iter().all(|&(_, _, high)| high),
        "nothing pressed, so every column reads its pull-up high: {:?}",
        pins.idr_writes
    );

    // Press (row 2, col 1) and scan row 2 by driving it LOW.
    pad.set_pressed(Some((2, 1)));
    pins.idr_writes.clear();
    pins.drive_out(ROW_ADDR, 2, false);
    BusResidentDevice::service(&mut pad, &mut pins, 1);
    assert_eq!(
        pins.idr_writes,
        vec![(COL_ADDR, 1, false)],
        "scanning the pressed key's row pulls exactly its column low"
    );

    // Scan a row the key is NOT on: the column returns high.
    pins.idr_writes.clear();
    pins.drive_out(ROW_ADDR, 2, true);
    pins.drive_out(ROW_ADDR, 0, false);
    BusResidentDevice::service(&mut pad, &mut pins, 2);
    assert_eq!(
        pins.idr_writes,
        vec![(COL_ADDR, 1, true)],
        "row 0 does not bridge a key on row 2, so column 1 releases high"
    );

    // Idle again, nothing changes: a settled keypad costs the port no writes.
    pins.idr_writes.clear();
    BusResidentDevice::service(&mut pad, &mut pins, 3);
    assert!(
        pins.idr_writes.is_empty(),
        "a device at rest must touch nothing: {:?}",
        pins.idr_writes
    );

    // ⚠️ THIS USED TO ASSERT THE OPPOSITE, and the opposite was the bug.
    //
    // It read: "a keypad has no business on the external-level seam", requiring
    // `input_writes` to stay EMPTY. That held only because every part in view at
    // the time let a store to the input register land. On silicon whose input
    // word is READ-ONLY the store is correctly ignored and the column never
    // moves — EFR32 Series 2 (DIN @0x14), SAM PORT (IN @0x20), ESP32-C3. The
    // matrix was inert on all three, silently: attach succeeded, the stimulus
    // reported applied, no pin moved.
    //
    // So a keypad has exactly the same business on that seam as the DHT22,
    // which has always driven both. `gpio_devices_drive_read_only_inputs.rs`
    // gates the behaviour on a real EFR32 port; this asserts the port CONTRACT:
    // both halves are used, and each column change appears on each seam once.
    // `idr_writes` is cleared as the scan progresses, so the seam is checked
    // against the full drive history the scan above performed: four columns
    // settled high, then column 1 low for the press, then back high.
    assert_eq!(
        pins.input_writes,
        vec![
            (COL_ADDR, 0, true),
            (COL_ADDR, 1, true),
            (COL_ADDR, 2, true),
            (COL_ADDR, 3, true),
            (COL_ADDR, 1, false),
            (COL_ADDR, 1, true),
        ],
        "every column change must reach BOTH seams — the MMIO store for ports \
         that accept it, the external-level seam for ports whose input word is \
         read-only — and a settled keypad must still add nothing"
    );
}

/// The port must stay primitive.
///
/// Every argument and return of every `DevicePins` method has to be a scalar or
/// an `Option` of one. The moment one of them names an engine type — most
/// obviously `&mut SystemBus`, but a `Peripheral`, a `PeripheralEntry` or a
/// `dyn Bus` would do the same job — a device can reach the whole machine again
/// through a port that still looks narrow at the call site.
///
/// The build cannot catch that: adding a method to a trait is a compiling
/// change. So this reads the declaration.
#[test]
fn resident_device_port_stays_narrow() {
    /// Everything a pad operation is allowed to be spelled with.
    const ALLOWED: [&str; 9] = [
        "self", "mut", "u8", "u16", "u32", "u64", "usize", "bool", "Option",
    ];

    let src = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("bus")
            .join("resident_device.rs"),
    )
    .expect("read bus/resident_device.rs");

    let body = src
        .split_once("pub trait DevicePins {")
        .expect("DevicePins declaration — this test is measuring the wrong file")
        .1
        .split_once("\n}")
        .expect("end of the DevicePins trait")
        .0;

    // Method signatures only: doc comments describe the world, and they are
    // allowed to mention `SystemBus` (they have to — that is the whole story).
    let sigs: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("fn "))
        .collect();

    // Anti-vacuity: a parser that matched nothing would pass every assertion
    // below by measuring nothing at all.
    assert!(
        sigs.len() >= 3,
        "found {} DevicePins method signatures — the scan is broken, so this \
         test would have passed by finding no work; body was:\n{body}",
        sigs.len()
    );
    assert!(
        sigs.iter().any(|s| s.contains("drive_idr_bit")),
        "scan did not find `drive_idr_bit`, so it is not reading real \
         signatures: {sigs:?}"
    );

    for sig in &sigs {
        for tok in sig
            .split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|t| !t.is_empty())
            // Skip `fn` and the method name; only the types are constrained.
            .skip(2)
        {
            assert!(
                ALLOWED.contains(&tok) || tok.chars().next().is_some_and(|c| c.is_lowercase()),
                "`DevicePins::{sig}` names `{tok}`. The port a bus-resident \
                 device is handed must stay primitive — one engine type in it \
                 and every off-chip model is coupled to the machine again, \
                 which is exactly what narrowing `service` removed. Allowed \
                 spellings: {ALLOWED:?}"
            );
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// The two bit-banged DISPLAYS, on the same port.
//
// `SystemBus` used to carry `tm1637: Vec<Tm1637>` and
// `seven_segment: Vec<SevenSegment>` as typed fields, each driven by a bespoke
// MMIO write hook (`maybe_clock_tm1637`, `maybe_sample_seven_segment`) that
// cached GPIO peripheral indices by hand and was called from three places in
// `bus/accessors.rs`. Both hooks were one part's private copy of
// `maybe_service_edge_driven_gpio_devices`, and the typed fields were the
// reason a display could not be a descriptor: the bus had to be edited to add
// one.
//
// Both models are now `BusResidentDevice`s on `gpio_devices`. The tests below
// are the two halves of that claim — the behaviour reached through the narrow
// port, and the structural guard that the fields cannot come back.
// ───────────────────────────────────────────────────────────────────────────

use labwired_config::DeviceDescriptor;
use labwired_core::bus::BusResidentDevice;
use labwired_core::inspect::{Artifact, DeviceEvidence, InspectOpts};
use labwired_core::peripherals::components::declarative_gpio::{BoundPin, DeclarativeGpioDevice};

const CLK_ADDR: u64 = 0x4800_0014;
const CLK_BIT: u8 = 8;
const DIO_BIT: u8 = 9;

/// Build a `gpio_device` from its SHIPPED descriptor, with pads bound by hand.
///
/// ⚠️ The descriptor comes from `embedded_device_yaml`, not from a fixture
/// written here. A fixture would make every test below a test of itself: the
/// shipped YAML could lose a rule and this file would stay green.
fn from_descriptor(type_name: &str, observed: &[(&str, u64, u8)]) -> DeclarativeGpioDevice {
    let yaml = labwired_config::embedded_device_yaml(type_name)
        .unwrap_or_else(|| panic!("{type_name} is an embedded descriptor"));
    let desc = DeviceDescriptor::from_yaml(yaml).expect("the shipped descriptor parses");
    DeclarativeGpioDevice::new(
        "panel".to_string(),
        &desc,
        observed
            .iter()
            .map(|(role, addr, bit)| BoundPin {
                role: (*role).to_string(),
                addr: *addr,
                bit: *bit,
            })
            .collect(),
        Vec::new(),
        8_000_000,
        std::borrow::Cow::Borrowed(&[]),
    )
    .expect("the shipped descriptor constructs")
}

/// The device's artifact, read through the SAME evidence seam `inspect` uses.
fn artifact(dev: &DeclarativeGpioDevice) -> Artifact {
    DeviceEvidence::artifacts(dev, "panel", &InspectOpts::default())
        .into_iter()
        .next()
        .expect("a declared artifact is published")
}

/// Bit-bang one TM1637 byte, LSB first, plus the ACK clock — through the PORT,
/// servicing the model after every pad move exactly as the bus's write hook
/// does after every MMIO store.
fn tm1637_byte(dev: &mut DeclarativeGpioDevice, pins: &mut FakePins, byte: u8, now: &mut u64) {
    for i in 0..8 {
        let bit = (byte >> i) & 1 != 0;
        tm1637_lines(dev, pins, false, bit, now);
        tm1637_lines(dev, pins, true, bit, now);
    }
    tm1637_lines(dev, pins, false, true, now); // ACK clock
    tm1637_lines(dev, pins, true, true, now);
    tm1637_lines(dev, pins, false, true, now);
}

/// Put both levels on the pads with ONE port update and service once — which is
/// what one BSRR store does, and what the simultaneous-pad event exists for.
fn tm1637_lines(
    dev: &mut DeclarativeGpioDevice,
    pins: &mut FakePins,
    clk: bool,
    dio: bool,
    now: &mut u64,
) {
    pins.drive_out(CLK_ADDR, CLK_BIT, clk);
    pins.drive_out(CLK_ADDR, DIO_BIT, dio);
    *now += 1;
    BusResidentDevice::service(dev, pins, *now);
}

/// A real TM1637 decodes a real frame with no `SystemBus` anywhere in the test.
///
/// Before the resident-device port the model could only be driven through
/// `SystemBus::maybe_clock_tm1637`, so this test could not have been written:
/// its existence is the measurement. It is now a DESCRIPTOR rather than a Rust
/// model, and the port did not have to change for that — which is the second
/// measurement. The protocol is the genuine one: START, data command, STOP,
/// START, address command, two grid bytes, STOP, display control.
#[test]
fn a_tm1637_decodes_a_frame_with_no_bus_in_sight() {
    let mut pins = FakePins::default();
    let mut dev = from_descriptor(
        "tm1637-7seg",
        &[("CLK", CLK_ADDR, CLK_BIT), ("DIO", CLK_ADDR, DIO_BIT)],
    );
    let mut now = 0u64;

    tm1637_lines(&mut dev, &mut pins, true, true, &mut now); // idle
    tm1637_lines(&mut dev, &mut pins, true, false, &mut now); // START
    tm1637_lines(&mut dev, &mut pins, false, false, &mut now);
    tm1637_byte(&mut dev, &mut pins, 0x40, &mut now); // auto-increment data cmd
    tm1637_lines(&mut dev, &mut pins, true, false, &mut now);
    tm1637_lines(&mut dev, &mut pins, true, true, &mut now); // STOP

    tm1637_lines(&mut dev, &mut pins, true, false, &mut now); // START
    tm1637_lines(&mut dev, &mut pins, false, false, &mut now);
    tm1637_byte(&mut dev, &mut pins, 0xC0, &mut now); // address 0
    tm1637_byte(&mut dev, &mut pins, 0x06, &mut now); // '1'
    tm1637_byte(&mut dev, &mut pins, 0x5B, &mut now); // '2'
    tm1637_lines(&mut dev, &mut pins, true, false, &mut now);
    tm1637_lines(&mut dev, &mut pins, true, true, &mut now); // STOP

    tm1637_lines(&mut dev, &mut pins, true, false, &mut now); // START
    tm1637_lines(&mut dev, &mut pins, false, false, &mut now);
    tm1637_byte(&mut dev, &mut pins, 0x8F, &mut now); // display ON, brightness 7
    tm1637_lines(&mut dev, &mut pins, true, false, &mut now);
    tm1637_lines(&mut dev, &mut pins, true, true, &mut now); // STOP

    let a = artifact(&dev);
    assert_eq!(a.kind, "text_display");
    assert_eq!(a.meta["text"], "12  ", "decoded grids");
    assert_eq!(a.meta["display_on"], serde_json::Value::Bool(true));
    assert_eq!(a.meta["brightness"], 7);

    // A DISPLAY drives nothing. Both halves of the driving port must stay
    // untouched — a model that wrote a pad here would be inventing a level the
    // firmware never sampled.
    assert!(
        pins.idr_writes.is_empty() && pins.input_writes.is_empty(),
        "a TM1637 only observes: {:?} {:?}",
        pins.idr_writes,
        pins.input_writes
    );
}

/// The direct-drive digit is combinational: nine pads in, one mask out, no
/// history. Both COM polarities, through the port.
#[test]
fn a_seven_segment_digit_reads_nine_pads_with_no_bus_in_sight() {
    const ODR: u64 = 0x4800_0014;
    const ROLES: [&str; 8] = ["A", "B", "C", "D", "E", "F", "G", "DP"];
    let mut pins = FakePins::default();
    let mut observed: Vec<(&str, u64, u8)> = ROLES
        .iter()
        .enumerate()
        .map(|(i, r)| (*r, ODR, i as u8))
        .collect();
    observed.push(("COM", ODR, 8));
    let mut dev = from_descriptor("seven-segment", &observed);

    let show = |pins: &mut FakePins, dev: &mut DeclarativeGpioDevice, segs: u8, com: bool| {
        for i in 0..8u8 {
            pins.drive_out(ODR, i, (segs >> i) & 1 != 0);
        }
        pins.drive_out(ODR, 8, com);
        BusResidentDevice::service(dev, pins, 0);
        let a = artifact(dev);
        (
            a.meta["text"].as_str().unwrap_or("").to_string(),
            a.meta["segments"].as_i64().unwrap_or(-1),
        )
    };

    // Common cathode (COM low): a segment lights when its pin is HIGH.
    assert_eq!(show(&mut pins, &mut dev, 0x3F, false), ("0".into(), 0x3F));
    assert_eq!(show(&mut pins, &mut dev, 0x06, false), ("1".into(), 0x06));
    // Common anode (COM high): the same glyph, every pin inverted.
    assert_eq!(show(&mut pins, &mut dev, !0x06, true), ("1".into(), 0x06));

    assert!(
        pins.idr_writes.is_empty() && pins.input_writes.is_empty(),
        "a 7-segment digit only observes"
    );
}

/// Each display must name the output registers whose writes service it, and
/// must say it needs NO per-cycle pass — the two facts that together replace
/// its deleted private hook and keep its board on the walk-free fast path.
///
/// The keypad is the positive control: a scanned device DOES need the tick, and
/// names no edge address. Without it this test would pass on a build where
/// every device answered the same way. The HX711 is the SECOND positive
/// control, and a sharper one: it is a `gpio_device` from the same primitive as
/// the two displays, and it must answer `true` because it owns a timer and
/// drives a pad.
#[test]
fn the_displays_are_edge_serviced_and_the_keypad_is_not() {
    let tm = from_descriptor(
        "tm1637-7seg",
        &[("CLK", 0x4800_0014, 8), ("DIO", 0x4800_0414, 9)],
    );
    assert_eq!(
        BusResidentDevice::edge_service_addrs(&tm),
        &[0x4800_0014u64, 0x4800_0414],
        "both ODR addresses, sorted"
    );
    assert!(!BusResidentDevice::needs_per_cycle_service(&tm));

    // Two pads on ONE port dedupe to one address: the bus consults this on
    // every MMIO write, so a duplicate would be a cost paid per store.
    let one_port = from_descriptor(
        "tm1637-7seg",
        &[("CLK", 0x4800_0014, 8), ("DIO", 0x4800_0014, 9)],
    );
    assert_eq!(
        BusResidentDevice::edge_service_addrs(&one_port),
        &[0x4800_0014u64]
    );

    let mut nine: Vec<(&str, u64, u8)> = ["A", "B", "C", "D", "E", "F", "G", "DP"]
        .iter()
        .enumerate()
        .map(|(i, r)| (*r, 0x4800_0014u64, i as u8))
        .collect();
    nine.push(("COM", 0x4800_0414, 0));
    let seg = from_descriptor("seven-segment", &nine);
    assert_eq!(
        BusResidentDevice::edge_service_addrs(&seg),
        &[0x4800_0014u64, 0x4800_0414],
        "nine pads across two ports dedupe to two addresses"
    );
    assert!(!BusResidentDevice::needs_per_cycle_service(&seg));

    let pad = wired_keypad();
    assert!(
        pad.edge_service_addrs().is_empty(),
        "a scanned keypad is tick-driven; naming an edge address would service \
         it twice per store"
    );
    assert!(
        pad.needs_per_cycle_service(),
        "positive control: a device that IS scanned per tick must say so, or \
         this test would pass on a build where every device answered `false`"
    );
}

/// A `gpio_device` that OWNS A TIMER must still be serviced every cycle.
///
/// The displays answer `false` above through an expression, not an override —
/// `!timers.is_empty() || !driven.is_empty()` — so the two halves of that
/// expression need a part that exercises each. The HX711 is both: it arms a
/// power-on timer and it drives DOUT. Without this, a change that made the
/// expression a bare `false` would leave every test above green and would stop
/// the HX711's clock dead.
#[test]
fn a_gpio_device_with_a_timer_still_needs_the_tick() {
    let hx = from_descriptor("hx711", &[("SCK", 0x4800_0014, 8)]);
    assert!(
        BusResidentDevice::needs_per_cycle_service(&hx),
        "the HX711 owns a timer and drives a pad; saying `false` would stop its \
         derived clock and the part would look busy forever"
    );
}

/// A part that declares NO `artifact:` publishes NO evidence — absence, not an
/// empty panel.
///
/// The two displays above report through `BusResidentDevice::evidence`. That
/// seam is on the trait, so EVERY resident device answers it, and the honest
/// answer for a load cell is `None`. An empty-but-present artifact would make
/// `labwired_verify`'s display oracle resolve against a blank panel for a part
/// that has no panel at all.
#[test]
fn a_part_that_shows_nothing_publishes_no_artifact() {
    let hx = from_descriptor("hx711", &[("SCK", 0x4800_0014, 8)]);
    assert!(
        BusResidentDevice::evidence(&hx).is_none(),
        "an HX711 is not a display"
    );
    let tm = from_descriptor(
        "tm1637-7seg",
        &[("CLK", 0x4800_0014, 8), ("DIO", 0x4800_0014, 9)],
    );
    assert!(
        BusResidentDevice::evidence(&tm).is_some(),
        "anti-vacuity: a part that DOES declare an artifact must answer Some, \
         or the assertion above passes on a build where evidence is never wired"
    );
}

/// `SystemBus` must carry NO typed display field, and neither bespoke write
/// hook may come back.
///
/// The type system cannot catch this: adding `pub tm1637: Vec<Tm1637>` back to
/// the struct and a `self.maybe_clock_tm1637(idx)` line to `accessors.rs`
/// compiles and passes every behavioural test in the tree — the display would
/// work, through plumbing that exists for one part. That is exactly how the
/// field got there the first time, so the guard is a source read.
#[test]
fn no_typed_display_field_on_the_bus() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("bus");
    let mod_rs = std::fs::read_to_string(dir.join("mod.rs")).expect("read bus/mod.rs");
    let accessors =
        std::fs::read_to_string(dir.join("accessors.rs")).expect("read bus/accessors.rs");
    let hooks =
        std::fs::read_to_string(dir.join("device_hooks.rs")).expect("read bus/device_hooks.rs");

    // Anti-vacuity FIRST: the scan must be reading the real struct. If these
    // disappear, every assertion below passes by measuring nothing.
    for present in [
        "pub gpio_devices: Vec<Box<dyn BusResidentDevice>>",
        "pub observed: Vec<std::sync::Arc<dyn ObservedDevice>>",
    ] {
        assert!(
            mod_rs.contains(present),
            "field scan lost `{present}` — it is reading the wrong file, so \
             this test would pass by finding no fields at all"
        );
    }
    assert!(
        accessors.contains("maybe_service_edge_driven_gpio_devices"),
        "accessors.rs no longer calls the GENERIC edge hook — without it the \
         displays are not serviced at all and this test is measuring nothing"
    );

    for banned in [
        "tm1637_7seg::Tm1637",
        "seven_segment::SevenSegment",
        "pub tm1637",
        "pub seven_segment",
    ] {
        assert!(
            !mod_rs.contains(banned),
            "`{banned}` is back on SystemBus. A display binds on PINS: it is a \
             `BusResidentDevice` on `gpio_devices` and reports through \
             `BusResidentDevice::evidence`. A typed field per part is the \
             plumbing this port removed."
        );
    }

    for banned in ["maybe_clock_tm1637", "maybe_sample_seven_segment"] {
        assert!(
            !accessors.contains(&format!("self.{banned}")),
            "`bus/accessors.rs` calls `{banned}` again — that is a bespoke \
             per-part MMIO write hook, and `maybe_service_edge_driven_gpio_\
             devices` already does the job for every resident device."
        );
        assert!(
            !hooks.contains(&format!("fn {banned}")),
            "`{banned}` is defined again in bus/device_hooks.rs"
        );
    }
}
