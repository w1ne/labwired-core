//! Compile-time plugin interface for out-of-tree chip IP.
//!
//! The open core ships the common chip catalog. Special chips whose models
//! are proprietary live in the private `labwired-ip` repo and are linked in
//! through this trait by the `labwired-pro` binary.

use labwired_config::{PeripheralConfig, SystemManifest};

use crate::bus::bus_trace::BusTrace;
use crate::peripherals::chip_map::ChipMap;
use crate::Peripheral;

/// Version of the plugin interface. Bump on any breaking change to
/// `ChipPlugin` or the types it exposes; `labwired_cli::run_with_plugins`
/// refuses plugins whose `api_version()` does not match.
pub const PLUGIN_API_VERSION: u32 = 1;

/// Inputs a plugin's peripheral factory receives — the union of what the
/// in-tree family factories get today.
pub struct PeripheralBuildCtx<'a> {
    pub canonical_type: &'a str,
    pub manifest: &'a SystemManifest,
    pub bus_trace: &'a BusTrace,
    pub chip_map: ChipMap<'a>,
}

/// A bundle of out-of-tree chips: behavioral peripheral models plus the
/// embedded YAMLs that let `chip: "<name>"` resolve without the filesystem.
pub trait ChipPlugin {
    /// Must return [`PLUGIN_API_VERSION`] of the core the plugin was built against.
    fn api_version(&self) -> u32;

    /// Built-in-style chip names this plugin provides.
    fn chip_names(&self) -> &[&str] {
        &[]
    }

    /// Embedded chip descriptor YAML for a name from [`Self::chip_names`].
    fn chip_yaml(&self, name: &str) -> Option<&'static str> {
        let _ = name;
        None
    }

    /// Embedded peripheral descriptor (debug schema / declarative register
    /// map), keyed by the chip-relative path used in chip YAMLs, e.g.
    /// `"mkw41z4/rsim.yaml"`.
    fn embedded_descriptor(&self, key: &str) -> Option<&'static str> {
        let _ = key;
        None
    }

    /// Try to build a peripheral. The three-way return distinguishes:
    /// - `None`: "not my peripheral type" — fall through to the open
    ///   families, the generic factory, and the declarative loaders;
    /// - `Some(Err(_))`: "mine, but construction failed" — must propagate
    ///   as an error instead of falling through to a misleading
    ///   unknown-type error;
    /// - `Some(Ok(_))`: a successfully built device.
    fn try_build_peripheral(
        &self,
        ctx: &PeripheralBuildCtx<'_>,
        p_cfg: &PeripheralConfig,
    ) -> Option<anyhow::Result<Box<dyn Peripheral>>> {
        let _ = (ctx, p_cfg);
        None
    }
}
