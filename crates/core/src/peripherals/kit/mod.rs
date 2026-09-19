// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Unified peripheral contract.
//!
//! Every external (off-chip) device the simulator supports — a sensor on
//! I2C, a display on SPI, a modem on UART — historically lived in three or
//! four places at once: a Rust model, a hand-written arm in `bus/mod.rs`
//! that wired it into a system.yaml, a TypeScript SVG component, and a
//! scattering of palette/library metadata. The result was a 6-step ritual
//! per peripheral that was easy to half-finish, and impossible for tooling
//! to verify.
//!
//! `PeripheralKit` collapses that ritual to one trait. A peripheral
//! implements [`PeripheralKit`], registers a single `&'static` instance in
//! [`registry::kits`], and gets:
//!   * automatic dispatch from `bus/mod.rs` (the legacy hand-written arm
//!     can be deleted),
//!   * an entry in the offline-generated `peripherals-manifest.json` that
//!     the browser/playground consumes,
//!   * coverage by the [`peripheral_kit_gate`](../../../../tests/peripheral_kit_gate.rs)
//!     test that fails CI if any surface (tests, manifest entry, lab,
//!     UI component) is missing.
//!
//! The trait is shaped CI-first. Browser concerns — palette icons, library
//! tile copy, demo-firmware paths — live in [`KitMetadata`] as pure data
//! exposed from the trait, not in the trait surface itself. That keeps the
//! kit usable from headless CI / wasm builds where no UI exists.

mod ctx;
pub mod declarative;
pub mod registry;

pub use ctx::AttachCtx;
pub use declarative::DeclarativeDeviceKit;

use anyhow::Result;

/// What every external peripheral must provide.
///
/// Built-in implementations have static registry owners. Runtime declarative
/// kits own their descriptor and metadata and can be dropped after attachment;
/// the attached model owns everything it needs to run.
pub trait PeripheralKit: Send + Sync {
    /// Metadata borrowed from this kit: device_type, label, transport, config keys,
    /// associated lab. Consumed by the manifest generator and by tooling
    /// that wants to introspect what's available.
    fn metadata(&self) -> &KitMetadata;

    /// Construct the model and attach it to the bus. `ctx` carries the
    /// `ExternalDevice` config from system.yaml plus typed accessors that
    /// handle the connection lookup / downcast / config-parsing boilerplate.
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()>;
}

/// Pure-data peripheral description. Serialised verbatim into
/// `peripherals-manifest.json`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct KitMetadata {
    /// String used as `type:` in system.yaml `external_devices` entries.
    /// Must be unique across all kits.
    pub device_type: std::borrow::Cow<'static, str>,
    /// Display label shown in the playground library tile / chip row.
    pub label: std::borrow::Cow<'static, str>,
    /// One-line summary for the library tile.
    pub summary: std::borrow::Cow<'static, str>,
    /// Long-form description shown in the library detail view.
    pub detail: std::borrow::Cow<'static, str>,
    /// The bus transport this peripheral attaches to.
    pub transport: Transport,
    /// Palette grouping for the playground command palette / icon fallback.
    pub category: Category,
    /// Config keys this peripheral accepts in `config:` under its system.yaml
    /// `external_devices` entry. Used for docs + manifest schema; not (yet)
    /// enforced at parse time.
    pub config_keys: std::borrow::Cow<'static, [ConfigKey]>,
    /// Starter labs that ship a one-click demo using this peripheral. A
    /// peripheral may appear in zero, one, or several labs (e.g. the same
    /// e-paper model used in both an STM32 and ESP32 example). The first
    /// entry is treated as the primary lab by tooling that wants a default.
    pub labs: std::borrow::Cow<'static, [LabRef]>,
    /// Drivable input channels this device accepts at runtime through the
    /// generic stimulus API ([`crate::sim_input::SimInput`] →
    /// `Machine::set_input` → test-script `stimuli:` / MCP `run_lab`). Part
    /// of the device schema: the same channel definitions back the device's
    /// `SimInput` impl, so the manifest cannot advertise channels the model
    /// doesn't serve. Empty = not an input device.
    pub inputs: std::borrow::Cow<'static, [crate::sim_input::InputChannel]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Transport {
    Uart,
    I2c,
    Spi,
    Analog,
    GpioGroup,
    /// Host-side CAN tool attached to a named bxCAN/FDCAN peripheral
    /// (`connection:` is the controller id, not a pad bus).
    Can,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Uart,
    I2c,
    Spi,
    Analog,
    Gpio,
    Misc,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConfigKey {
    pub name: std::borrow::Cow<'static, str>,
    pub ty: ConfigType,
    pub doc: std::borrow::Cow<'static, str>,
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigType {
    Str,
    Int,
    Bool,
    Float,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LabRef {
    /// `boardId` in `BOARD_CONFIGS` (e.g. `"quectel-bg770a-lab"`).
    pub board_id: std::borrow::Cow<'static, str>,
    /// Chip identifier the lab runs against (e.g. `"stm32f103"`).
    pub chip: std::borrow::Cow<'static, str>,
    /// Path under `core/examples/` containing the firmware + system.yaml.
    pub example_dir: std::borrow::Cow<'static, str>,
    /// Demo ELF filename under `packages/playground/public/wasm/`.
    pub demo_elf: std::borrow::Cow<'static, str>,
}
