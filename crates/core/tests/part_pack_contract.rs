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
use labwired_core::system::xtensa::{attach_esp32_external_devices, configure_xtensa_esp32};
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

/// The classic ESP32/S3 browser path constructs its bank in Rust and calls the
/// exported attacher directly, so its contract cannot depend on `from_config`
/// having run first. A browser-style parsed manifest missing the pack schema
/// must fail before that direct path records or attaches anything.
#[test]
fn direct_xtensa_attach_validates_manifest_part_contracts() {
    let manifest = SystemManifest::from_yaml(
        r#"
schema_version: "1.0"
name: "direct-xtensa-part-pack-contract"
chip: "esp32"
external_devices: []
parts:
  - type: "acme:missing-schema"
    behavior:
      primitive: analog_source
"#,
    )
    .expect("raw browser-style manifest must deserialize before the engine validates it");
    let mut bus = SystemBus::new();
    let _cpu = configure_xtensa_esp32(&mut bus);

    let err = attach_esp32_external_devices(&mut bus, &manifest)
        .expect_err("direct Xtensa attachment must enforce the part-pack contract");
    assert!(
        format!("{err:#}").contains("missing `schema: labwired.part/v1`"),
        "the direct attach error must name the missing contract declaration: {err:#}"
    );
}

#[test]
fn direct_xtensa_attach_preflights_unused_pack_semantics() {
    let manifest = SystemManifest::from_yaml(
        r#"
schema_version: "1.0"
name: "direct-xtensa-invalid-pack"
chip: "esp32"
external_devices: []
parts:
  - schema: labwired.part/v1
    type: "acme:invalid-analog"
    source: acme-private
    behavior:
      primitive: analog_source
"#,
    )
    .expect("raw browser-style manifest must deserialize before the engine validates it");
    let mut bus = SystemBus::new();
    let _cpu = configure_xtensa_esp32(&mut bus);

    let err = attach_esp32_external_devices(&mut bus, &manifest)
        .expect_err("direct Xtensa attachment must preflight every carried part pack");
    assert!(
        format!("{err:#}").contains("behavior.analog"),
        "the direct attach error must identify the invalid runtime descriptor: {err:#}"
    );
}

/// A manifest is a complete portable catalog, not just a bag of descriptors
/// that happen to be referenced today. Otherwise a bad private leaf is saved
/// successfully and fails only when a future canvas happens to use it.
#[test]
fn every_manifest_pack_is_runtime_validated_even_when_unused() {
    let src = r#"
schema_version: "1.0"
name: "unused-invalid-pack"
chip: "esp32c3"
external_devices: []
parts:
  - schema: labwired.part/v1
    type: "acme:invalid-analog"
    source: acme-private
    behavior:
      primitive: analog_source
"#;

    let msg = build_error(
        src,
        "an unused malformed part pack must not silently enter a runnable manifest",
    );
    assert!(
        msg.contains("behavior.analog"),
        "the runtime error must identify the missing analog model, got: {msg}"
    );
}

#[test]
fn unused_analog_part_packs_require_exactly_one_input_channel() {
    let src = r#"
schema_version: "1.0"
name: "unused-invalid-analog-inputs"
chip: "esp32c3"
external_devices: []
parts:
  - schema: labwired.part/v1
    type: "acme:two-input-analog"
    source: acme-private
    behavior:
      primitive: analog_source
      analog:
        curve: [[0, 0], [100, 3300]]
    metadata:
      inputs:
        - { key: first, label: First, unit: "%", min: 0, max: 100 }
        - { key: second, label: Second, unit: "%", min: 0, max: 100 }
"#;

    let msg = build_error(
        src,
        "an analog source with multiple drive channels is ambiguous and must be rejected",
    );
    assert!(
        msg.contains("exactly one input channel") && msg.contains("2"),
        "the runtime error must identify the invalid analog input count, got: {msg}"
    );
}

#[test]
fn unused_spi_part_packs_run_the_same_semantic_validation_as_attached_ones() {
    let src = r#"
schema_version: "1.0"
name: "unused-invalid-spi-pack"
chip: "esp32c3"
external_devices: []
parts:
  - schema: labwired.part/v1
    type: "acme:bad-spi"
    source: acme-private
    behavior:
      primitive: spi_device
      spi:
        framing: { command_bytes: 2 }
        registers: []
"#;

    let msg = build_error(
        src,
        "an unused SPI pack must receive the device-level semantic checks",
    );
    assert!(
        msg.contains("behavior.spi declares no registers"),
        "the runtime error must come from the SPI primitive validator, got: {msg}"
    );
}

#[test]
fn unsupported_part_pack_primitives_are_rejected_even_when_unused() {
    let src = r#"
schema_version: "1.0"
name: "unused-unknown-primitive"
chip: "esp32c3"
external_devices: []
parts:
  - schema: labwired.part/v1
    type: "acme:novel-sensor"
    source: acme-private
    behavior:
      primitive: quantum_bus
"#;

    let msg = build_error(
        src,
        "a pack may use an engine primitive, but not invent a silent runtime protocol",
    );
    assert!(
        msg.contains("quantum_bus") && msg.contains("acme:novel-sensor"),
        "the runtime error must identify the unsupported pack primitive, got: {msg}"
    );
}

#[test]
fn unused_gpio_part_packs_validate_their_required_pin_roles() {
    let src = r#"
schema_version: "1.0"
name: "unused-invalid-gpio-pack"
chip: "esp32c3"
external_devices: []
parts:
  - schema: labwired.part/v1
    type: "acme:incomplete-encoder"
    source: acme-private
    behavior:
      primitive: quadrature
      pins:
        a: clk_pin
"#;

    let msg = build_error(
        src,
        "a GPIO primitive missing a required role must fail before a future canvas uses it",
    );
    assert!(
        msg.contains("quadrature") && msg.contains("b"),
        "the runtime error must name the missing quadrature role, got: {msg}"
    );
}

#[test]
fn unused_part_packs_cannot_implicitly_shadow_a_builtin() {
    let pack = ACME_PACK.replace("\"acme:tmp999\"", "tmp102");
    let mut root: serde_yaml::Mapping = serde_yaml::from_str(
        r#"
schema_version: "1.0"
name: "unused-builtin-shadow"
chip: "esp32c3"
external_devices: []
"#,
    )
    .expect("harness manifest is valid YAML");
    root.insert(
        "parts".into(),
        serde_yaml::Value::Sequence(vec![serde_yaml::from_str(&pack).unwrap()]),
    );
    let src = serde_yaml::to_string(&root).unwrap();

    let msg = build_error(
        &src,
        "a private pack cannot defer its built-in collision until a later canvas uses it",
    );
    assert!(
        msg.contains("shadows a built-in") && msg.contains("overrides: tmp102"),
        "the preflight error must name the explicit override requirement, got: {msg}"
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

/// Analog packs take the same runtime-owned-kit route as SPI packs.  They are
/// not GPIO timing primitives: their kit owns both the ADC source and the
/// `SimInput` channel the browser drives.  A manifest that carries an unknown
/// analogue leaf therefore has to expose that channel after the bus builds.
#[test]
fn an_analog_source_pack_attaches_through_the_kit_door() {
    const ANALOG_PACK: &str = r#"
schema: labwired.part/v1
type: "acme:soil-proxy"
source: acme-private
behavior:
  primitive: analog_source
  analog:
    curve:
      - [0, 3300]
      - [100, 0]
metadata:
  inputs:
    - { key: moisture, label: Moisture, unit: "%", min: 0, max: 100, default: 50 }
"#;
    let mut root: serde_yaml::Mapping = serde_yaml::from_str(
        r#"
schema_version: "1.0"
name: "part-pack-analog"
chip: "esp32c3"
external_devices:
  - id: soil
    type: "acme:soil-proxy"
    connection: apb_saradc
    config: { channel: 3 }
"#,
    )
    .unwrap();
    root.insert(
        "parts".into(),
        serde_yaml::Value::Sequence(vec![serde_yaml::from_str(ANALOG_PACK).unwrap()]),
    );
    let src = serde_yaml::to_string(&root).unwrap();

    let mut bus = build_bus(&src).expect("an analog-source pack must build");
    assert!(
        bus.list_inputs()
            .iter()
            .any(|(owner, channel)| owner == "soil" && channel.key == "moisture"),
        "the pack must expose its declared simulator input"
    );
    let idx = bus
        .find_peripheral_index_by_name("apb_saradc")
        .expect("the C3 declares its SAR ADC");
    let adc = bus.peripherals[idx]
        .dev
        .as_any_mut()
        .expect("the SAR ADC is downcastable")
        .downcast_mut::<labwired_core::peripherals::esp32c3::apb_saradc::Esp32c3ApbSarAdc>()
        .expect("the C3 SAR ADC has its production model");
    assert_eq!(
        adc.channel_input_count(3),
        ((1650u32 * 4095) / 3300) as u16,
        "the descriptor default must seed the midpoint of its analog curve, not 0%"
    );
}
