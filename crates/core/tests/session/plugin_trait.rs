use labwired_config::{ChipDescriptor, PeripheralConfig, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::plugin::{ChipPlugin, PeripheralBuildCtx, PLUGIN_API_VERSION};
use labwired_core::{Bus, Peripheral};

struct EmptyPlugin;

impl ChipPlugin for EmptyPlugin {
    fn api_version(&self) -> u32 {
        PLUGIN_API_VERSION
    }
}

#[test]
fn default_impls_claim_nothing() {
    let p = EmptyPlugin;
    assert_eq!(p.api_version(), PLUGIN_API_VERSION);
    assert!(p.chip_names().is_empty());
    assert!(p.chip_yaml("anything").is_none());
    assert!(p.embedded_descriptor("anything/x.yaml").is_none());
}

#[derive(Debug)]
struct ConstPeripheral;

impl Peripheral for ConstPeripheral {
    fn read(&self, _offset: u64) -> labwired_core::SimResult<u8> {
        Ok(0xA5)
    }
    fn write(&mut self, _offset: u64, _value: u8) -> labwired_core::SimResult<()> {
        Ok(())
    }
}

struct MockPlugin;

impl ChipPlugin for MockPlugin {
    fn api_version(&self) -> u32 {
        PLUGIN_API_VERSION
    }
    fn try_build_peripheral(
        &self,
        ctx: &PeripheralBuildCtx<'_>,
        _p_cfg: &PeripheralConfig,
    ) -> Option<anyhow::Result<Box<dyn Peripheral>>> {
        (ctx.canonical_type == "mock_const")
            .then(|| Ok(Box::new(ConstPeripheral) as Box<dyn Peripheral>))
    }
}

struct FailingPlugin;

impl ChipPlugin for FailingPlugin {
    fn api_version(&self) -> u32 {
        PLUGIN_API_VERSION
    }
    fn try_build_peripheral(
        &self,
        ctx: &PeripheralBuildCtx<'_>,
        _p_cfg: &PeripheralConfig,
    ) -> Option<anyhow::Result<Box<dyn Peripheral>>> {
        (ctx.canonical_type == "mock_const")
            .then(|| Err(anyhow::anyhow!("mock plugin build failure")))
    }
}

const MOCK_CHIP_YAML: &str = r#"
name: "mockchip"
arch: "arm"
core: "cortex-m0+"
flash: { base: 0x0, size: "64KB" }
ram: { base: 0x20000000, size: "4KB" }
peripherals:
  - { id: mock0, type: mock_const, base_address: 0x40000000 }
"#;

fn mock_chip() -> ChipDescriptor {
    serde_yaml::from_str(MOCK_CHIP_YAML).expect("mock chip yaml parses")
}

fn mock_manifest() -> SystemManifest {
    SystemManifest::from_yaml("name: mock\nchip: unused\n").expect("mock manifest parses")
}

#[test]
fn plugin_peripheral_is_built_and_readable_over_mmio() {
    let chip = mock_chip();
    let manifest = mock_manifest();
    let plugins: [&dyn ChipPlugin; 1] = [&MockPlugin];
    let bus = SystemBus::from_config_with_plugins(&chip, &manifest, &plugins).expect("bus builds");
    assert_eq!(bus.read_u8(0x4000_0000).ok(), Some(0xA5));
}

#[test]
fn plugin_error_propagates_instead_of_falling_through() {
    let chip = mock_chip();
    let manifest = mock_manifest();
    let plugins: [&dyn ChipPlugin; 1] = [&FailingPlugin];
    let result = SystemBus::from_config_with_plugins(&chip, &manifest, &plugins);
    let err = match result {
        Ok(_) => panic!("plugin Some(Err) must fail the bus build"),
        Err(e) => e,
    };
    // `from_config_with_plugins` wraps the plugin's error in peripheral
    // context, so the top-level message is the context line; the `{:#}`
    // alternate format walks the whole chain.
    let chain = format!("{err:#}");
    assert!(
        chain.contains("mock plugin build failure"),
        "expected the plugin's own error in the chain, got: {chain}"
    );
    assert!(
        chain.contains("mock0"),
        "expected the peripheral id in the context, got: {chain}"
    );
}

struct SystickClaimingPlugin;

impl ChipPlugin for SystickClaimingPlugin {
    fn api_version(&self) -> u32 {
        PLUGIN_API_VERSION
    }
    fn try_build_peripheral(
        &self,
        ctx: &PeripheralBuildCtx<'_>,
        _p_cfg: &PeripheralConfig,
    ) -> Option<anyhow::Result<Box<dyn Peripheral>>> {
        (ctx.canonical_type == "systick")
            .then(|| Ok(Box::new(ConstPeripheral) as Box<dyn Peripheral>))
    }
}

#[test]
fn plugin_wins_over_in_tree_factory() {
    // `systick` is built unconditionally by the in-tree generic factory
    // (`peripherals::generic_factory::try_build`), so a plugin claiming it
    // only answers when plugin dispatch runs FIRST.
    let chip_yaml = r#"
name: "mockchip"
arch: "arm"
core: "cortex-m0+"
flash: { base: 0x0, size: "64KB" }
ram: { base: 0x20000000, size: "4KB" }
peripherals:
  - { id: systick0, type: systick, base_address: 0xE000E010 }
"#;
    let chip: ChipDescriptor = serde_yaml::from_str(chip_yaml).expect("chip yaml parses");
    let manifest = mock_manifest();

    // Guard against a vacuous pass: without the plugin the in-tree model
    // answers, and it does not return the plugin's marker byte.
    let plain = SystemBus::from_config_with_plugins(&chip, &manifest, &[]).expect("bus builds");
    assert_ne!(
        plain.read_u8(0xE000_E010).ok(),
        Some(0xA5),
        "fixture is vacuous: in-tree systick already reads 0xA5"
    );

    let plugins: [&dyn ChipPlugin; 1] = [&SystickClaimingPlugin];
    let bus = SystemBus::from_config_with_plugins(&chip, &manifest, &plugins).expect("bus builds");
    assert_eq!(bus.read_u8(0xE000_E010).ok(), Some(0xA5));
}
