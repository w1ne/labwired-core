// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! `debug_schema` names registers. It must never *model* one.
//!
//! A chip peripheral can carry an optional `config.debug_schema` pointing at a
//! `PeripheralDescriptor`. That descriptor exists so a debugger can render
//! `OUT = 0x00000020 [PIN5=1]` instead of "No register descriptors available"
//! for a peripheral whose behaviour lives in hand-written Rust.
//!
//! The hazard it introduces is a reporting one: attaching a datasheet's worth of
//! register names to a chip could look like the chip gained register coverage.
//! It has not. The schema is decode metadata; it changes nothing the bus does.
//!
//! `register_coverage` measures the live bus (does an address respond? does it
//! hold state?), so in principle it cannot be moved by schema at all. This test
//! pins that reasoning down empirically for the chip that carries the largest
//! schema, so nobody has to re-derive it — and so a future change that DID let
//! schema leak into bus behaviour fails here loudly.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

fn root(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// Build the nRF52840 bus twice: as configured, and with every `debug_schema`
/// key stripped out. The two must be indistinguishable to anything that probes
/// addresses.
fn build(strip_schema: bool) -> (SystemBus, usize) {
    let chip_path = root("configs/chips/nrf52840.yaml");
    let mut chip: ChipDescriptor =
        ChipDescriptor::from_file(&chip_path).expect("nrf52840 chip yaml");

    let with_schema = chip
        .peripherals
        .iter()
        .filter(|p| p.config.contains_key("debug_schema"))
        .count();

    if strip_schema {
        for peripheral in &mut chip.peripherals {
            peripheral.config.remove("debug_schema");
        }
    }

    // Parse the manifest from YAML rather than constructing it, so this goes
    // through the same deserialisation the real load path uses.
    let manifest: SystemManifest = serde_yaml::from_str(&format!(
        "name: schema-fidelity-probe\nchip: \"{}\"\n",
        chip_path.display()
    ))
    .expect("probe manifest parses");

    let bus = SystemBus::from_config(&chip, &manifest).expect("bus builds");
    (bus, with_schema)
}

#[test]
fn debug_schema_does_not_change_bus_behaviour() {
    let (with, schema_count) = build(false);
    let (without, _) = build(true);

    // Guard against a vacuous pass: if nothing carries a schema, the comparison
    // below proves nothing.
    assert!(
        schema_count > 40,
        "expected nRF52840 to carry debug_schema on most peripherals, found {schema_count}"
    );

    assert_eq!(
        with.peripherals.len(),
        without.peripherals.len(),
        "schema changed the peripheral count"
    );

    // Probe every peripheral's window the way register_coverage does: read, then
    // write-and-read-back. If schema were leaking into behaviour, one of these
    // would diverge.
    for (a, b) in with.peripherals.iter().zip(without.peripherals.iter()) {
        assert_eq!(a.name, b.name, "schema reordered the peripherals");
        assert_eq!(a.base, b.base, "{}: schema moved the base address", a.name);
        assert_eq!(a.size, b.size, "{}: schema changed the window size", a.name);
        assert_eq!(a.irq, b.irq, "{}: schema changed the interrupt", a.name);
    }

    let mut with = with;
    let mut without = without;
    let mut probed = 0u32;

    for index in 0..with.peripherals.len() {
        let base = with.peripherals[index].base;
        let size = with.peripherals[index].size.min(0x400);
        let name = with.peripherals[index].name.clone();

        for offset in (0..size).step_by(4) {
            let addr = base + offset;

            let read_with = with.read_u32(addr).ok();
            let read_without = without.read_u32(addr).ok();
            assert_eq!(
                read_with, read_without,
                "{name}: reset read diverged at {addr:#010x} — schema affected the model"
            );

            let _ = with.write_u32(addr, 0xFFFF_FFFF);
            let _ = without.write_u32(addr, 0xFFFF_FFFF);
            assert_eq!(
                with.read_u32(addr).ok(),
                without.read_u32(addr).ok(),
                "{name}: write-back diverged at {addr:#010x} — schema affected the model"
            );

            probed += 1;
        }
    }

    // The probe must actually have touched a meaningful amount of address space.
    assert!(probed > 1_000, "probe covered only {probed} words");
}
