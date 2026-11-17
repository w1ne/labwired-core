// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! TSOP38238-style IR receiver and NEC remote source: OUT/IR idle HIGH,
//! active-low demodulated NEC envelopes with 9 ms / 4.5 ms leader, 562.5 µs
//! bit marks, 0/1 spaces, a stop mark, and 108 ms repeat frames.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::Path;

const PORTD: u64 = 0x0001_0029;
const OUT_BIT: u8 = 2;
const CPU_HZ: u64 = 2_000_000;
const BF00: f64 = 0xBF00 as f64;
const CMD_1: f64 = 0x10 as f64;

fn cycles(us: f64) -> u64 {
    (us * CPU_HZ as f64 / 1_000_000.0).round() as u64
}

fn nec_raw(address: u32, command: u32) -> u32 {
    let cmd = command & 0xff;
    let inv = (!command) & 0xff;
    if address > 255 {
        (address & 0xffff) | (cmd << 16) | (inv << 24)
    } else {
        (address & 0xff) | (((!address) & 0xff) << 8) | (cmd << 16) | (inv << 24)
    }
}

/// Edge list for one NEC frame: (cycle offset from the first falling edge, level).
fn nec_frame_edges(address: u32, command: u32) -> Vec<(u64, bool)> {
    let mut edges = vec![(0, false)];
    let mut t = cycles(9000.0);
    edges.push((t, true));
    t += cycles(4500.0);
    let mut raw = nec_raw(address, command);
    for _ in 0..32 {
        edges.push((t, false));
        t += cycles(562.5);
        edges.push((t, true));
        t += if raw & 1 == 0 {
            cycles(562.5)
        } else {
            cycles(1687.5)
        };
        raw >>= 1;
    }
    edges.push((t, false));
    t += cycles(562.5);
    edges.push((t, true));
    let _ = t;
    edges
}

fn repeat_edges() -> Vec<(u64, bool)> {
    let mut t = 0;
    let mut edges = vec![(t, false)];
    t += cycles(9000.0);
    edges.push((t, true));
    t += cycles(2250.0);
    edges.push((t, false));
    t += cycles(562.5);
    edges.push((t, true));
    edges
}

fn bus_for(device_type: &str, id: &str, pin_key: &str, input: &str) -> SystemBus {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip = ChipDescriptor::from_file(root.join("configs/chips/atmega328p.yaml")).unwrap();
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        r#"
name: nec-ir
chip: atmega328p.yaml
external_devices:
  - id: {id}
    type: {device_type}
    connection: gpio
    config:
      {pin_key}: PD2
      cpu_hz: {CPU_HZ}
board_io: []
"#
    ))
    .unwrap();
    let _ = input;
    SystemBus::from_config(&chip, &manifest).unwrap()
}

fn service(bus: &mut SystemBus, cycle: u64) {
    bus.set_current_cycle(cycle);
    let _ = bus.tick_peripherals_fully();
}

fn out_high(bus: &SystemBus) -> bool {
    bus.read_u8(PORTD).unwrap() & (1 << OUT_BIT) != 0
}

/// Apply address + command/key at cycle 0 and return the first falling-edge cycle.
fn press(bus: &mut SystemBus, id: &str, command_key: &str, address: f64, command: f64) -> u64 {
    service(bus, 0);
    assert!(out_high(bus), "idle OUT/IR must be HIGH");
    bus.set_input(Some(id), "address", address).unwrap();
    bus.set_input(Some(id), command_key, command).unwrap();
    service(bus, 0);
    if !out_high(bus) {
        return 0;
    }
    // Seeded inputs do not fire; the press is serviced on the next tick.
    service(bus, 1);
    assert!(
        !out_high(bus),
        "press must pull OUT/IR LOW for the 9 ms leader"
    );
    1
}

fn assert_edges(bus: &mut SystemBus, origin: u64, edges: &[(u64, bool)], label: &str) {
    for (i, &(offset, level)) in edges.iter().enumerate() {
        let at = origin + offset;
        service(bus, at);
        assert_eq!(
            out_high(bus),
            level,
            "{label} edge {i} at +{offset} cyc: expected {} got {}",
            if level { "HIGH" } else { "LOW" },
            if out_high(bus) { "HIGH" } else { "LOW" }
        );
        if i + 1 < edges.len() {
            let next = origin + edges[i + 1].0;
            if next > at + 1 {
                let mid = at + (next - at) / 2;
                service(bus, mid);
                assert_eq!(
                    out_high(bus),
                    level,
                    "{label} mid-segment after edge {i} at {mid} should stay {}",
                    if level { "HIGH" } else { "LOW" }
                );
            }
        }
    }
}

#[test]
fn tsop38238_extended_nec_frame_bf00_command_0x10() {
    let mut bus = bus_for("ir_receiver_tsop38238", "ir", "out_pin", "command");
    let origin = press(&mut bus, "ir", "command", BF00, CMD_1);
    let edges = nec_frame_edges(0xBF00, 0x10);
    assert_edges(&mut bus, origin, &edges, "extended 0xBF00/0x10 frame");
    // LSB of 0xBF00 is 0, so the first bit space is the short 562.5 µs HIGH.
    assert_eq!(nec_raw(0xBF00, 0x10), 0xEF10_BF00);
    assert_eq!(0xBF00 & 1, 0);
}

#[test]
fn tsop38238_8bit_address_frame() {
    let mut bus = bus_for("ir_receiver_tsop38238", "ir", "out_pin", "command");
    let origin = press(&mut bus, "ir", "command", 0x00 as f64, 0x1A as f64);
    let edges = nec_frame_edges(0x00, 0x1A);
    assert_edges(&mut bus, origin, &edges, "8-bit address 0x00/0x1A frame");
    assert_eq!(nec_raw(0x00, 0x1A), 0xE51A_FF00);
}

#[test]
fn tsop38238_repeat_frame_at_108ms_while_held() {
    let mut bus = bus_for("ir_receiver_tsop38238", "ir", "out_pin", "command");
    let origin = press(&mut bus, "ir", "command", BF00, CMD_1);
    let frame = nec_frame_edges(0xBF00, 0x10);
    assert_edges(&mut bus, origin, &frame, "first frame");
    // After the stop mark, OUT idles HIGH until the 108 ms repeat leader.
    let after_stop = origin + frame.last().unwrap().0;
    let repeat_at = origin + cycles(108_000.0);
    assert!(
        repeat_at > after_stop + cycles(1000.0),
        "repeat must start after the first frame ends"
    );
    service(&mut bus, after_stop + cycles(1000.0));
    assert!(out_high(&bus), "idle HIGH between frame and repeat");
    assert_edges(
        &mut bus,
        repeat_at,
        &repeat_edges(),
        "108 ms NEC repeat frame",
    );
}

#[test]
fn tsop38238_release_cancels_frame_and_returns_idle_high() {
    let mut bus = bus_for("ir_receiver_tsop38238", "ir", "out_pin", "command");
    let origin = press(&mut bus, "ir", "command", BF00, CMD_1);
    // Mid-leader (~1 ms): still LOW, then release.
    service(&mut bus, origin + cycles(1000.0));
    assert!(!out_high(&bus), "still in the 9 ms leader");
    bus.set_input(Some("ir"), "command", -1.0).unwrap();
    service(&mut bus, origin + cycles(1000.0));
    service(&mut bus, origin + cycles(1000.0) + 1);
    assert!(out_high(&bus), "release must restore idle HIGH");
    service(&mut bus, origin + cycles(20_000.0));
    assert!(out_high(&bus), "released OUT stays HIGH (no leftover bits)");
    service(&mut bus, origin + cycles(108_000.0));
    assert!(out_high(&bus), "released key must not emit a repeat frame");
}

#[test]
fn nec_remote_emits_same_extended_frame_on_ir() {
    let mut bus = bus_for("ir_remote_nec", "rc", "ir_pin", "key");
    let origin = press(&mut bus, "rc", "key", BF00, CMD_1);
    let edges = nec_frame_edges(0xBF00, 0x10);
    assert_edges(&mut bus, origin, &edges, "remote 0xBF00/0x10 frame on IR");
}

#[test]
fn nec_remote_repeat_and_release() {
    let mut bus = bus_for("ir_remote_nec", "rc", "ir_pin", "key");
    let origin = press(&mut bus, "rc", "key", BF00, 0x05 as f64);
    let frame = nec_frame_edges(0xBF00, 0x05);
    assert_edges(&mut bus, origin, &frame, "remote Play/Pause frame");
    assert_edges(
        &mut bus,
        origin + cycles(108_000.0),
        &repeat_edges(),
        "remote 108 ms repeat",
    );
    bus.set_input(Some("rc"), "key", -1.0).unwrap();
    let after = origin + cycles(108_000.0) + cycles(9000.0) + cycles(2250.0) + cycles(562.5) + 1;
    service(&mut bus, after);
    assert!(out_high(&bus), "remote release returns IR to idle HIGH");
    service(&mut bus, origin + cycles(216_000.0));
    assert!(
        out_high(&bus),
        "released remote must not emit a second repeat"
    );
}
