use super::*;

/// `SystemManifest::from_file` inlines a `can-player` device's `path:`
/// (resolved relative to the system yaml on disk) into `data:`, so the
/// sim core itself only ever consumes inline text (keeps `std::fs` out
/// of the wasm-safe core).
#[test]
fn from_file_inlines_can_player_path_into_data() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("s.log"), "(1.0) can0 123#11\n").unwrap();
    let yaml = r#"
name: "t"
chip: "chip.yaml"
external_devices:
  - type: "can-player"
    id: "p"
    connection: "bxcan1"
    config:
      path: "./s.log"
board_io: []
"#;
    let sys_path = dir.path().join("system.yaml");
    std::fs::write(&sys_path, yaml).unwrap();
    let m = SystemManifest::from_file(&sys_path).unwrap();
    let cfg = &m.external_devices[0].config;
    assert!(cfg.get("path").is_none());
    assert_eq!(
        cfg.get("data").unwrap().as_str().unwrap(),
        "(1.0) can0 123#11\n"
    );
}

/// Setting both `path:` and `data:` on a `can-player` device is
/// ambiguous — silently letting `path` overwrite `data` (the prior
/// behavior) hides a config mistake. Must error naming the device id
/// and both keys.
#[test]
fn from_file_errors_when_both_path_and_data_set() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("s.log"), "(1.0) can0 123#11\n").unwrap();
    let yaml = r#"
name: "t"
chip: "chip.yaml"
external_devices:
  - type: "can-player"
    id: "p"
    connection: "bxcan1"
    config:
      path: "./s.log"
      data: "(1.0) can0 123#11\n"
board_io: []
"#;
    let sys_path = dir.path().join("system.yaml");
    std::fs::write(&sys_path, yaml).unwrap();
    let err = SystemManifest::from_file(&sys_path).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("'p'"), "unexpected error: {msg}");
    assert!(msg.contains("path"), "unexpected error: {msg}");
    assert!(msg.contains("data"), "unexpected error: {msg}");
}

/// A `path:` pointing at a file that doesn't exist fails with an error
/// that names the (resolved) path, not just an opaque io::Error.
#[test]
fn from_file_errors_on_nonexistent_path() {
    let dir = tempfile::tempdir().unwrap();
    let yaml = r#"
name: "t"
chip: "chip.yaml"
external_devices:
  - type: "can-player"
    id: "p"
    connection: "bxcan1"
    config:
      path: "./missing.log"
board_io: []
"#;
    let sys_path = dir.path().join("system.yaml");
    std::fs::write(&sys_path, yaml).unwrap();
    let err = SystemManifest::from_file(&sys_path).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("missing.log"), "unexpected error: {msg}");
}

/// A non-string `path:` value fails with an error naming the device id.
#[test]
fn from_file_errors_on_non_string_path() {
    let dir = tempfile::tempdir().unwrap();
    let yaml = r#"
name: "t"
chip: "chip.yaml"
external_devices:
  - type: "can-player"
    id: "p"
    connection: "bxcan1"
    config:
      path: 123
board_io: []
"#;
    let sys_path = dir.path().join("system.yaml");
    std::fs::write(&sys_path, yaml).unwrap();
    let err = SystemManifest::from_file(&sys_path).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("'p'"), "unexpected error: {msg}");
}

#[test]
fn spi_device_descriptor_parses_framing_and_registers() {
    let yaml = r#"
type: test_spi
behavior:
  primitive: spi_device
  spi:
    framing: { command_bytes: 1, rw_bit: 7, rw_read_high: true, addr_mask: 0x3F, auto_increment: true }
    registers:
      - { name: WHOAMI, addr: 0x00, width: 1, endian: le, access: r, reset: 0xE5 }
      - { name: DATA, addr: 0x32, width: 2, endian: le, access: r, source: accel }
metadata:
  inputs:
    - { key: accel, label: "Accel X", unit: g, min: -16, max: 16, default: 0 }
"#;
    let d = DeviceDescriptor::from_yaml(yaml).unwrap();
    let spi = d.behavior.spi.as_ref().expect("behavior.spi present");
    assert_eq!(spi.framing.command_bytes, 1);
    assert_eq!(spi.framing.rw_bit, Some(7));
    assert_eq!(spi.registers.len(), 2);
    assert_eq!(spi.registers[0].reset, 0xE5);
}

#[test]
fn spi_framing_defaults_are_adxl_shaped() {
    let f = SpiFraming::default();
    assert_eq!(f.command_bytes, 1);
    assert_eq!(f.rw_bit, Some(7));
    assert!(f.rw_read_high);
    assert_eq!(f.addr_mask, 0x3F);
    assert_eq!(f.addr_shift, 0);
    assert!(f.auto_increment);
}

#[test]
fn i2c_register_alias_still_names_the_shared_struct() {
    // The rename must not break existing I2c-named references.
    let _r: I2cRegister = RegisterSpec {
        name: "R".into(),
        addr: 0,
        width: 1,
        endian: Endian::Le,
        access: I2cAccess::R,
        write_mask: None,
        reset: 0,
        source: None,
        encode: None,
        scale_from: vec![],
        source_scale: None,
        resolution: None,
        signed: false,
        fields: vec![],
        page: None,
        self_clearing: None,
        popcount: None,
        zero_when: None,
        zero_unless: None,
        source_from: None,
        bits: vec![],
        on_read: None,
        on_write: None,
        calendar: None,
    };
}

#[test]
fn every_spelling_of_a_write_action_deserializes_to_one_value() {
    // The SystemRDL names arrive from three places — a datasheet, an SVD
    // import, a hand-written part — and each spells them differently. They
    // must be ALIASES of one value, never separate variants, or a descriptor
    // that says `one_to_clear` where the engine matches `write_one_to_clear`
    // silently does nothing.
    #[derive(Deserialize)]
    struct W {
        a: WriteAction,
    }
    for spelling in ["write_one_to_clear", "one_to_clear", "oneToClear", "w1c"] {
        let w: W = serde_yaml::from_str(&format!("a: {spelling}")).unwrap();
        assert_eq!(w.a, WriteAction::WriteOneToClear, "{spelling}");
    }
    for spelling in ["write_zero_to_clear", "zero_to_clear", "zeroToClear", "w0c"] {
        let w: W = serde_yaml::from_str(&format!("a: {spelling}")).unwrap();
        assert_eq!(w.a, WriteAction::WriteZeroToClear, "{spelling}");
    }
    for spelling in ["one_to_set", "write_one_to_set", "oneToSet", "w1s"] {
        let w: W = serde_yaml::from_str(&format!("a: {spelling}")).unwrap();
        assert_eq!(w.a, WriteAction::OneToSet, "{spelling}");
    }
    #[derive(Deserialize)]
    struct R {
        a: ReadAction,
    }
    for spelling in ["clear", "read_clear", "readClear", "clear_on_read"] {
        let r: R = serde_yaml::from_str(&format!("a: {spelling}")).unwrap();
        assert_eq!(r.a, ReadAction::Clear, "{spelling}");
    }
}

#[test]
fn a_timing_action_accepts_the_map_form_and_the_tag_form() {
    // serde_yaml 0.9 rejects the single-key map form for an externally tagged
    // enum, which is the form anyone writing a part by hand uses. Both must
    // land on the same value — see `TimingAction`.
    #[derive(Debug, Deserialize)]
    struct T {
        on_fire: Vec<TimingAction>,
    }
    let want = TimingAction::SetBits {
        register: "STATUS".into(),
        bits: 1,
    };
    for yaml in [
        "on_fire:\n  - set_bits: { register: STATUS, bits: 1 }\n",
        "on_fire:\n  - !set_bits { register: STATUS, bits: 1 }\n",
    ] {
        let t: T = serde_yaml::from_str(yaml).unwrap_or_else(|e| panic!("{yaml}: {e}"));
        assert_eq!(t.on_fire, vec![want.clone()]);
    }
    // A variant name nobody defined must be named, not silently dropped.
    let err = serde_yaml::from_str::<T>("on_fire:\n  - nope: { register: STATUS, bits: 1 }\n")
        .expect_err("an unknown timing action must be rejected")
        .to_string();
    assert!(err.contains("nope"), "got: {err}");
}
