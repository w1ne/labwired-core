use labwired_config::{EnvironmentManifest, NodeConfig};
use labwired_core::world::World;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn world_rejects_a_direct_manifest_that_bypasses_file_validation() {
    let manifest = EnvironmentManifest {
        schema_version: "2.0".to_string(),
        name: "invalid-world".to_string(),
        nodes: Vec::new(),
        interconnects: Vec::new(),
    };

    let error = match World::from_manifest(manifest, Path::new(".")) {
        Ok(_) => panic!("World::from_manifest accepted an invalid direct manifest"),
        Err(error) => format!("{error:#}"),
    };

    assert!(error.contains("schema_version '2.0'"), "{error}");
}

#[test]
fn world_rejects_non_cortex_m_nodes_before_loading_firmware() {
    let manifest = EnvironmentManifest {
        schema_version: "1.0".to_string(),
        name: "riscv-world".to_string(),
        nodes: vec![NodeConfig {
            id: "riscv".to_string(),
            system: "configs/systems/ci-fixture-riscv-uart1.yaml".to_string(),
            firmware: "tests/fixtures/riscv-ci-fixture.elf".to_string(),
            config_overrides: HashMap::new(),
        }],
        interconnects: Vec::new(),
    };

    let error = match World::from_manifest(manifest, &repo_root()) {
        Ok(_) => panic!("World::from_manifest accepted a non-Cortex-M node"),
        Err(error) => format!("{error:#}"),
    };

    assert!(
        error.contains("environment worlds currently support only Cortex-M nodes"),
        "{error}"
    );
    assert!(error.contains("RiscV"), "{error}");
}

#[test]
fn world_rejects_riscv_firmware_for_a_cortex_m_node_before_execution() {
    let manifest = EnvironmentManifest {
        schema_version: "1.0".to_string(),
        name: "mismatched-firmware-world".to_string(),
        nodes: vec![NodeConfig {
            id: "h5".to_string(),
            system: "configs/systems/nucleo-h563zi-demo.yaml".to_string(),
            firmware: "tests/fixtures/riscv-ci-fixture.elf".to_string(),
            config_overrides: HashMap::new(),
        }],
        interconnects: Vec::new(),
    };

    let error = match World::from_manifest(manifest, &repo_root()) {
        Ok(_) => panic!("World::from_manifest accepted RISC-V firmware for a Cortex-M node"),
        Err(error) => format!("{error:#}"),
    };

    assert!(
        error.contains("node 'h5': firmware architecture RiscV is incompatible with Cortex-M"),
        "{error}"
    );
}
