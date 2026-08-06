// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! **The `labwired.part/v1` contract, asserted from outside the engine.**
//!
//! Everything here is derived from `docs/part-packs.md` and the TMP102/TMP1075
//! datasheet shape it borrows — not from reading the implementation. A pack
//! author who has only the contract document should be able to predict every
//! assertion below; where they could not, the contract is under-specified and
//! that is the bug.
//!
//! The three negative controls matter as much as the positive case. A test that
//! only asserts "the private part reads 0x1234" passes just as well if the
//! device attached for some unrelated reason, so each positive case is paired
//! with the same manifest minus the one thing under test.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::esp32c3::i2c::Esp32c3I2c;
use labwired_core::peripherals::i2c::I2cDevice;
use std::path::PathBuf;

/// A private I²C temperature sensor that exists nowhere in this repository.
///
/// Wire protocol is the register-pointer shape every such datasheet describes:
/// the master writes a one-byte pointer, then streams a 16-bit big-endian word.
/// `reset: 0x1234` therefore reads back as the two bytes `0x12, 0x34` — that is
/// a datasheet fact about big-endian framing, not an implementation detail.
const ACME_PACK: &str = r#"
schema: labwired.part/v1
type: "acme:tmp999"
source: acme-private
behavior:
  primitive: i2c_device
  i2c:
    default_address: 0x4a
    pointer_mask: 0x03
    registers:
      - { name: TEMP, addr: 0x00, width: 2, endian: be, access: r, reset: 0x1234 }
"#;

/// Build a manifest carrying `packs`, wiring one device of `device_type` to the
/// C3's I²C0. Composed through `serde_yaml::Value` so the test says what it
/// means instead of depending on the indentation of a string literal.
fn manifest_yaml(packs: &[&str], device_type: &str) -> String {
    let mut root: serde_yaml::Mapping = serde_yaml::from_str(&format!(
        r#"
schema_version: "1.0"
name: "part-pack-contract"
chip: "esp32c3"
external_devices:
  - id: t1
    type: "{device_type}"
    connection: i2c0
    route: {{ sda: "GPIO4", scl: "GPIO5" }}
"#
    ))
    .expect("harness manifest is valid YAML");

    if !packs.is_empty() {
        let parts: Vec<serde_yaml::Value> = packs
            .iter()
            .map(|p| serde_yaml::from_str(p).expect("harness pack is valid YAML"))
            .collect();
        root.insert("parts".into(), serde_yaml::Value::Sequence(parts));
    }
    serde_yaml::to_string(&root).expect("harness manifest serialises")
}

/// Parse and build exactly as the browser does — `from_yaml`, not `from_file`.
///
/// That is deliberate: `from_file` is the CLI path, and validating there only
/// would mean the hosted and browser runtimes accept manifests the CLI rejects.
/// Every contract rule asserted in this file must be enforced by the engine
/// itself, so the harness does nothing but hand it the document.
fn build_bus(manifest_src: &str) -> anyhow::Result<SystemBus> {
    let manifest: SystemManifest = SystemManifest::from_yaml(manifest_src)?;
    // The C3 descriptor references relative peripheral sub-YAMLs, so it must be
    // loaded from its real path rather than the embedded copy.
    let chip_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/esp32c3.yaml");
    let chip = ChipDescriptor::from_file(&chip_path)?;
    SystemBus::from_config(&chip, &manifest)
}

/// The error a rejected manifest produces. `SystemBus` is not `Debug`, and the
/// negative cases only ever care about what the engine said.
fn build_error(manifest_src: &str, why: &str) -> String {
    match build_bus(manifest_src) {
        Ok(_) => panic!("{why}"),
        Err(e) => format!("{e:#}"),
    }
}

/// Point at `reg`, repeated-START, read `n` bytes — the datasheet's read framing.
fn read_reg(d: &mut dyn I2cDevice, reg: u8, n: usize) -> Vec<u8> {
    d.start();
    d.write(reg);
    d.start();
    let out: Vec<u8> = (0..n).map(|_| d.read()).collect();
    d.stop();
    out
}

fn slave_bytes_at(bus: &mut SystemBus, address: u8, reg: u8, n: usize) -> Option<Vec<u8>> {
    let idx = bus.find_peripheral_index_by_name("i2c0")?;
    let any = bus.peripherals[idx].dev.as_any_mut()?;
    let c3 = any.downcast_mut::<Esp32c3I2c>()?;
    let slave = c3
        .attached_slaves_mut()
        .iter_mut()
        .find(|d| d.address() == address)?;
    Some(read_reg(slave.as_mut(), reg, n))
}

// ─── the part connects ─────────────────────────────────────────────────────

#[test]
fn a_part_the_engine_has_never_seen_attaches_and_answers() {
    let src = manifest_yaml(&[ACME_PACK], "acme:tmp999");
    let mut bus = build_bus(&src).expect("a manifest carrying its own part must build");

    let bytes = slave_bytes_at(&mut bus, 0x4a, 0x00, 2)
        .expect("the pack's device must be attached at its declared address");
    assert_eq!(
        bytes,
        vec![0x12, 0x34],
        "a 16-bit big-endian register with reset 0x1234 reads MSB first"
    );
}

/// The negative control for the test above: without `parts:`, the very same
/// manifest must NOT quietly produce a working device. Unknown types fail loud
/// (a green run with a silently missing device proves nothing — see
/// `from_config` residual attach). If this ever passes, the pack no longer
/// proves anything.
#[test]
fn the_same_part_without_its_pack_is_rejected() {
    let src = manifest_yaml(&[], "acme:tmp999");
    let msg = build_error(
        &src,
        "an unknown device type must fail the build, not attach a silent stub",
    );
    assert!(
        msg.contains("unsupported type") && msg.contains("acme:tmp999"),
        "the error must name the missing type, got: {msg}"
    );
}

// ─── shadowing a built-in is deliberate or it is an error ──────────────────

#[test]
fn shadowing_a_builtin_without_declaring_it_is_refused() {
    let pack = ACME_PACK.replace("\"acme:tmp999\"", "tmp102");
    let src = manifest_yaml(&[&pack], "tmp102");

    let msg = build_error(
        &src,
        "silently replacing a built-in model must not be possible",
    );
    assert!(
        msg.contains("shadows a built-in"),
        "the error must say what happened, got: {msg}"
    );
    assert!(
        msg.contains("overrides: tmp102"),
        "the error must name the way out, got: {msg}"
    );
    assert!(
        msg.contains("acme-private"),
        "the error must name the source that shipped the pack, got: {msg}"
    );
}

#[test]
fn declaring_the_override_replaces_the_builtin_model() {
    let pack = ACME_PACK.replace("\"acme:tmp999\"", "tmp102").replace(
        "source: acme-private",
        "source: acme-private\noverrides: tmp102",
    );
    let src = manifest_yaml(&[&pack], "tmp102");
    let mut bus = build_bus(&src).expect("a declared override must build");

    // 0x4a, not the built-in TMP102's 0x48: the pack's address proves the pack's
    // model is the one on the bus, not merely that nothing errored.
    let bytes = slave_bytes_at(&mut bus, 0x4a, 0x00, 2)
        .expect("the overriding pack's device must be the one attached");
    assert_eq!(bytes, vec![0x12, 0x34], "the pack's register map must win");
    assert!(
        slave_bytes_at(&mut bus, 0x48, 0x00, 2).is_none(),
        "the built-in TMP102 must be gone, not merely outvoted"
    );
}

// ─── the contract's own rules ──────────────────────────────────────────────

#[test]
fn two_packs_for_one_type_is_an_error_not_a_race() {
    let other = ACME_PACK.replace("source: acme-private", "source: someone-else");
    let src = manifest_yaml(&[ACME_PACK, &other], "acme:tmp999");

    let msg = build_error(&src, "one part is one document");
    assert!(
        msg.contains("acme-private") && msg.contains("someone-else"),
        "both sources must be named, got: {msg}"
    );
}

#[test]
fn a_pack_without_a_schema_declaration_is_refused() {
    let pack = ACME_PACK.replace("schema: labwired.part/v1\n", "");
    let src = manifest_yaml(&[&pack], "acme:tmp999");

    let msg = build_error(&src, "an out-of-tree file must say which schema it obeys");
    assert!(
        msg.contains("labwired.part/v1"),
        "the error must name the contract version"
    );
}

#[test]
fn an_unresolved_path_entry_reaching_the_engine_is_refused() {
    let src = manifest_yaml(&["path: ./acme.yaml"], "acme:tmp999");

    let msg = build_error(&src, "`path:` is a CLI convenience, not an engine feature");
    assert!(
        msg.contains("never loaded"),
        "the error must explain that the CLI inlines it, got: {msg}"
    );
}

// ─── the two halves actually meet ──────────────────────────────────────────

/// The manifest the APP emits, parsed by the ENGINE.
///
/// The fixture is `compile()` output captured verbatim from
/// `@labwired/board-config` — not hand-written to look like it. Both halves of
/// this contract can be individually correct and still not meet: the app's YAML
/// writer emits a namespaced `type: acme:tmp999` as an unquoted plain scalar
/// with a colon in it, and whether that survives the engine's parser is a fact
/// about two libraries, not a fact either side can assert alone.
///
/// Regenerate with the snippet in `core/docs/part-packs.md` if the emitter
/// changes; a diff here is a real cross-boundary change and wants reading.
#[test]
fn the_manifest_the_app_emits_is_one_the_engine_can_run() {
    let src = include_str!("fixtures/emitted-part-pack-manifest.yaml");
    let mut bus = build_bus(src).expect("compile() output must build on the engine");

    let bytes = slave_bytes_at(&mut bus, 0x4a, 0x00, 2)
        .expect("the emitted pack's device must be attached at its declared address");
    assert_eq!(
        bytes,
        vec![0x12, 0x34],
        "the register map survived the app → manifest → engine round trip"
    );
}

// ─── the other primitive that carries its own transport ────────────────────

/// SPI packs travel a different road than I²C ones: I²C devices are built by
/// the shared `build_i2c_tree` factory, while SPI devices only ever attach
/// through the `PeripheralKit` registry. So "packs work" proved on I²C alone
/// would be a claim about one of the two doors.
///
/// Framing and expected bytes are datasheet facts about a 4-wire register
/// device: the master sends `[R/W | MB | addr]` then clocks the register out.
/// A read of DEVID (0x00) is therefore command byte 0x80 followed by the
/// register's reset value.
#[test]
fn an_spi_pack_attaches_through_the_kit_door() {
    const SPI_PACK: &str = r#"
schema: labwired.part/v1
type: "acme:acc999"
source: acme-private
behavior:
  primitive: spi_device
  spi:
    framing: { command_bytes: 1, rw_bit: 7, rw_read_high: true, addr_mask: 0x3F, auto_increment: true }
    registers:
      - { name: DEVID, addr: 0x00, width: 1, endian: le, access: r, reset: 0xE5 }
"#;
    let mut root: serde_yaml::Mapping = serde_yaml::from_str(
        r#"
schema_version: "1.0"
name: "part-pack-spi"
chip: "esp32c3"
external_devices:
  - id: acc
    type: "acme:acc999"
    connection: spi2
    config: { cs_pin: "GPIO7" }
"#,
    )
    .unwrap();
    root.insert(
        "parts".into(),
        serde_yaml::Value::Sequence(vec![serde_yaml::from_str(SPI_PACK).unwrap()]),
    );
    let src = serde_yaml::to_string(&root).unwrap();

    let mut bus = build_bus(&src).expect("an SPI pack must build");
    let idx = bus
        .find_peripheral_index_by_name("spi2")
        .expect("the C3 declares spi2");
    let any = bus.peripherals[idx]
        .dev
        .as_any_mut()
        .expect("spi2 must downcast");
    let spi = any
        .downcast_mut::<labwired_core::peripherals::esp32c3::spi::Esp32c3Spi>()
        .expect("spi2 is the C3 controller");
    let devices = spi.attached_devices_mut();
    assert_eq!(devices.len(), 1, "the pack's device must be on the SPI bus");

    let dev = &mut devices[0];
    dev.cs_select();
    dev.transfer(0x80); // read, address 0x00
    let devid = dev.transfer(0x00);
    dev.cs_release();
    assert_eq!(
        devid, 0xE5,
        "the pack's declared reset value must clock out"
    );
}
