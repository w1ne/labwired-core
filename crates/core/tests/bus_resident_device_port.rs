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
