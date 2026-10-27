use labwired_config::{ChipDescriptor, PeripheralConfig, SystemManifest};
use labwired_core::plugin::{ChipPlugin, PeripheralBuildCtx, PLUGIN_API_VERSION};
use labwired_core::system::builder::{
    build_machine, build_machine_with_plugins, BlobMap, BootMode, BuildOptions, BuildRequest,
    FirmwareSource,
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
        matches!(
            ctx.canonical_type,
            "test_plugin_peripheral" | "test_plugin_manifest_peripheral"
        )
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
    assert_eq!(
        built
            .machine
            .read_memory(0x4000_0000, 1)
            .ok()
            .and_then(|bytes| bytes.first().copied()),
        Some(0xA5),
        "machine MMIO reads use the plugin-owned peripheral"
    );
}

fn xtensa_chip_with_plugin_peripheral(chip_name: &str) -> ChipDescriptor {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/chips")
        .join(format!("{chip_name}.yaml"));
    let mut chip = ChipDescriptor::from_file(path).expect("built-in Xtensa chip descriptor loads");
    chip.peripherals.push(PeripheralConfig {
        id: "plugin_chip_declared".to_string(),
        r#type: "test_plugin_peripheral".to_string(),
        base_address: 0x7000_0000,
        size: Some("4KB".to_string()),
        irq: None,
        irq_controller: None,
        clock: None,
        config: Default::default(),
    });
    chip
}

#[test]
fn plugin_peripherals_attach_through_both_specialized_xtensa_builders() {
    let plugin = TestChipPlugin;
    let plugins: [&dyn ChipPlugin; 1] = [&plugin];

    for (chip_name, firmware) in [
        (
            "esp32",
            &include_bytes!("../../../tests/fixtures/tier1/esp32.elf")[..],
        ),
        (
            "esp32s3",
            &include_bytes!("../../../tests/fixtures/tier1/esp32s3.elf")[..],
        ),
    ] {
        let chip = xtensa_chip_with_plugin_peripheral(chip_name);
        let manifest = SystemManifest::from_yaml(&format!(
            "name: plugin {chip_name} board\nchip: {chip_name}\nperipherals:\n  - id: plugin_manifest_declared\n    type: test_plugin_manifest_peripheral\n    base_address: 0x70001000\n    size: 4KB\n"
        ))
        .expect("Xtensa board manifest parses");
        let blobs = BlobMap::new();

        let without_plugins = build_machine(BuildRequest {
            chip: &chip,
            system: &manifest,
            firmware: FirmwareSource::Elf(firmware),
            boot: BootMode::FastBoot,
            blobs: &blobs,
            options: BuildOptions::default(),
        })
        .expect("specialized Xtensa machine builds without plugins");
        assert_ne!(
            without_plugins
                .machine
                .read_memory(0x7000_0000, 1)
                .ok()
                .and_then(|bytes| bytes.first().copied()),
            Some(0xA5),
            "empty plugin registry leaves {chip_name} plugin window unclaimed"
        );

        let with_plugins = build_machine_with_plugins(
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
        .expect("specialized Xtensa machine builds with plugins");

        assert_eq!(
            with_plugins
                .machine
                .read_memory(0x7000_0000, 1)
                .ok()
                .and_then(|bytes| bytes.first().copied()),
            Some(0xA5),
            "chip-descriptor plugin peripheral attaches on {chip_name}"
        );
        assert_eq!(
            with_plugins
                .machine
                .read_memory(0x7000_1000, 1)
                .ok()
                .and_then(|bytes| bytes.first().copied()),
            Some(0xA5),
            "manifest plugin peripheral attaches on {chip_name}"
        );
        let uart_name = if chip_name == "esp32" {
            "uart0"
        } else {
            "uart0_s3"
        };
        assert!(
            with_plugins
                .machine
                .get_peripherals()
                .iter()
                .any(|(name, _, _)| name == uart_name),
            "plugin attachment preserves the hardwired {chip_name} UART bank"
        );
    }
}
