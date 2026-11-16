use super::*;
use std::path::Path;

#[test]
fn a_bare_name_resolves_to_the_bundled_descriptor() {
    let chip = ChipDescriptor::resolve("stm32f103", Path::new("/nonexistent"))
        .expect("built-in chip resolves without touching the filesystem");
    assert_eq!(chip.name, "stm32f103c8");
}

/// Guards the drift between `BUILTIN_CHIP_NAMES` and the `include_str!`
/// arms: a name advertised in an error message must actually load.
#[test]
fn every_advertised_builtin_name_loads_and_parses() {
    for name in BUILTIN_CHIP_NAMES {
        ChipDescriptor::resolve(name, Path::new("/nonexistent"))
            .unwrap_or_else(|e| panic!("built-in chip '{name}' failed to load: {e:#}"));
    }
}

#[test]
fn a_path_still_resolves_relative_to_the_manifest() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("custom.yaml"),
        embedded_chip_yaml("stm32f103").unwrap(),
    )
    .unwrap();
    let chip = ChipDescriptor::resolve("./custom.yaml", dir.path()).unwrap();
    assert_eq!(chip.name, "stm32f103c8");
}

#[test]
fn an_unknown_builtin_names_the_available_ones() {
    let err = ChipDescriptor::resolve("stm32f999", Path::new(".")).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("unknown built-in chip 'stm32f999'"), "{msg}");
    assert!(msg.contains("stm32f103"), "{msg}");
}

#[test]
fn a_yaml_extension_is_treated_as_a_path_not_a_builtin() {
    assert!(!is_builtin_chip_spec("stm32f103.yaml"));
    assert!(!is_builtin_chip_spec("chips/stm32f103"));
    assert!(is_builtin_chip_spec("stm32f103"));
}

#[test]
fn resolve_with_falls_back_to_plugin_chips() {
    let yaml = "name: \"secret1\"\narch: \"arm\"\ncore: \"cortex-m0+\"\n\
                    flash: { base: 0, size: \"4KB\" }\n\
                    ram: { base: 0x20000000, size: \"1KB\" }\nperipherals: []\n";
    let d = ChipDescriptor::resolve_with("secret1", Path::new("."), &|name| {
        (name == "secret1").then_some(yaml)
    })
    .unwrap();
    assert_eq!(d.name, "secret1");
}

#[test]
fn resolve_with_prefers_builtins_over_plugin_chips() {
    // A complete, valid descriptor: if precedence inverted, this parses and
    // the assert below fails on the name — not on a parse panic.
    let impostor = "name: \"impostor\"\narch: \"arm\"\ncore: \"cortex-m0+\"\n\
                        flash: { base: 0, size: \"4KB\" }\n\
                        ram: { base: 0x20000000, size: \"1KB\" }\nperipherals: []\n";
    let d = ChipDescriptor::resolve_with("stm32f103", Path::new("."), &|_| Some(impostor)).unwrap();
    assert_eq!(d.name, "stm32f103c8");
}

#[test]
fn resolve_with_keeps_the_unknown_chip_error_without_a_plugin_match() {
    let err = ChipDescriptor::resolve_with("stm32f999", Path::new("."), &|_| None).unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("unknown built-in chip 'stm32f999'"), "{msg}");
    assert!(msg.contains("stm32f103"), "{msg}");
}

#[test]
fn resolve_with_still_loads_paths_from_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("custom.yaml"),
        embedded_chip_yaml("stm32f103").unwrap(),
    )
    .unwrap();
    let chip = ChipDescriptor::resolve_with("./custom.yaml", dir.path(), &|_| None).unwrap();
    assert_eq!(chip.name, "stm32f103c8");
}

// The `MOVED_CHIP_NAMES` tombstone branch in `resolve_with` is exercised
// once the first chip migrates to the private `labwired-ip` repo; until
// then the list is empty and there is nothing to resolve against.
#[test]
fn no_chips_have_migrated_yet() {
    assert!(MOVED_CHIP_NAMES.is_empty());
}

fn script_with_inputs(inputs: &str) -> Result<TestScript> {
    let yaml = format!(
        "schema_version: \"1.2\"\ninputs:\n{inputs}limits:\n  max_steps: 10\nassertions: []\n"
    );
    let script: TestScript = serde_yaml::from_str(&yaml)?;
    script.validate()?;
    Ok(script)
}

#[test]
fn chip_alone_is_accepted() {
    let script = script_with_inputs("  firmware: \"fw.elf\"\n  chip: \"stm32f103\"\n").unwrap();
    assert_eq!(script.inputs.chip.as_deref(), Some("stm32f103"));
    assert!(script.inputs.system.is_none());
}

#[test]
fn chip_and_system_together_are_rejected() {
    let err = script_with_inputs(
        "  firmware: \"fw.elf\"\n  chip: \"stm32f103\"\n  system: \"./system.yaml\"\n",
    )
    .unwrap_err();
    assert!(format!("{err:#}").contains("not both"), "{err:#}");
}

#[test]
fn a_path_in_inputs_chip_is_rejected() {
    let err = script_with_inputs("  firmware: \"fw.elf\"\n  chip: \"./chip.yaml\"\n").unwrap_err();
    assert!(format!("{err:#}").contains("built-in chip name"), "{err:#}");
}

#[test]
fn an_unknown_chip_in_inputs_is_rejected_before_the_run() {
    let err = script_with_inputs("  firmware: \"fw.elf\"\n  chip: \"stm32f999\"\n").unwrap_err();
    assert!(
        format!("{err:#}").contains("unknown built-in chip"),
        "{err:#}"
    );
}

#[test]
fn the_synthetic_manifest_has_nothing_attached() {
    let m = ResolvedSystem::from_builtin_chip("stm32f103")
        .unwrap()
        .manifest;
    assert_eq!(m.chip, "stm32f103");
    assert!(m.external_devices.is_empty());
    assert!(m.board_io.is_empty());
    assert!(!m.schema_version.is_empty());
}

/// A chip resolved by bare name is the descriptor its file loads by path,
/// including a package variant that `include:`s its base die (build.rs
/// flattens those before embedding). Before that, a built-in `include:` was
/// parsed without expansion and silently lost the base's peripherals.
#[test]
fn every_builtin_resolves_to_the_same_descriptor_as_its_file() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips");
    for name in BUILTIN_CHIP_NAMES {
        let by_name = ChipDescriptor::resolve(name, Path::new("/nonexistent"))
            .unwrap_or_else(|e| panic!("built-in '{name}': {e:#}"));
        let by_path = ChipDescriptor::from_file(dir.join(format!("{name}.yaml")))
            .unwrap_or_else(|e| panic!("file '{name}': {e:#}"));
        assert!(
            by_name.include.is_none(),
            "{name}: include survived expansion"
        );
        assert_eq!(
            serde_json::to_value(&by_name).unwrap(),
            serde_json::to_value(&by_path).unwrap(),
            "built-in '{name}' differs from configs/chips/{name}.yaml"
        );
    }
}

/// The include overlay: base keys survive, local scalars win, `peripherals`
/// replace by `id` (keeping the base order) and `pins` merge by label.
#[test]
fn an_included_base_is_overlaid_not_replaced() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("base.yaml"),
        r#"name: base
arch: arm
core: cortex-m33
cpu_hz: 1000
flash: { base: 0x10000000, size: "4096KB" }
ram: { base: 0x20000000, size: "8KB" }
pins:
  P0: { gpio: sio, bit: 0 }
peripherals:
  - { id: a, type: stub, base_address: 0x1000, size: 4096 }
  - { id: b, type: stub, base_address: 0x2000, size: "4KB", config: { profile: x } }
"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("variant.yaml"),
        r#"include: base.yaml
name: variant
flash: { base: 0x10000000, size: "2048KB" }
pins:
  P1: { gpio: sio, bit: 33 }
peripherals:
  - { id: b, type: stub, base_address: 0x2000, size: "4KB", config: { profile: y } }
"#,
    )
    .unwrap();
    let chip = ChipDescriptor::from_file(dir.path().join("variant.yaml")).unwrap();
    assert_eq!(chip.name, "variant");
    assert_eq!(chip.core.as_deref(), Some("cortex-m33"));
    assert_eq!(chip.cpu_hz, 1000);
    assert_eq!(chip.flash.size, 2048 * 1024);
    assert_eq!(chip.pins.len(), 2);
    assert_eq!(chip.pins["P1"].bit, 33);
    let ids: Vec<_> = chip.peripherals.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["a", "b"]);
    assert_eq!(
        chip.peripherals[1]
            .config
            .get("profile")
            .and_then(|v| v.as_str()),
        Some("y")
    );
}

/// Text consumers (the CLI's `run` arch dispatch) get the whole chip for a
/// variant, not its overlay: the flattened text names the base's arch and
/// parses to the same descriptor `from_file` builds.
#[test]
fn flattened_yaml_of_a_variant_is_the_whole_chip() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/chips/rp2354b.yaml");
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("include:") && !raw.contains("arch:"));
    let text = ChipDescriptor::flattened_yaml(&path).unwrap();
    assert!(text.contains("arch: arm"), "{text}");
    let parsed: ChipDescriptor = serde_yaml::from_str(&text).unwrap();
    assert_eq!(
        serde_json::to_value(&parsed).unwrap(),
        serde_json::to_value(ChipDescriptor::from_file(&path).unwrap()).unwrap()
    );
    // A self-contained chip comes back verbatim, comments and all.
    let base = path.with_file_name("rp2350.yaml");
    assert_eq!(
        ChipDescriptor::flattened_yaml(&base).unwrap(),
        std::fs::read_to_string(&base).unwrap()
    );
}
