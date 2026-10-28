//! Headline CowManager demo acceptance at the engine layer:
//! one World loads a filesystem YAML chip and a ChipPlugin CPU together.
//!
//! Joost's product path (UI paste + hosted closed K32) sits on top of this.
//! This test locks the core contract so those layers can land as lean PRs.

use labwired_config::{EnvironmentManifest, NodeConfig, PeripheralConfig};
use labwired_core::plugin::{ChipPlugin, PeripheralBuildCtx, PLUGIN_API_VERSION};
use labwired_core::world::World;
use labwired_core::{Peripheral, SimResult};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

const PLUGIN_CHIP: &str = "demo-plugin-cpu";

static PLUGIN_BUILDS: AtomicUsize = AtomicUsize::new(0);

/// Minimal Cortex-M chip YAML served only by the plugin (no file on disk).
const PLUGIN_CHIP_YAML: &str = r#"
name: "demo-plugin-cpu"
arch: "arm"
core: "cortex-m3"
flash: { base: 0x0, size: "128KB" }
ram: { base: 0x20000000, size: "128KB" }
peripherals:
  - { id: plugin_mmio, type: demo_plugin_mmio, base_address: 0x50000000, size: "4KB" }
"#;

/// Filesystem YAML chip a user could author without Rust.
const FILE_CHIP_YAML: &str = r#"
name: "demo-yaml-cpu"
arch: "arm"
core: "cortex-m3"
flash: { base: 0x0, size: "128KB" }
ram: { base: 0x20000000, size: "128KB" }
peripherals: []
"#;

#[derive(Debug)]
struct DemoPluginMmio {
    cell: u8,
}

impl Peripheral for DemoPluginMmio {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(self.cell)
    }

    fn write(&mut self, _offset: u64, value: u8) -> SimResult<()> {
        self.cell = value;
        Ok(())
    }
}

struct DemoPluginCpu;

impl ChipPlugin for DemoPluginCpu {
    fn api_version(&self) -> u32 {
        PLUGIN_API_VERSION
    }

    fn chip_names(&self) -> &[&str] {
        &[PLUGIN_CHIP]
    }

    fn chip_yaml(&self, name: &str) -> Option<&'static str> {
        (name == PLUGIN_CHIP).then_some(PLUGIN_CHIP_YAML)
    }

    fn try_build_peripheral(
        &self,
        ctx: &PeripheralBuildCtx<'_>,
        _p_cfg: &PeripheralConfig,
    ) -> Option<anyhow::Result<Box<dyn Peripheral>>> {
        (ctx.canonical_type == "demo_plugin_mmio").then(|| {
            PLUGIN_BUILDS.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(DemoPluginMmio { cell: 0x5A }) as Box<dyn Peripheral>)
        })
    }
}

fn world_root() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lw-yaml-plus-plugin-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create world root");
    dir
}

fn write_world_root(root: &Path, firmware: &[u8]) {
    let yaml_node = root.join("yaml_node");
    let plugin_node = root.join("plugin_node");
    fs::create_dir_all(&yaml_node).unwrap();
    fs::create_dir_all(&plugin_node).unwrap();

    fs::write(yaml_node.join("chip.yaml"), FILE_CHIP_YAML).unwrap();
    fs::write(
        yaml_node.join("system.yaml"),
        "name: yaml-companion\nchip: chip.yaml\nexternal_devices: []\nboard_io: []\n",
    )
    .unwrap();
    // Plugin node names the plugin chip (no file). No board_io — MMIO proves the plugin.
    fs::write(
        plugin_node.join("system.yaml"),
        format!("name: plugin-cpu\nchip: {PLUGIN_CHIP}\nexternal_devices: []\nboard_io: []\n"),
    )
    .unwrap();

    fs::write(root.join("fw.elf"), firmware).unwrap();
}

/// Removes the temp world root on drop, so it is cleaned up even if the test panics.
struct TempDirGuard(PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn filesystem_yaml_chip_and_plugin_cpu_coexist_in_one_world() {
    PLUGIN_BUILDS.store(0, Ordering::SeqCst);

    let firmware = include_bytes!("../../../tests/fixtures/uart-ok-thumbv7m.elf");
    let dir = world_root();
    let _cleanup = TempDirGuard(dir.clone());
    write_world_root(&dir, firmware);

    let env = EnvironmentManifest {
        schema_version: "1.0".into(),
        name: "yaml-plus-plugin".into(),
        nodes: vec![
            NodeConfig {
                id: "yaml".into(),
                system: "yaml_node/system.yaml".into(),
                firmware: "fw.elf".into(),
                config_overrides: HashMap::new(),
                profile: None,
            },
            NodeConfig {
                id: "plugin".into(),
                system: "plugin_node/system.yaml".into(),
                firmware: "fw.elf".into(),
                config_overrides: HashMap::new(),
                profile: None,
            },
        ],
        interconnects: vec![],
        rf: None,
    };

    let plugin = DemoPluginCpu;
    let plugins: [&dyn ChipPlugin; 1] = [&plugin];

    // Without plugins the plugin node cannot resolve — filesystem-only path.
    let without = World::from_manifest(env.clone(), &dir);
    assert!(
        without.is_err(),
        "plugin chip must not resolve from filesystem alone"
    );

    let mut world = World::from_manifest_with_plugins(env, &dir, &plugins)
        .expect("YAML filesystem chip + plugin CPU build one world");

    assert_eq!(world.machines.len(), 2, "both nodes must be present");
    assert!(world.machines.contains_key("yaml"));
    assert!(world.machines.contains_key("plugin"));
    assert!(
        PLUGIN_BUILDS.load(Ordering::SeqCst) >= 1,
        "plugin must have built its MMIO peripheral for the plugin CPU node"
    );

    let plugin_node = world.machines.get_mut("plugin").expect("plugin node");
    assert_eq!(
        plugin_node.read_u8(0x5000_0000).expect("plugin MMIO read"),
        0x5A,
        "plugin CPU node must expose plugin-owned peripheral MMIO"
    );
    plugin_node
        .write_u8(0x5000_0000, 0xA5)
        .expect("plugin MMIO write");
    assert_eq!(
        plugin_node
            .read_u8(0x5000_0000)
            .expect("plugin MMIO reread"),
        0xA5,
        "plugin MMIO must round-trip through the plugin peripheral"
    );

    for _ in 0..64 {
        let results = world.step_all();
        assert!(
            results.values().all(|r| r.is_ok()),
            "both nodes must step: {results:?}"
        );
    }
}
