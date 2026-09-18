// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **74-series logic on a real board, with no firmware.**
//!
//! The unit tests beside the model (`peripherals/components/declarative_logic.rs`)
//! drive it against a fifty-line fake pad map. That proves the truth table and
//! the timing; it cannot prove the part is WIRED. Between the model and a board
//! sit the descriptor's `config:` keys, the `<role>_pin` defaults, two pad
//! resolvers (ODR for what the MCU drives, IDR for what it samples), the
//! peripheral tick and the MMIO write hook. A part can be perfectly correct and
//! attach to nothing — that is the failure mode this file exists for, and it is
//! silent: attach succeeds, the manifest lists the part, and no pin ever moves.
//!
//! So every test here places a real descriptor on a real chip from
//! `configs/chips/`, drives an input pad the way firmware would (a store to the
//! GPIO output register) and reads the answer the way firmware would (a load
//! from the input register). No ELF, no CPU: the pads are the whole interface,
//! which is what makes a gate testable without firmware at all.
//!
//! Two chips on purpose. The STM32L476's input word accepts an MMIO store and
//! the ESP32-C3's does not — a device that drove only `drive_idr_bit` would
//! pass every test on the first and be silently inert on the second, which is
//! the defect `gpio_devices_drive_read_only_inputs.rs` exists for. One C3 test
//! here keeps this family out of that hole.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// RCC_AHB2ENR on the L476 (RM0351 §6.4.17). GPIOA..GPIOC are unclocked out of
/// reset and drop every register write until firmware sets their bit, so a test
/// that skipped this would drive nothing and prove nothing.
const L476_RCC_AHB2ENR: u64 = 0x4002_104C;

/// Build a bus for `chip` with one `external_devices` entry.
///
/// `config` is the YAML body of the placement's `config:` block — the pad
/// labels, keyed by the descriptor's `<role>_pin` convention.
fn bus_with(chip_file: &str, device_type: &str, id: &str, config: &str) -> SystemBus {
    let chip_path = workspace_root().join("configs/chips").join(chip_file);
    let chip =
        ChipDescriptor::from_file(&chip_path).unwrap_or_else(|e| panic!("load {chip_file}: {e:#}"));
    let indented: String = config
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| format!("      {}\n", l.trim()))
        .collect();
    let yaml = format!(
        "name: \"logic-gate-test\"\nchip: \"{}\"\nboard_io: []\nexternal_devices:\n  \
         - id: \"{id}\"\n    type: \"{device_type}\"\n    connection: \"gpio\"\n    config:\n{indented}",
        chip_path.display(),
    );
    let manifest: SystemManifest =
        serde_yaml::from_str(&yaml).unwrap_or_else(|e| panic!("manifest parses: {e}\n{yaml}"));
    SystemBus::from_config(&chip, &manifest)
        .unwrap_or_else(|e| panic!("attach {device_type}: {e:#}\n{yaml}"))
}

/// Clock GPIOA/B/C so the port accepts stores. Firmware's first act, performed
/// here because there is no firmware.
fn clock_l476_gpio(bus: &mut SystemBus) {
    bus.write_u32(L476_RCC_AHB2ENR, 0b111)
        .expect("enable GPIOA/B/C clocks");
}

/// Drive a pad the way firmware does: a store to the port's OUTPUT register.
/// This is also what services the part synchronously through
/// `edge_service_addrs`, so the level is latched at the current cycle.
fn drive(bus: &mut SystemBus, pin: &str, high: bool) {
    let (addr, bit) = SystemBus::resolve_pin_odr_pub(bus, pin)
        .unwrap_or_else(|| panic!("{pin} resolves to an output register"));
    let mut word = bus.read_u32(addr).expect("read ODR");
    if high {
        word |= 1 << bit;
    } else {
        word &= !(1 << bit);
    }
    bus.write_u32(addr, word).expect("write ODR");
}

/// Sample a pad the way firmware does: a load from the port's INPUT register.
fn sample(bus: &mut SystemBus, pin: &str) -> bool {
    let (addr, bit) = SystemBus::resolve_pin_idr_pub(bus, pin)
        .unwrap_or_else(|| panic!("{pin} resolves to an input register"));
    bus.read_u32(addr).expect("read IDR") >> bit & 1 != 0
}

/// Advance the peripheral tick from `from` for `cycles` cycles, as
/// `Machine::advance` does. This is what lets a propagation deadline EXPIRE:
/// the write hook only runs when firmware writes, and a gate whose input has
/// settled has to come good with nothing writing anything.
fn tick(bus: &mut SystemBus, from: u64, cycles: u64) -> u64 {
    for c in from..(from + cycles) {
        bus.set_current_cycle(c);
        let _ = bus.tick_peripherals_fully();
    }
    from + cycles
}

// ─── the combinational gate ────────────────────────────────────────────────

/// `74hc00` pin 1 gate on PA0/PA1 → PA4, plus the three unused gates parked on
/// pins that exist. A descriptor binds EVERY role, so a placement must set
/// every key — which is itself worth pinning: a part that attached with half
/// its pads unbound would run with three gates reading a pad nobody chose.
const NAND_PINS: &str = r#"
a1_pin: "PA0"
b1_pin: "PA1"
y1_pin: "PA4"
a2_pin: "PB0"
b2_pin: "PB1"
y2_pin: "PB2"
a3_pin: "PB3"
b3_pin: "PB4"
y3_pin: "PB5"
a4_pin: "PB6"
b4_pin: "PB7"
y4_pin: "PB8"
"#;

#[test]
fn a_74hc00_answers_its_truth_table_on_stm32l476_pads() {
    for (a, b, want) in [
        (false, false, true),
        (false, true, true),
        (true, false, true),
        (true, true, false),
    ] {
        let mut bus = bus_with("stm32l476.yaml", "74hc00", "u1", NAND_PINS);
        clock_l476_gpio(&mut bus);
        drive(&mut bus, "PA0", a);
        drive(&mut bus, "PA1", b);
        tick(&mut bus, 0, 64);
        assert_eq!(
            sample(&mut bus, "PA4"),
            want,
            "NAND({a}, {b}) must read {want} on PA4 — this is the descriptor's own \
             `table:` reaching a real pad through the real resolvers"
        );
    }
}

/// **Negative control for the table.** The test above passes for two very
/// different reasons: the descriptor's table is being evaluated, or the engine
/// drives some fixed function that happens to agree. Placing a `74hc08` (AND)
/// on the identical pins and asserting the OPPOSITE level at every input
/// combination separates them — an engine ignoring `table:` would give the same
/// answer for both parts.
#[test]
fn an_and_gate_and_a_nand_gate_disagree_on_every_input() {
    const AND_PINS: &str = NAND_PINS;
    for (a, b) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut nand = bus_with("stm32l476.yaml", "74hc00", "u1", NAND_PINS);
        let mut and = bus_with("stm32l476.yaml", "74hc08", "u1", AND_PINS);
        for bus in [&mut nand, &mut and] {
            clock_l476_gpio(bus);
            drive(bus, "PA0", a);
            drive(bus, "PA1", b);
            tick(bus, 0, 64);
        }
        assert_ne!(
            sample(&mut nand, "PA4"),
            sample(&mut and, "PA4"),
            "a NAND and an AND wired identically must never agree; they did at \
             ({a}, {b}), which means PA4 is not following the descriptor's table"
        );
    }
}

// ─── propagation delay ─────────────────────────────────────────────────────

/// **The output moves after `tprop_ns`, and not before.**
///
/// `cpu_hz: 1_000_000_000` on the placement makes one nanosecond exactly one
/// simulated cycle, so the `74hc00`'s datasheet 9 ns is 9 cycles and the
/// assertion can walk them one at a time. At the board's own 80 MHz the same
/// 9 ns rounds to a single cycle (the engine floors the delay at one, because a
/// gate that answered inside the store that moved its input is not a gate) —
/// true, but a one-cycle window is a weak thing to assert on. The override
/// changes the clock, not the model.
#[test]
fn a_gate_output_moves_after_tprop_and_not_before() {
    let pins = format!("{NAND_PINS}cpu_hz: 1000000000\n");
    let mut bus = bus_with("stm32l476.yaml", "74hc00", "u1", &pins);
    clock_l476_gpio(&mut bus);

    // Settle with both inputs LOW: Y1 idles HIGH.
    drive(&mut bus, "PA0", false);
    drive(&mut bus, "PA1", false);
    let start = tick(&mut bus, 0, 64);
    assert!(
        sample(&mut bus, "PA4"),
        "precondition: an idle NAND holds HIGH"
    );

    // Raise both inputs at `start`. The store services the part synchronously,
    // so the new levels are latched here — but the answer is not published.
    bus.set_current_cycle(start);
    drive(&mut bus, "PA0", true);
    drive(&mut bus, "PA1", true);
    assert!(
        sample(&mut bus, "PA4"),
        "PA4 moved inside the very store that moved its inputs — a gate has a \
         propagation delay and this one claims 9 ns"
    );

    // Cycles 1..8 after the change: still the old level.
    for step in 1..9u64 {
        tick(&mut bus, start + step, 1);
        assert!(
            sample(&mut bus, "PA4"),
            "PA4 fell {step} cycle(s) after the input change; tprop_ns is 9 and one \
             nanosecond is one cycle at this placement's clock"
        );
    }

    // Cycle 9: the answer lands.
    tick(&mut bus, start + 9, 1);
    assert!(
        !sample(&mut bus, "PA4"),
        "PA4 must fall on the 9th cycle after the input change — if it never falls, \
         the deadline is never expiring and the part is only serviced on writes"
    );
}

// ─── the transceiver ───────────────────────────────────────────────────────

/// One bit of a `74hc245`: A1 on PA0, B1 on PA1, DIR on PA2, OE on PA3. The
/// other seven bits are parked on GPIOB pads so every role is bound.
fn xcvr_pins() -> String {
    let mut s =
        String::from("dir_pin: \"PA2\"\noe_pin: \"PA3\"\na1_pin: \"PA0\"\nb1_pin: \"PA1\"\n");
    for n in 2..=8 {
        s.push_str(&format!("a{n}_pin: \"PB{}\"\n", n - 2));
        s.push_str(&format!("b{n}_pin: \"PC{}\"\n", n - 2));
    }
    s
}

#[test]
fn a_74hc245_drives_the_side_dir_points_at() {
    let mut bus = bus_with("stm32l476.yaml", "74hc245", "u2", &xcvr_pins());
    clock_l476_gpio(&mut bus);

    // Enabled, DIR HIGH = A to B. The MCU drives A1 HIGH; B1 must follow.
    drive(&mut bus, "PA3", false); // OE low
    drive(&mut bus, "PA2", true); // DIR high
    drive(&mut bus, "PA0", true); // A1 high
    drive(&mut bus, "PA1", false); // the B1 pad's own output register is low
    let now = tick(&mut bus, 0, 64);
    assert!(sample(&mut bus, "PA1"), "DIR high must send A1 to B1");

    // ⚠️ THE OTHER HALF, and the one that makes this a direction test rather
    // than a wiring test. Flip DIR: the part must now READ B1 and DRIVE A1 —
    // with B1's driving level LOW, A1 must go LOW, which is the opposite of
    // what it reads now. A model that ignored DIR would leave both alone.
    drive(&mut bus, "PA2", false);
    let now = tick(&mut bus, now, 64);
    assert!(
        !sample(&mut bus, "PA0"),
        "DIR low must send B1 to A1; PA0 still reading HIGH means the direction pad \
         is not being read at all"
    );

    // OE HIGH isolates both buses: neither side is driven any more. Proving
    // that on a pad whose level is sticky means proving the part stops
    // FOLLOWING — so move B1 and check A1 does not answer.
    drive(&mut bus, "PA3", true); // OE high
    let now = tick(&mut bus, now, 64);
    drive(&mut bus, "PA1", true); // B1 goes high while isolated
    tick(&mut bus, now, 64);
    assert!(
        !sample(&mut bus, "PA0"),
        "with OE high the transceiver is off the wire; A1 must not follow B1"
    );
}

// ─── the three-state buffer ────────────────────────────────────────────────

fn buf125_pins() -> String {
    let mut s = String::from("a1_pin: \"PA0\"\ny1_pin: \"PA4\"\noe1_pin: \"PA2\"\n");
    for n in 2..=4 {
        s.push_str(&format!("a{n}_pin: \"PB{}\"\n", n * 3));
        s.push_str(&format!("y{n}_pin: \"PB{}\"\n", n * 3 + 1));
        s.push_str(&format!("oe{n}_pin: \"PB{}\"\n", n * 3 + 2));
    }
    s
}

#[test]
fn a_disabled_74hc125_buffer_stops_following_its_input() {
    let mut bus = bus_with("stm32l476.yaml", "74hc125", "u3", &buf125_pins());
    clock_l476_gpio(&mut bus);

    drive(&mut bus, "PA2", false); // 1OE low = enabled
    drive(&mut bus, "PA0", true); // 1A high
    let now = tick(&mut bus, 0, 64);
    assert!(
        sample(&mut bus, "PA4"),
        "an enabled buffer follows its input"
    );

    // Disable, then move the input. Hi-Z on this twin means the part RELEASES
    // the pad: there is no resistor network, so the pad keeps the level it had
    // and the observable is that it stops answering. A part that had merely
    // gone idle would not survive this second half.
    drive(&mut bus, "PA2", true); // 1OE high = released
    let now = tick(&mut bus, now, 64);
    drive(&mut bus, "PA0", false); // 1A falls while released
    let now = tick(&mut bus, now, 64);
    assert!(
        sample(&mut bus, "PA4"),
        "a released output must not follow its input; PA4 following 1A LOW means the \
         enable is not gating anything"
    );

    // Re-enable: it must come back at the input's CURRENT level, not the one it
    // was holding when it went away.
    drive(&mut bus, "PA2", false);
    tick(&mut bus, now, 64);
    assert!(
        !sample(&mut bus, "PA4"),
        "re-enabling must publish the input level the part has now"
    );
}

// ─── the read-only input word ──────────────────────────────────────────────

/// The ESP32-C3's GPIO input word is READ-ONLY to firmware, so a store to it is
/// correctly dropped and a device that reached its pad only through
/// `drive_idr_bit` would be silently inert on this whole chip family. One gate
/// on the C3 is what keeps this primitive out of that hole.
#[test]
fn a_logic_gate_drives_a_read_only_input_word_on_esp32c3() {
    let mut pins = String::new();
    for (i, n) in (1..=6).enumerate() {
        pins.push_str(&format!("a{n}_pin: \"GPIO{}\"\n", i * 2));
        pins.push_str(&format!("y{n}_pin: \"GPIO{}\"\n", i * 2 + 1));
    }
    let mut bus = bus_with("esp32c3.yaml", "74hc04", "u4", &pins);

    drive(&mut bus, "GPIO0", true);
    let now = tick(&mut bus, 0, 64);
    assert!(
        !sample(&mut bus, "GPIO1"),
        "an inverter fed HIGH must drive its output LOW on the C3 too; reading HIGH \
         here is the pad never having been driven at all"
    );

    drive(&mut bus, "GPIO0", false);
    tick(&mut bus, now, 64);
    assert!(
        sample(&mut bus, "GPIO1"),
        "and it must follow back — a pad stuck at one level proves as little as one \
         that never moved"
    );
}

// ─── the bus switch ────────────────────────────────────────────────────────

#[test]
fn a_74cbtlv3257_follows_its_select_pad() {
    let mut pins = String::from("s_pin: \"PA2\"\noe_pin: \"PA3\"\n");
    pins.push_str("a1_pin: \"PA0\"\nb1_pin: \"PA1\"\ny1_pin: \"PA4\"\n");
    for n in 2..=4 {
        pins.push_str(&format!("a{n}_pin: \"PB{}\"\n", n * 3));
        pins.push_str(&format!("b{n}_pin: \"PB{}\"\n", n * 3 + 1));
        pins.push_str(&format!("y{n}_pin: \"PB{}\"\n", n * 3 + 2));
    }
    let mut bus = bus_with("stm32l476.yaml", "74cbtlv3257", "u5", &pins);
    clock_l476_gpio(&mut bus);

    drive(&mut bus, "PA3", false); // OE low = switches closed
    drive(&mut bus, "PA0", true); // 1A high
    drive(&mut bus, "PA1", false); // 1B low
    drive(&mut bus, "PA2", false); // S low = pick the A side
    let now = tick(&mut bus, 0, 64);
    assert!(sample(&mut bus, "PA4"), "S low selects 1A");

    drive(&mut bus, "PA2", true); // S high = pick the B side
    tick(&mut bus, now, 64);
    assert!(!sample(&mut bus, "PA4"), "S high selects 1B");
}
