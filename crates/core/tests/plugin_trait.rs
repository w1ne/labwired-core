use labwired_core::plugin::{ChipPlugin, PeripheralBuildCtx, PLUGIN_API_VERSION};

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
