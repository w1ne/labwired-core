use labwired_config::{ChipDescriptor, PeripheralConfig, SystemManifest};
use labwired_core::plugin::{ChipPlugin, PeripheralBuildCtx, PLUGIN_API_VERSION};
use labwired_core::system::builder::{
    build_machine_with_plugins, BlobMap, BootMode, BuildOptions, BuildRequest, FirmwareSource,
};
use labwired_core::{Peripheral, SimResult};
use std::path::Path;

const PLUGIN_CHIP: &str = "test-plugin-cortex-m";
const CHIP_YAML: &str = r#"
name: "test-plugin-cortex-m"
arch: "arm"
core: "cortex-m0+"
flash: { base: 0x0, size: "64KB" }
ram: { base: 0x20000000, size: "4KB" }
peripherals:
  - { id: plugin_gpio, type: test_plugin_peripheral, base_address: 0x40000000 }
"#;

#[derive(Debug)]
struct PluginPeripheral;

impl Peripheral for PluginPeripheral {
    fn read(&self, _offset: u64) -> SimResult<u8> {
        Ok(0xA5)
    }

    fn write(&mut self, _offset: u64, _value: u8) -> SimResult<()> {
        Ok(())
    }
}

struct TestChipPlugin;

impl ChipPlugin for TestChipPlugin {
    fn api_version(&self) -> u32 {
        PLUGIN_API_VERSION
    }

    fn chip_names(&self) -> &[&str] {
        &[PLUGIN_CHIP]
    }

    fn chip_yaml(&self, name: &str) -> Option<&'static str> {
        (name == PLUGIN_CHIP).then_some(CHIP_YAML)
    }

    fn try_build_peripheral(
        &self,
        ctx: &PeripheralBuildCtx<'_>,
        _p_cfg: &PeripheralConfig,
    ) -> Option<anyhow::Result<Box<dyn Peripheral>>> {
        (ctx.canonical_type == "test_plugin_peripheral")
            .then(|| Ok(Box::new(PluginPeripheral) as Box<dyn Peripheral>))
    }
}

#[test]
fn plugin_chip_builds_with_plugin_registry() {
    let plugin = TestChipPlugin;
    let plugins: [&dyn ChipPlugin; 1] = [&plugin];
    let chip = ChipDescriptor::resolve_with(PLUGIN_CHIP, Path::new("."), &|name| {
        plugins.iter().find_map(|plugin| plugin.chip_yaml(name))
    })
    .expect("plugin chip descriptor resolves");
    let manifest = SystemManifest::from_yaml(
        r#"
name: test plugin board
chip: test-plugin-cortex-m
board_io:
  - id: plugin-led
    kind: led
    peripheral: plugin_gpio
    pin: 3
    signal: output
"#,
    )
    .expect("manifest parses");
    let firmware = include_bytes!("../../../tests/fixtures/uart-ok-thumbv7m.elf");
    let blobs = BlobMap::new();

    let built = build_machine_with_plugins(
        BuildRequest {
            chip: &chip,
            system: &manifest,
            firmware: FirmwareSource::Elf(firmware),
            boot: BootMode::FastBoot,
            blobs: &blobs,
            options: BuildOptions::default(),
        },
        &plugins,
    )
    .expect("plugin chip machine builds");

    assert_eq!(built.arch, labwired_config::Arch::Arm);
    assert_eq!(built.board_io.len(), 1);
    assert_eq!(built.board_io[0].id, "plugin-led");
}
