use labwired_config::{EnvironmentManifest, NodeConfig};
use labwired_core::world::World;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_WORLD_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TemporaryWorldDir(PathBuf);

impl TemporaryWorldDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "labwired-world-validation-{}-{}",
            std::process::id(),
            TEMP_WORLD_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryWorldDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn write_arm_elf(path: &Path, reset_vector: u32) {
    // Minimal ELF32/EM_ARM executable with one 8-byte PT_LOAD segment at the
    // conventional Cortex-M flash base. The reset handler is caller-controlled
    // so this fixture can prove that an EM_ARM header alone is insufficient.
    let mut bytes = vec![0u8; 84 + 8];
    bytes[0..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 1; // ELFCLASS32
    bytes[5] = 1; // little endian
    bytes[6] = 1; // ELF version
    bytes[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    bytes[18..20].copy_from_slice(&40u16.to_le_bytes()); // EM_ARM
    bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
    bytes[24..28].copy_from_slice(&reset_vector.to_le_bytes());
    bytes[28..32].copy_from_slice(&52u32.to_le_bytes()); // e_phoff
    bytes[36..40].copy_from_slice(&0x0500_0000u32.to_le_bytes()); // EABI v5
    bytes[40..42].copy_from_slice(&52u16.to_le_bytes()); // e_ehsize
    bytes[42..44].copy_from_slice(&32u16.to_le_bytes()); // e_phentsize
    bytes[44..46].copy_from_slice(&1u16.to_le_bytes()); // e_phnum

    let ph = 52;
    bytes[ph..ph + 4].copy_from_slice(&1u32.to_le_bytes()); // PT_LOAD
    bytes[ph + 4..ph + 8].copy_from_slice(&84u32.to_le_bytes());
    bytes[ph + 8..ph + 12].copy_from_slice(&0x0800_0000u32.to_le_bytes());
    bytes[ph + 12..ph + 16].copy_from_slice(&0x0800_0000u32.to_le_bytes());
    bytes[ph + 16..ph + 20].copy_from_slice(&8u32.to_le_bytes());
    bytes[ph + 20..ph + 24].copy_from_slice(&8u32.to_le_bytes());
    bytes[ph + 24..ph + 28].copy_from_slice(&5u32.to_le_bytes());
    bytes[ph + 28..ph + 32].copy_from_slice(&0x1000u32.to_le_bytes());

    bytes[84..88].copy_from_slice(&0x2000_1000u32.to_le_bytes());
    bytes[88..92].copy_from_slice(&reset_vector.to_le_bytes());
    std::fs::write(path, bytes).unwrap();
}

fn temporary_arm_world(
    core: Option<&str>,
    reset_vector: u32,
) -> (TemporaryWorldDir, EnvironmentManifest) {
    let dir = TemporaryWorldDir::new();
    let core = core
        .map(|core| format!("core: {core}\n"))
        .unwrap_or_default();
    std::fs::write(
        dir.path().join("chip.yaml"),
        format!(
            "name: temporary-arm\narch: arm\n{core}flash:\n  base: 0x08000000\n  size: 64KB\nram:\n  base: 0x20000000\n  size: 16KB\nperipherals: []\n"
        ),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("system.yaml"),
        "name: temporary-arm\nchip: chip.yaml\n",
    )
    .unwrap();
    write_arm_elf(&dir.path().join("firmware.elf"), reset_vector);

    (
        dir,
        EnvironmentManifest {
            schema_version: "1.0".to_string(),
            name: "temporary-arm-world".to_string(),
            nodes: vec![NodeConfig {
                id: "node".to_string(),
                system: "system.yaml".to_string(),
                firmware: "firmware.elf".to_string(),
                config_overrides: HashMap::new(),
            }],
            interconnects: Vec::new(),
        },
    )
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
fn world_rejects_non_cortex_m_arm_cores_before_constructing_a_cortex_m_machine() {
    for (name, core) in [("missing-core", None), ("cortex-a9", Some("cortex-a9"))] {
        let (dir, manifest) = temporary_arm_world(core, 0x0800_0009);
        let error = match World::from_manifest(manifest, dir.path()) {
            Ok(_) => panic!("World::from_manifest accepted {name} as Cortex-M"),
            Err(error) => format!("{error:#}"),
        };

        assert!(
            error.contains("requires an explicit Cortex-M core"),
            "{name}: {error}"
        );
    }
}

#[test]
fn world_rejects_an_arm_elf_without_a_cortex_m_thumb_reset_vector() {
    let (dir, manifest) = temporary_arm_world(Some("cortex-m3"), 0x0800_0010);
    let error = match World::from_manifest(manifest, dir.path()) {
        Ok(_) => panic!("World::from_manifest accepted a non-Thumb ARM firmware image"),
        Err(error) => format!("{error:#}"),
    };

    assert!(
        error.contains("does not contain a valid Cortex-M Thumb reset vector"),
        "{error}"
    );
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
