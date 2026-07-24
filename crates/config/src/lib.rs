// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

/// Canonical device config v1 — the single JSON shape the agent builds and both
/// engines load directly (Phase 1: parse + structural validation; resolve() is
/// a documented Phase 2 stub).
pub mod canonical;

fn deserialize_u64_lax<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum IntOrString {
        Int(u64),
        String(String),
    }

    match IntOrString::deserialize(deserializer)? {
        IntOrString::Int(v) => Ok(v),
        IntOrString::String(s) => {
            let s = s.trim();
            if let Some(stripped) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                u64::from_str_radix(&stripped.replace('_', ""), 16)
                    .map_err(serde::de::Error::custom)
            } else {
                s.replace('_', "")
                    .parse::<u64>()
                    .map_err(serde::de::Error::custom)
            }
        }
    }
}

/// Default schema version for YAML configs
fn default_schema_version() -> String {
    "1.0".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    #[serde(alias = "cortex-m3", alias = "cortex-m4", alias = "cortex-m7")]
    Arm,
    #[serde(alias = "riscv32", alias = "rv32i", alias = "rv32imac")]
    RiscV,
    #[serde(alias = "xtensa-lx7", alias = "xtensa-lx6")]
    Xtensa,
    Unknown,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MemoryRange {
    #[serde(deserialize_with = "deserialize_u64_lax")]
    pub base: u64,
    pub size: String, // e.g. "128KB"
}

/// An additional named RAM/ROM-backed memory window beyond the primary
/// `flash`/`ram`. Needed by SoCs that expose several CPU-visible memory windows
/// (e.g. the ESP32-C3's separate IRAM `0x4037C000` and flash-DROM `0x3C000000`
/// views), which real firmware links code/rodata into.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NamedMemoryRange {
    pub name: String,
    #[serde(deserialize_with = "deserialize_u64_lax")]
    pub base: u64,
    pub size: String,
    /// Optional env var naming a path to a raw binary loaded into this region at
    /// `base` (e.g. a chip's mask ROM dump). Used for copyrighted vendor blobs
    /// that can't be committed — the region stays zero-filled if unset/missing.
    #[serde(default)]
    pub image_env: Option<String>,
}

/// Optional RCC clock-gate declaration for a peripheral. When present, the bus
/// models silicon clock-gating: a CPU access to the peripheral only takes effect
/// while `bit` is set in the RCC peripheral's `reg` enable register. Peripherals
/// without a `clock:` field are never gated (safe default — existing configs and
/// firmware that don't enable a clock keep working unchanged).
///
/// `reg` is the symbolic enable-register name (e.g. "apb1enr", "apb2enr",
/// "ahbenr", "ahb2enr"); the bus maps it to the chip family's actual RCC offset
/// at build time, so the same name resolves correctly on F1 vs L4.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClockGate {
    /// Symbolic RCC enable-register name, e.g. "apb1enr" / "apb2enr" / "ahbenr".
    pub reg: String,
    /// Enable-bit position within that register.
    pub bit: u8,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PeripheralConfig {
    pub id: String,
    pub r#type: String, // "uart", "timer", "gpio", etc.
    #[serde(deserialize_with = "deserialize_u64_lax")]
    pub base_address: u64,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub irq: Option<u32>,
    /// Optional RCC clock-gate. `None` → the peripheral is never gated.
    #[serde(default)]
    pub clock: Option<ClockGate>,
    #[serde(default)]
    pub config: HashMap<String, serde_yaml::Value>,
}

/// One entry in a chip's authoritative pin map: which GPIO peripheral this pin's
/// output register lives on, and the bit within that port's data register. This
/// is silicon truth (from the SVD / board pinmux) — the pin *label* no longer
/// implies a port, so a board whose silkscreen labels a `gpioc` pin "PB0" resolves
/// correctly. Extra YAML fields (e.g. `functions:`, consumed by the app codegen)
/// are ignored here.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PinLoc {
    pub gpio: String,
    pub bit: u8,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChipDescriptor {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub name: String,
    pub arch: Arch, // Parsed from string
    /// Exact CPU core, e.g. "cortex-m3", "cortex-m33", "cortex-m0+".
    /// `Arch` collapses all Cortex-M variants into `Arm`, but some bus
    /// behavior is core-specific (bit-band aliasing exists only on M3/M4),
    /// so the precise core is carried separately. Optional for configs
    /// that predate this field.
    #[serde(default)]
    pub core: Option<String>,
    pub flash: MemoryRange,
    pub ram: MemoryRange,
    /// Offset in bytes from the flash base to the application vector table
    /// when a second-stage bootloader precedes it. The RP2040 bootrom runs a
    /// 256-byte stage-2 (boot2) blob from flash and only then enters the
    /// vector table at `flash_base + 0x100`, so set this to `0x100` for the
    /// RP2040. `0` (the default) means the vector table sits at the flash base
    /// — the usual case for STM32/nRF/etc. The simulator does not execute the
    /// stage-2 blob (flash is directly mapped); it uses this offset to find
    /// the real reset vector when the flash-base vectors are not valid.
    #[serde(default, deserialize_with = "deserialize_u64_lax")]
    pub reset_vector_offset: u64,
    /// RP2040-style atomic register aliases. When true, every 0x1000-strided
    /// alias of a peripheral register in the APB window decodes as an atomic
    /// op on the base register: `+0x0000` normal, `+0x1000` XOR, `+0x2000`
    /// SET (bitwise OR), `+0x3000` CLR (bitwise AND-NOT). The RP2040 HAL drives
    /// nearly all of its register setup through these aliases (`hw_set_bits`,
    /// `hw_clear_bits`), so without them an unmodified image faults on the
    /// first `hw_set_bits`. Default `false` (other Cortex-M parts).
    #[serde(default)]
    pub atomic_register_aliases: bool,
    /// Extra CPU-visible memory windows beyond `flash`/`ram` (e.g. ESP32 IRAM
    /// and flash-DROM). Empty for chips with a simple two-region map.
    #[serde(default)]
    pub memory_regions: Vec<NamedMemoryRange>,
    pub peripherals: Vec<PeripheralConfig>,
    /// Authoritative pin → GPIO map. When present, pin resolution uses this map
    /// instead of parsing the label letter; an undeclared pin fails to resolve
    /// (no silent standard-layout fallback). Absent → standard STM32/Nordic parse.
    #[serde(default)]
    pub pins: std::collections::BTreeMap<String, PinLoc>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExternalDevice {
    pub id: String,
    pub r#type: String,
    pub connection: String, // e.g. "uart1", "i2c1"
    /// Physical signal-to-pad route for a bus-attached device. Signal names
    /// are transport-generic (`sda`/`scl`, `mosi`/`miso`/`sck`, `tx`/`rx`)
    /// while pad labels stay target-native (`GPIO4`, `PB7`, ...).
    ///
    /// The schema keeps this optional so fixed-pin targets can remain concise;
    /// target-specific loaders decide when a transport requires it. In
    /// particular, ESP32-C3 I²C rejects a missing route because its GPIO matrix
    /// makes the controller-to-pad wiring runtime-configurable.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub route: BTreeMap<String, String>,
    #[serde(default)]
    pub config: HashMap<String, serde_yaml::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CosimAdapter {
    ExternalProcess,
    Fmi,
    Mock,
}

fn default_cosim_step_ns() -> u64 {
    1_000
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CosimModelConfig {
    pub id: String,
    pub adapter: CosimAdapter,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default = "default_cosim_step_ns")]
    pub step_ns: u64,
    #[serde(default)]
    pub inputs: HashMap<String, String>,
    #[serde(default)]
    pub outputs: HashMap<String, String>,
    #[serde(default)]
    pub config: HashMap<String, serde_yaml::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoardIoKind {
    Led,
    Button,
    AdcInput,
    PwmOutput,
    I2cDevice,
    SpiDevice,
    UartDevice,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum BoardIoSignal {
    #[default]
    Output,
    Input,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BoardIoBinding {
    pub id: String,
    pub kind: BoardIoKind,
    pub peripheral: String,
    pub pin: u8,
    #[serde(default)]
    pub signal: BoardIoSignal,
    #[serde(default = "default_true")]
    pub active_high: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub i2c_address: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemManifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub name: String,
    pub chip: String, // Reference to chip name or file path
    #[serde(default)]
    pub memory_overrides: HashMap<String, String>,
    #[serde(default)]
    pub external_devices: Vec<ExternalDevice>,
    #[serde(default)]
    pub cosim_models: Vec<CosimModelConfig>,
    #[serde(default)]
    pub board_io: Vec<BoardIoBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_uart: Option<String>,
    #[serde(default)]
    pub peripherals: Vec<PeripheralConfig>,
    /// Per-cycle peripheral-walk deletion (only consulted in `event-scheduler`
    /// builds; no-op otherwise). Three states:
    ///
    /// - **absent (`None`)** — the core auto-derives walk-deletability at
    ///   `from_config` finalize time: the walk is deleted iff EVERY peripheral
    ///   on the bus is provably walk-independent for all firmware states
    ///   (scheduler-driven, or its `tick()` is a structural no-op — see the
    ///   core's `Peripheral::needs_legacy_walk` contract). Conservative: any
    ///   peripheral that could ever do walk work keeps the walk on.
    /// - **`Some(true)`** — force the walk deleted (hand opt-in / escape hatch
    ///   for configs the author verified byte-identical walk-free but that the
    ///   conservative auto-derivation cannot prove — e.g. a firmware that never
    ///   arms the timers/ADC/DMA the chip descriptor instantiates).
    /// - **`Some(false)`** — pin the walk ON, overriding any auto-derivation.
    ///
    /// Deserializes from the YAML `walk_deleted:` key; omit it for auto-derive.
    #[serde(default)]
    pub walk_deleted: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    pub id: String,
    pub system: String,   // Path to SystemManifest
    pub firmware: String, // Path to ELF
    #[serde(default)]
    pub config_overrides: HashMap<String, serde_yaml::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentManifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub name: String,
    pub nodes: Vec<NodeConfig>,
    #[serde(default)]
    pub interconnects: Vec<InterconnectConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct InterconnectConfig {
    pub r#type: String,     // "uart_cross_link", "virtual_switch", etc.
    pub nodes: Vec<String>, // List of node IDs
    #[serde(default)]
    pub config: HashMap<String, serde_yaml::Value>,
}

impl EnvironmentManifest {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let source = std::fs::read_to_string(path)?;
        // `NodeConfig` deliberately keeps a plain HashMap for source
        // compatibility with callers that build worlds in Rust. A YAML key with
        // an empty mapping would otherwise deserialize identically to an absent
        // key, so inspect the wire shape before that normalization happens.
        let wire: serde_yaml::Value =
            serde_yaml::from_str(&source).context("Failed to parse Environment Manifest")?;
        reject_explicit_node_config_overrides(&wire)?;
        let manifest: Self =
            serde_yaml::from_str(&source).context("Failed to parse Environment Manifest")?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validate the structural contract shared by all environment runners.
    ///
    /// Topology-specific checks stay with `World::from_manifest`, where the
    /// named peripherals and machines are available. This layer rejects input
    /// that cannot describe an unambiguous world at all.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != "1.0" {
            anyhow::bail!(
                "Unsupported environment schema_version '{}'. Supported version: '1.0'",
                self.schema_version
            );
        }
        if self.name.trim().is_empty() {
            anyhow::bail!("Environment manifest requires a non-empty name");
        }
        if self.nodes.is_empty() {
            anyhow::bail!("Environment manifest requires at least one node");
        }

        let mut node_ids = HashSet::with_capacity(self.nodes.len());
        for (index, node) in self.nodes.iter().enumerate() {
            if node.id.trim().is_empty() {
                anyhow::bail!("Environment manifest nodes[{index}].id must be non-empty");
            }
            if !node_ids.insert(&node.id) {
                anyhow::bail!("Environment manifest has duplicate node id '{}'", node.id);
            }
            if node.system.trim().is_empty() {
                anyhow::bail!("Environment manifest nodes[{index}].system must be non-empty");
            }
            if node.firmware.trim().is_empty() {
                anyhow::bail!("Environment manifest nodes[{index}].firmware must be non-empty");
            }
            if !node.config_overrides.is_empty() {
                anyhow::bail!(
                    "Environment manifest nodes[{index}].config_overrides is unsupported in environment schema 1.0"
                );
            }
        }

        for (index, interconnect) in self.interconnects.iter().enumerate() {
            validate_environment_interconnect_config(index, interconnect)?;
        }

        Ok(())
    }
}

/// Reject `config_overrides` on the YAML wire before Serde collapses an absent
/// field, `{}`, and `null` into the same empty `HashMap`. Programmatic callers
/// cannot express that distinction, but every user-facing environment manifest
/// passes through [`EnvironmentManifest::from_file`].
fn reject_explicit_node_config_overrides(wire: &serde_yaml::Value) -> Result<()> {
    let nodes_key = serde_yaml::Value::String("nodes".to_string());
    let overrides_key = serde_yaml::Value::String("config_overrides".to_string());
    let Some(nodes) = wire
        .as_mapping()
        .and_then(|manifest| manifest.get(&nodes_key))
        .and_then(serde_yaml::Value::as_sequence)
    else {
        return Ok(());
    };

    for (index, node) in nodes.iter().enumerate() {
        if node
            .as_mapping()
            .is_some_and(|node| node.contains_key(&overrides_key))
        {
            anyhow::bail!(
                "Environment manifest nodes[{index}].config_overrides is unsupported in environment schema 1.0"
            );
        }
    }

    Ok(())
}

fn validate_environment_interconnect_config(
    index: usize,
    interconnect: &InterconnectConfig,
) -> Result<()> {
    let kind = interconnect.r#type.as_str();
    match kind {
        "uart_cross_link" => {
            reject_unknown_interconnect_config_keys(
                index,
                kind,
                &interconnect.config,
                &["node_a_uart", "node_b_uart"],
            )?;
            optional_nonempty_interconnect_string(
                index,
                kind,
                &interconnect.config,
                "node_a_uart",
            )?;
            optional_nonempty_interconnect_string(
                index,
                kind,
                &interconnect.config,
                "node_b_uart",
            )?;
        }
        "can_bus" => {
            reject_unknown_interconnect_config_keys(
                index,
                kind,
                &interconnect.config,
                &["peripheral"],
            )?;
            if interconnect
                .config
                .get("peripheral")
                .and_then(serde_yaml::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                anyhow::bail!("can_bus: missing nonblank config.peripheral");
            }
        }
        "egress" => {
            reject_unknown_interconnect_config_keys(
                index,
                kind,
                &interconnect.config,
                &[
                    "uart",
                    "transport",
                    "url",
                    "topic",
                    "encoding",
                    "buffer_max",
                ],
            )?;
            optional_nonempty_interconnect_string(index, kind, &interconnect.config, "uart")?;
            let transport = optional_nonempty_interconnect_string(
                index,
                kind,
                &interconnect.config,
                "transport",
            )?
            .unwrap_or("tcp");
            let url =
                optional_nonempty_interconnect_string(index, kind, &interconnect.config, "url")?;
            if url.is_none() {
                anyhow::bail!("egress: missing 'url'");
            }
            let topic =
                optional_nonempty_interconnect_string(index, kind, &interconnect.config, "topic")?;
            match transport {
                "tcp" | "http" => {
                    if topic.is_some() {
                        anyhow::bail!(
                            "interconnects[{index}].config.topic is supported only for egress transport mqtt"
                        );
                    }
                }
                "mqtt" => {
                    if topic.is_none() {
                        anyhow::bail!("egress: mqtt needs 'topic'");
                    }
                }
                other => anyhow::bail!("egress: unknown transport '{other}'"),
            }
            let encoding = optional_nonempty_interconnect_string(
                index,
                kind,
                &interconnect.config,
                "encoding",
            )?
            .unwrap_or("raw");
            if !matches!(encoding, "raw" | "ndjson-trace" | "frames-json") {
                anyhow::bail!("egress: unknown encoding '{encoding}'");
            }
            if let Some(buffer_max) = interconnect.config.get("buffer_max") {
                let Some(buffer_max) = buffer_max.as_u64() else {
                    anyhow::bail!(
                        "interconnects[{index}].config.buffer_max must be a positive integer"
                    );
                };
                if buffer_max == 0 || usize::try_from(buffer_max).is_err() {
                    anyhow::bail!(
                        "interconnects[{index}].config.buffer_max must be a positive integer"
                    );
                }
            }
        }
        other => anyhow::bail!("unsupported interconnect type '{other}'"),
    }
    Ok(())
}

fn reject_unknown_interconnect_config_keys(
    index: usize,
    kind: &str,
    config: &HashMap<String, serde_yaml::Value>,
    allowed: &[&str],
) -> Result<()> {
    let mut unknown: Vec<_> = config
        .keys()
        .filter(|key| !allowed.contains(&key.as_str()))
        .collect();
    unknown.sort();
    if let Some(key) = unknown.first() {
        anyhow::bail!("interconnects[{index}].config.{key} is not supported for {kind}");
    }
    Ok(())
}

fn optional_nonempty_interconnect_string<'a>(
    index: usize,
    kind: &str,
    config: &'a HashMap<String, serde_yaml::Value>,
    key: &str,
) -> Result<Option<&'a str>> {
    let Some(value) = config.get(key) else {
        return Ok(None);
    };
    let Some(value) = value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        anyhow::bail!("interconnects[{index}].config.{key} must be a non-empty string for {kind}");
    };
    Ok(Some(value))
}

impl ChipDescriptor {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)?;

        if path.extension().is_some_and(|ext| ext == "json") {
            let ir: labwired_ir::IrDevice = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse Strict IR from {:?}", path))?;
            Ok(Self::from(ir))
        } else {
            serde_yaml::from_str(&content).context("Failed to parse Chip Descriptor YAML")
        }
    }
}

impl SystemManifest {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let f = std::fs::File::open(path)?;
        let mut manifest: SystemManifest =
            serde_yaml::from_reader(f).context("Failed to parse System Manifest")?;
        // can-player accepts `path:` as a CLI convenience; core itself only
        // ever sees `data:` (keeps std::fs out of the sim core → wasm-safe).
        let base = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        for ext in &mut manifest.external_devices {
            if ext.r#type == "can-player" {
                if ext.config.contains_key("path") && ext.config.contains_key("data") {
                    return Err(anyhow::anyhow!(
                        "can-player '{}': both 'path' and 'data' are set in config; set only one \
                         ('path' is a CLI convenience that inlines the log into 'data')",
                        ext.id
                    ));
                }
                if let Some(p) = ext.config.remove("path") {
                    let p = p
                        .as_str()
                        .ok_or_else(|| {
                            anyhow::anyhow!("can-player '{}': path must be a string", ext.id)
                        })?
                        .to_string();
                    let full = base.join(&p);
                    let text = std::fs::read_to_string(&full).map_err(|e| {
                        anyhow::anyhow!("can-player '{}': cannot read log {:?}: {e}", ext.id, full)
                    })?;
                    ext.config
                        .insert("data".into(), serde_yaml::Value::String(text));
                }
            }
        }
        Ok(manifest)
    }

    pub fn validate_cosim_models(&self) -> Vec<String> {
        let mut issues = Vec::new();

        for (index, model) in self.cosim_models.iter().enumerate() {
            let location = format!("cosim_models[{index}]");
            if model.id.trim().is_empty() {
                issues.push(format!("{location}.id must be a non-empty identifier"));
            }
            if model.step_ns == 0 {
                issues.push(format!("{location}.step_ns must be greater than zero"));
            }
            if matches!(
                model.adapter,
                CosimAdapter::ExternalProcess | CosimAdapter::Fmi
            ) && model
                .model
                .as_deref()
                .is_none_or(|path| path.trim().is_empty())
            {
                issues.push(format!(
                    "{location}.model is required for {:?} adapters",
                    model.adapter
                ));
            }
        }

        issues
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Access {
    #[serde(alias = "R/W", alias = "rw")]
    ReadWrite,
    #[serde(alias = "RO", alias = "r")]
    ReadOnly,
    #[serde(alias = "WO", alias = "w")]
    WriteOnly,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FieldDescriptor {
    pub name: String,
    pub bit_range: [u8; 2], // [msb, lsb]
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReadAction {
    None,
    Clear,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WriteAction {
    None,
    #[serde(alias = "oneToClear")]
    WriteOneToClear,
    #[serde(alias = "zeroToClear")]
    WriteZeroToClear,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SideEffectsDescriptor {
    #[serde(default)]
    pub read_action: Option<ReadAction>,
    #[serde(default)]
    pub write_action: Option<WriteAction>,
    #[serde(default)]
    pub on_read: Option<String>,
    #[serde(default)]
    pub on_write: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimingTrigger {
    Write {
        register: String,
        #[serde(default)]
        value: Option<u32>,
        #[serde(default)]
        mask: Option<u32>,
    },
    Read {
        register: String,
    },
    Periodic {
        period_cycles: u64,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimingAction {
    SetBits { register: String, bits: u32 },
    ClearBits { register: String, bits: u32 },
    WriteValue { register: String, value: u32 },
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct TimingDescriptor {
    pub id: String,
    pub trigger: TimingTrigger,
    pub delay_cycles: u64,
    pub action: TimingAction,
    #[serde(default)]
    pub interrupt: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RegisterDescriptor {
    pub id: String,
    pub address_offset: u64,
    pub size: u8, // 8, 16, 32
    pub access: Access,
    pub reset_value: u32,
    #[serde(default)]
    pub fields: Vec<FieldDescriptor>,
    #[serde(default)]
    pub side_effects: Option<SideEffectsDescriptor>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PeripheralDescriptor {
    pub peripheral: String,
    pub version: String,
    pub registers: Vec<RegisterDescriptor>,
    #[serde(default)]
    pub interrupts: Option<std::collections::HashMap<String, u32>>,
    #[serde(default)]
    pub timing: Option<Vec<TimingDescriptor>>,
}

/// Declarative descriptor for a GPIO / pin-timing external device — the family
/// that DRIVES pins the MCU samples as inputs (rotary encoder, matrix keypad,
/// DHT22, HC-SR04, NeoPixel). Unlike register-mapped [`PeripheralDescriptor`]
/// peripherals, these live directly on the [`SystemBus`] as bus-resident
/// devices (or GPIO observers) and each carries a genuinely irreducible timing
/// algorithm — the **primitive** (quadrature walk, matrix reflect, one-wire
/// frame, …). This descriptor makes EVERYTHING AROUND the primitive data: the
/// device `type`, its pin bindings, and (later) the canvas-compiler emit
/// mapping. A device that reuses an existing primitive is then one YAML file
/// with zero Rust in either engine.
///
/// The struct deserializes only the fields the current implementation wires.
/// Serde ignores unknown keys, so a descriptor YAML may already carry
/// `metadata:` / `emit:` sections (documenting the full intent) before the code
/// that consumes them exists.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeviceDescriptor {
    /// `type:` string in a system.yaml `external_devices` entry. Unique across
    /// all declarative device descriptors.
    pub r#type: String,
    /// Runtime behavior: which irreducible primitive backs this device and how
    /// its abstract pin roles bind to `config:` keys.
    pub behavior: DeviceBehavior,
    /// How the canvas compiler emits this device's `external_devices` (and any
    /// auxiliary `board_io`) block. When present, BOTH engines (the Rust
    /// `canonical.rs` emitter and the TypeScript `compile()` emitter) derive the
    /// block from this single spec instead of a hand-mirrored pair.
    #[serde(default)]
    pub emit: Option<DeviceEmit>,
    /// Display + runtime metadata. `metadata.inputs` is load-bearing: it defines
    /// the [`crate`]-external stimulus channels the device accepts (the same
    /// channels the engine's generic device serves through `SimInput`). The
    /// remaining display fields are carried for the phase-2 `KitMetadata`
    /// derivation. Optional so the GPIO descriptors that predate the typed
    /// schema still parse.
    #[serde(default)]
    pub metadata: Option<DeviceMetadata>,
}

/// Display + runtime metadata for a declarative device. Only `inputs` is
/// consumed by the engine today; the display fields document phase-2 intent.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct DeviceMetadata {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    /// Long-form description shown in the library detail view. Absent ⇒ the kit
    /// falls back to `summary` (the pre-existing declarative-kit behaviour).
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    /// Extra `config:` keys this device accepts beyond `i2c_address`, mirrored
    /// verbatim into the peripheral manifest. When present this list is taken as
    /// the COMPLETE set of config keys (list `i2c_address` explicitly if the
    /// device accepts it); when absent the kit synthesises the lone
    /// `i2c_address` key. Carrying these lets a declarative descriptor reproduce
    /// a hand-written kit's manifest entry byte-for-byte.
    #[serde(default)]
    pub config_keys: Vec<ConfigKeySpec>,
    /// Starter labs that ship a one-click demo using this device, mirrored into
    /// the manifest exactly like a hand-written kit's `labs`.
    #[serde(default)]
    pub labs: Vec<LabSpec>,
    /// The named stimulus channels this device accepts. For an `i2c_device`
    /// primitive these are the measurement slots that register/response
    /// `source:` keys read; each `default` seeds the value the part reports
    /// until something drives it, and `min`/`max` bound accepted stimuli.
    #[serde(default)]
    pub inputs: Vec<InputSpec>,
}

/// A `config:` key advertised in the peripheral manifest. Mirrors the engine's
/// `KitMetadata::config_keys` entries so a declarative descriptor can reproduce
/// a hand-written kit's manifest documentation.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ConfigKeySpec {
    pub name: String,
    /// One of `str` | `int` | `bool` | `float`.
    pub ty: String,
    pub doc: String,
}

/// A starter-lab reference, mirroring the engine's `KitMetadata::labs` entries.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LabSpec {
    pub board_id: String,
    pub chip: String,
    pub example_dir: String,
    pub demo_elf: String,
}

/// One drivable stimulus channel (a measurement slot). Datasheet-facing
/// engineering units; the engine owns the conversion to raw register form.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InputSpec {
    /// Stable key both `source:` fields and the runtime stimulus API address.
    pub key: String,
    pub label: String,
    /// Engineering unit (e.g. `lx`, `ppm`, `°C`).
    pub unit: String,
    /// Inclusive accepted range.
    pub min: f64,
    pub max: f64,
    /// Value the slot holds until driven. Absent ⇒ 0.0.
    #[serde(default)]
    pub default: Option<f64>,
}

/// The `behavior.i2c` section of a declarative `i2c_device` — a datasheet-shaped
/// description of an I²C sensor's wire protocol, interpreted by the engine's
/// generic device. Two device shapes are covered, and a descriptor is exactly
/// one of them (see `registers` vs `commands`):
///   * **register-pointer** devices (`registers:`) — the master writes a 1-byte
///     pointer, then streams a fixed-width LE/BE word (VEML7700-style);
///   * **command** devices (`commands:`) — the master writes a 16-bit big-endian
///     command, then reads N words each followed by a CRC-8 byte
///     (Sensirion-style).
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct I2cSpec {
    /// 7-bit slave address used when the `external_devices` entry omits
    /// `i2c_address`.
    pub default_address: u8,
    /// Command-code width in bytes for a command device. `2` (the default) is
    /// the Sensirion 16-bit big-endian opcode; `1` is a single-byte opcode
    /// device (BH1750-style, where each measurement mode / power command is one
    /// byte). A command dispatches the instant the master has written
    /// `code_width` bytes. Ignored by register devices.
    #[serde(default = "default_code_width")]
    pub code_width: u8,
    /// CRC-8 parameters for command-response framing. Absent ⇒ responses carry
    /// no per-word checksum.
    #[serde(default)]
    pub crc8: Option<Crc8Spec>,
    /// Pointer-addressable registers. Present ⇒ this is a register device.
    #[serde(default)]
    pub registers: Vec<I2cRegister>,
    /// Command set. Present ⇒ this is a command device.
    #[serde(default)]
    pub commands: Vec<I2cCommand>,
}

/// CRC-8 parameters. Sensirion parts use `poly 0x31`, `init 0xFF`, no final XOR.
#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub struct Crc8Spec {
    pub poly: u8,
    pub init: u8,
}

/// Byte order of a register's on-wire word.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Endian {
    Le,
    Be,
}

/// Register access. `r` = read-only (the master only reads it); `rw` = the
/// master may also write it, and the model accumulates + echoes those writes.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum I2cAccess {
    R,
    Rw,
}

/// A pointer-addressable register.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct I2cRegister {
    pub name: String,
    /// Pointer byte the master writes to select this register.
    pub addr: u8,
    /// Width in bytes streamed on read / accumulated on write.
    pub width: u8,
    pub endian: Endian,
    pub access: I2cAccess,
    /// Power-on value (also the value read back before any write / measurement).
    #[serde(default)]
    pub reset: u32,
    /// Input-channel key whose (encoded) value this register reports on read.
    /// Absent ⇒ a plain storage register (reads back its written value / reset).
    #[serde(default)]
    pub source: Option<String>,
    /// Linear encoding applied to the sourced measurement before it is placed
    /// in the register word.
    #[serde(default)]
    pub encode: Option<Encode>,
    /// A constant the sourced value is multiplied by *before* encoding, applied
    /// as its own floating-point step (not folded into `encode.scale`). This is
    /// a datasheet responsivity ratio — e.g. the VEML7700 white channel reads
    /// `1.15 ×` the visible ALS illuminance. Absent ⇒ 1.0 (no pre-scale). It is
    /// a distinct multiply so the intermediate rounds byte-identically to a
    /// reference model that scales the measurement before converting it.
    #[serde(default)]
    pub source_scale: Option<f64>,
    /// Zero or more bit-field-selected scale factors, each read from another
    /// register's field and **multiplied together** (a single mapping or a YAML
    /// list are both accepted). In the default (multiply) mode these compound
    /// the counts-per-unit — e.g. a gain field ×1/×2/×4. In `resolution` mode
    /// they compound the resolution divisor instead (gain **and**
    /// integration-time fields together, which one field alone cannot express).
    #[serde(default, deserialize_with = "de_scale_from_list")]
    pub scale_from: Vec<ScaleFrom>,
    /// Resolution-divide mode. When present, the register reports
    /// `round((value × source_scale) ÷ resolution)`, where
    /// `resolution = <this base> × Π(scale_from factors)` folded left-to-right.
    /// This is the datasheet form for parts whose count = illuminance ÷
    /// resolution and whose resolution scales with programmed gain and
    /// integration time (VEML7700). Absent ⇒ the register uses the multiply
    /// encoding (`value × encode.scale × Π factors`). Mutually exclusive with a
    /// `source`-less register.
    #[serde(default)]
    pub resolution: Option<f64>,
}

/// Accept either a single `scale_from` mapping or a YAML list of them, yielding
/// a `Vec`. A bare mapping is the common single-field case (backward-compatible
/// with descriptors written before compounding fields existed); a list is used
/// when several bit-fields multiply together (e.g. gain × integration time).
fn de_scale_from_list<'de, D>(deserializer: D) -> Result<Vec<ScaleFrom>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(ScaleFrom),
        Many(Vec<ScaleFrom>),
    }
    Ok(match OneOrMany::deserialize(deserializer)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
    })
}

/// Linear measurement encoding: `raw = value * scale + offset`, clamped to the
/// optional `[clamp_min, clamp_max]` window (in raw units) before it is packed
/// into the word. `scale` defaults to 1.0, `offset` to 0.0.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Encode {
    #[serde(default = "one_f64")]
    pub scale: f64,
    #[serde(default)]
    pub offset: f64,
    #[serde(default)]
    pub clamp_min: Option<f64>,
    #[serde(default)]
    pub clamp_max: Option<f64>,
}

fn one_f64() -> f64 {
    1.0
}

/// A register-bit-field-keyed scale map. The engine extracts
/// `(value(register) >> shift) & mask` and multiplies the encode scale by
/// `map[field]` (or 1.0 when the field value is absent from the map).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ScaleFrom {
    /// Name of the register whose bit-field selects the factor.
    pub register: String,
    /// Mask applied after `shift`.
    pub mask: u32,
    #[serde(default)]
    pub shift: u8,
    /// Extracted field value → scale factor.
    pub map: std::collections::BTreeMap<u32, f64>,
}

/// One command in a command device's command set.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct I2cCommand {
    pub name: String,
    /// 16-bit command code, big-endian on the wire.
    pub code: u16,
    /// Microseconds the part needs after the command before its response is
    /// ready. Reads before this elapses return not-ready bytes. Absent ⇒ ready
    /// immediately. Gated on simulated wall-clock (`advance_time_us`).
    #[serde(default)]
    pub delay_us: Option<u64>,
    /// Response words in clock-out order. Empty ⇒ a write-only command.
    #[serde(default)]
    pub response: Vec<ResponseWord>,
    /// Count of parameter words the master writes after the code (each a 16-bit
    /// word plus CRC on the wire). Accepted and ignored.
    #[serde(default)]
    pub params_words: u8,
}

/// One word of a command response — either a live measurement (`source`) or a
/// fixed constant (`const`). Exactly one of the two applies.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ResponseWord {
    /// Input-channel key whose encoded value fills this word.
    #[serde(default)]
    pub source: Option<String>,
    /// A fixed value (e.g. a data-ready flag or serial-number word).
    #[serde(rename = "const", default)]
    pub const_value: Option<u32>,
    /// Word width in bytes (big-endian on the wire). Default 2.
    #[serde(default = "default_response_width")]
    pub width: u8,
    /// Linear encoding for a `source` word.
    #[serde(default)]
    pub encode: Option<Encode>,
}

fn default_response_width() -> u8 {
    2
}

fn default_code_width() -> u8 {
    2
}

/// The canvas-compiler emit spec for a declarative device — the single source
/// both engines interpret. A `config` entry sources its value one of four ways
/// (a wired MCU pin, a list of wired pins, a computed board value, or a parsed
/// part attribute); a device that also needs an auxiliary `board_io` entry
/// (e.g. a rotary encoder's push switch) lists it under `board_io`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeviceEmit {
    /// The emitted `type:` string. Defaults to the descriptor `type` when
    /// omitted — set it only when they differ (e.g. descriptor `rotary_encoder`
    /// emits `rotary-encoder`; part `ultrasonic` emits `hc-sr04`).
    #[serde(default)]
    pub device_type: Option<String>,
    /// The emitted `connection:` (e.g. `"gpio"`).
    pub connection: String,
    /// Ordered `config:` entries. The whole device emits nothing if any entry
    /// whose source is a pin binding cannot be resolved (all pin bindings are
    /// required — a partially-wired device is not emitted).
    pub config: Vec<EmitConfig>,
    /// Auxiliary `board_io` entries (e.g. a rotary encoder's SW button). Each is
    /// optional — an unwired one is simply skipped.
    #[serde(default)]
    pub board_io: Vec<EmitBoardIo>,
}

/// One emitted `config:` entry. Exactly one of the `from_*` sources applies,
/// checked in declaration order; `default` supplies the fallback for `from_attr`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EmitConfig {
    /// The emitted key (e.g. `clk_pin`, `cpu_hz`).
    pub key: String,
    /// Source: the first of these part-pin names that is wired to the MCU
    /// supplies a quoted pad label; if none is wired the whole device is
    /// skipped. Mutually exclusive with the other sources.
    #[serde(default)]
    pub from_part_pin: Option<Vec<String>>,
    /// Source: every listed part-pin must be wired; emits a `["p1", "p2", …]`
    /// list. If any is unwired the whole device is skipped.
    #[serde(default)]
    pub from_part_pins: Option<Vec<String>>,
    /// Source: a computed board value by name — `"sim_cpu_hz"` (the firmware
    /// clock) or `"echo_pacing_cpu_hz"` (the HC-SR04 echo-pacing override).
    #[serde(default)]
    pub from: Option<String>,
    /// Source: a numeric part attribute of this name, parsed as f64.
    #[serde(default)]
    pub from_attr: Option<String>,
    /// Fallback for `from_attr` when the attribute is absent or non-numeric.
    #[serde(default)]
    pub default: Option<f64>,
}

/// One auxiliary `board_io` entry emitted alongside the device (e.g. a rotary
/// encoder's momentary push switch). Skipped when its pin is unwired.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EmitBoardIo {
    /// The first of these part-pin names wired to the MCU supplies the pad.
    pub from_part_pin: Vec<String>,
    /// The emitted `kind:` (e.g. `"button"`).
    pub kind: String,
    /// The emitted `signal:` (e.g. `"input"`).
    pub signal: String,
    /// The emitted `active_high:`.
    pub active_high: bool,
}

/// The runtime half of a [`DeviceDescriptor`]: the primitive to instantiate and
/// how to source its pins/params from the placed device's `config:` block.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DeviceBehavior {
    /// Name of the irreducible Rust primitive to instantiate — e.g.
    /// `"quadrature"` (rotary encoder). The `bus/declarative_device.rs`
    /// attach dispatch matches on this.
    pub primitive: String,
    /// Abstract pin role → the `config:` key that carries its pad label. For
    /// the quadrature primitive: `{ "a": "clk_pin", "b": "dt_pin" }`. Ordered
    /// (BTreeMap) so attach is deterministic.
    #[serde(default)]
    pub pins: std::collections::BTreeMap<String, String>,
    /// Optional scalar params (with their `config:` key and default) the
    /// primitive needs beyond pins — e.g. `cpu_hz`. Kept as raw YAML values so
    /// the primitive decides the concrete type.
    #[serde(default)]
    pub params: std::collections::BTreeMap<String, serde_yaml::Value>,
    /// For the `i2c_device` primitive: the datasheet-shaped wire-protocol spec
    /// the engine's generic I²C device interprets. Absent for the GPIO
    /// primitives (quadrature / matrix / one-wire / pulse-echo).
    #[serde(default)]
    pub i2c: Option<I2cSpec>,
}

impl DeviceDescriptor {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        serde_yaml::from_str(yaml).context("Failed to parse Device Descriptor")
    }

    /// Look up and parse the embedded descriptor for a device `type:` string
    /// (accepts either spelling for the encoder). Returns `Ok(None)` for a type
    /// with no declarative descriptor. This is the SINGLE embed point — both the
    /// runtime attach path (`core`'s `bus/declarative_device.rs`) and the canvas
    /// emitter (`canonical.rs`) resolve descriptors through here, so there is one
    /// source of truth for the `configs/devices/*.yaml` set.
    pub fn embedded(device_type: &str) -> Result<Option<Self>> {
        match embedded_device_yaml(device_type) {
            Some(yaml) => Ok(Some(Self::from_yaml(yaml).with_context(|| {
                format!("Failed to parse embedded device descriptor for '{device_type}'")
            })?)),
            None => Ok(None),
        }
    }
}

/// The embedded `configs/devices/*.yaml` descriptors, keyed by `type:` string.
/// `include_str!` bundles them so wasm builds (no `std::fs`) resolve them too.
pub fn embedded_device_yaml(device_type: &str) -> Option<&'static str> {
    match device_type {
        "rotary_encoder" | "rotary-encoder" => {
            Some(include_str!("../../../configs/devices/rotary_encoder.yaml"))
        }
        "keypad" => Some(include_str!("../../../configs/devices/keypad.yaml")),
        "dht22" | "am2302" => Some(include_str!("../../../configs/devices/dht22.yaml")),
        "hc-sr04" | "hcsr04" => Some(include_str!("../../../configs/devices/hc_sr04.yaml")),
        "sht31" => Some(include_str!("../../../configs/devices/sht31.yaml")),
        "bh1750" => Some(include_str!("../../../configs/devices/bh1750.yaml")),
        "veml7700" => Some(include_str!("../../../configs/devices/veml7700.yaml")),
        _ => None,
    }
}

impl PeripheralDescriptor {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = std::fs::read_to_string(&path)?;
        Self::from_yaml(&content)
    }

    pub fn from_yaml(yaml: &str) -> Result<Self> {
        serde_yaml::from_str(yaml).context("Failed to parse Peripheral Descriptor")
    }
}

impl From<labwired_ir::IrPeripheral> for PeripheralDescriptor {
    fn from(ir: labwired_ir::IrPeripheral) -> Self {
        let mut interrupts = std::collections::HashMap::new();
        for int in ir.interrupts {
            interrupts.insert(int.name, int.value);
        }

        Self {
            peripheral: ir.name,
            version: "ir-v1".to_string(),
            registers: ir
                .registers
                .into_iter()
                .map(|r| RegisterDescriptor {
                    id: r.name,
                    address_offset: r.offset,
                    size: r.size as u8,
                    access: match r.access {
                        labwired_ir::IrAccess::ReadOnly => Access::ReadOnly,
                        labwired_ir::IrAccess::WriteOnly => Access::WriteOnly,
                        _ => Access::ReadWrite,
                    },
                    reset_value: r.reset_value as u32,
                    fields: r
                        .fields
                        .into_iter()
                        .map(|f| FieldDescriptor {
                            name: f.name,
                            bit_range: [(f.bit_offset + f.bit_width - 1) as u8, f.bit_offset as u8],
                            description: f.description,
                        })
                        .collect(),
                    side_effects: r.side_effects.map(|se| SideEffectsDescriptor {
                        read_action: se.read_action.and_then(|s| match s.as_str() {
                            "clear" => Some(ReadAction::Clear),
                            "none" => Some(ReadAction::None),
                            _ => None,
                        }),
                        write_action: se.write_action.and_then(|s| match s.as_str() {
                            "one_to_clear" | "oneToClear" | "w1c" => {
                                Some(WriteAction::WriteOneToClear)
                            }
                            "zero_to_clear" | "zeroToClear" | "w0c" => {
                                Some(WriteAction::WriteZeroToClear)
                            }
                            "none" => Some(WriteAction::None),
                            _ => None,
                        }),
                        on_read: None,
                        on_write: None,
                    }),
                })
                .collect(),
            interrupts: if interrupts.is_empty() {
                None
            } else {
                Some(interrupts)
            },
            timing: if ir.timing.is_empty() {
                None
            } else {
                let mut timing = Vec::new();
                for t in ir.timing {
                    // Try to convert JSON trigger/action to enums
                    let trigger: Result<TimingTrigger, _> = serde_json::from_value(t.trigger);
                    let action: Result<TimingAction, _> = serde_json::from_value(t.action);

                    if let (Ok(trig), Ok(act)) = (trigger, action) {
                        timing.push(TimingDescriptor {
                            id: t.id,
                            trigger: trig,
                            delay_cycles: t.delay_cycles,
                            action: act,
                            interrupt: t.interrupt,
                        });
                    } else {
                        tracing::warn!("Failed to convert IR timing hook '{}' to config", t.id);
                    }
                }
                if timing.is_empty() {
                    None
                } else {
                    Some(timing)
                }
            },
        }
    }
}

impl From<labwired_ir::IrDevice> for ChipDescriptor {
    fn from(ir: labwired_ir::IrDevice) -> Self {
        let ir_arch = ir.arch.to_uppercase();
        let arch = match ir_arch.as_str() {
            "CM3" | "CM4" | "CM7" | "ARM" => Arch::Arm,
            "RISCV" | "RV32" => Arch::RiscV,
            "XTENSA" | "LX7" | "LX6" => Arch::Xtensa,
            _ => Arch::Arm, // Default to Arm for CMSIS-SVD
        };
        // CMSIS-SVD carries the exact core ("CM3", "CM4", "CM33", ...);
        // preserve it so core-specific bus behavior (bit-band) can be gated.
        let core = ir_arch
            .strip_prefix("CM")
            .map(|rest| format!("cortex-m{}", rest.to_lowercase()));

        let flash = ir
            .memory_regions
            .get("FLASH")
            .map(|r| MemoryRange {
                base: r.base,
                size: format!("{}B", r.size),
            })
            .unwrap_or(MemoryRange {
                base: 0,
                size: "0".to_string(),
            });

        let ram = ir
            .memory_regions
            .get("RAM")
            .map(|r| MemoryRange {
                base: r.base,
                size: format!("{}B", r.size),
            })
            .unwrap_or(MemoryRange {
                base: 0,
                size: "0".to_string(),
            });

        Self {
            schema_version: default_schema_version(),
            name: ir.name,
            arch,
            core,
            flash,
            ram,
            reset_vector_offset: 0,
            atomic_register_aliases: false,
            memory_regions: Vec::new(),
            peripherals: ir
                .peripherals
                .into_values()
                .map(|p| {
                    let ir_p_name = p.name.clone();
                    let ir_p_base = p.base_address;
                    PeripheralConfig {
                        id: ir_p_name,
                        r#type: "strict_ir_internal".to_string(),
                        base_address: ir_p_base,
                        size: None,
                        irq: None,
                        clock: None,
                        config: std::collections::HashMap::from([(
                            "internal_ir_peripheral".to_string(),
                            serde_yaml::to_value(p).unwrap(),
                        )]),
                    }
                })
                .collect(),
            pins: std::collections::BTreeMap::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct TestInputs {
    pub firmware: String,
    pub system: Option<String>,
}

/// Inputs for a multi-node environment test. Environment scripts are selected
/// exclusively by `inputs.env`; they cannot name single-node firmware inputs.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct EnvTestInputs {
    pub env: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct TestLimits {
    pub max_steps: u64,
    #[serde(default)]
    pub max_cycles: Option<u64>,
    #[serde(default)]
    pub max_uart_bytes: Option<u64>,
    #[serde(default)]
    pub no_progress_steps: Option<u64>,
    #[serde(default)]
    pub wall_time_ms: Option<u64>,
    #[serde(default)]
    pub max_vcd_bytes: Option<u64>,
    #[serde(default)]
    pub stop_when_assertions_pass: bool,
    /// Number of steps the machine must keep executing past the first moment
    /// all runtime assertions pass before `AssertionsPassed` is accepted. This
    /// closes the print-then-crash false-pass hole: firmware that emits its
    /// acceptance token and then faults will break with the fault reason during
    /// the settling window instead of certifying as passed.
    #[serde(default = "default_stop_settle_steps")]
    pub stop_when_assertions_pass_settle_steps: u64,
    /// Absolute step floor: the assertions-pass early-stop may not trigger
    /// before this many steps have executed.
    #[serde(default)]
    pub stop_when_assertions_pass_min_steps: u64,
}

fn default_stop_settle_steps() -> u64 {
    100_000
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// Runner failed before simulation started (e.g. script parse/validation error).
    ConfigError,
    MaxSteps,
    MaxCycles,
    MaxUartBytes,
    MaxVcdBytes,
    NoProgress,
    WallTime,
    AssertionsPassed,
    MemoryViolation,
    DecodeError,
    Halt,
    Exception,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct UartContainsAssertion {
    pub uart_contains: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct UartRegexAssertion {
    pub uart_regex: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct StopReasonAssertion {
    pub expected_stop_reason: StopReason,
}

#[derive(Debug, Clone)]
pub struct MemoryValueDetails {
    pub address: u64,
    pub expected_value: u64,
    pub mask: Option<u64>,
    /// Value width to read at `address`. Accepts bytes (1/2/4) or the
    /// equivalent bit width (8/16/32); both map to a u8/u16/u32 read.
    /// Defaults to a 32-bit (u32) word.
    pub size: Option<u8>,
    /// Target node for a multi-node environment assertion. Single-node scripts
    /// leave this unset and continue to use the existing machine path.
    pub node: Option<String>,
}

// `MemoryValueDetails` is public and callers historically construct it with a
// struct literal. Keep that field shape intact while retaining the distinction
// between an omitted `node` and parsed `node: null`: the latter is an invalid
// explicit qualifier in single-node scripts and must survive a serde round
// trip. This reserved private sentinel is created only while deserializing a
// `node: null` field.
const EXPLICIT_NULL_NODE_SENTINEL: &str = "\u{0}labwired:explicit-null-node";

fn is_explicit_null_node(node: Option<&str>) -> bool {
    node == Some(EXPLICIT_NULL_NODE_SENTINEL)
}

impl MemoryValueDetails {
    /// Creates an unqualified memory assertion with all optional fields unset.
    ///
    /// Set [`Self::node`] after construction when building an environment
    /// assertion programmatically.
    pub fn new(address: u64, expected_value: u64) -> Self {
        Self {
            address,
            expected_value,
            mask: None,
            size: None,
            node: None,
        }
    }
}

#[derive(Serialize)]
struct SerializableMemoryValueDetails<'a> {
    address: u64,
    expected_value: u64,
    mask: Option<u64>,
    size: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    node: Option<Option<&'a str>>,
}

impl Serialize for MemoryValueDetails {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        SerializableMemoryValueDetails {
            address: self.address,
            expected_value: self.expected_value,
            mask: self.mask,
            size: self.size,
            node: if is_explicit_null_node(self.node.as_deref()) {
                Some(None)
            } else {
                self.node.as_deref().map(Some)
            },
        }
        .serialize(serializer)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryValueDetailsWire {
    address: u64,
    expected_value: u64,
    #[serde(default)]
    mask: Option<u64>,
    #[serde(default)]
    size: Option<u8>,
    #[serde(default)]
    node: FieldPresence<String>,
}

impl<'de> Deserialize<'de> for MemoryValueDetails {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = MemoryValueDetailsWire::deserialize(deserializer)?;
        Ok(Self {
            address: wire.address,
            expected_value: wire.expected_value,
            mask: wire.mask,
            size: wire.size,
            node: match wire.node {
                FieldPresence::Absent => None,
                FieldPresence::Present(Some(node)) => Some(node),
                FieldPresence::Present(None) => Some(EXPLICIT_NULL_NODE_SENTINEL.to_string()),
            },
        })
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct MemoryValueAssertion {
    pub memory_value: MemoryValueDetails,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum UdsTesterResult {
    Done,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct UdsTesterDetails {
    pub id: String,
    pub result: UdsTesterResult,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct UdsTesterAssertion {
    pub uds_tester: UdsTesterDetails,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum TestAssertion {
    UartContains(UartContainsAssertion),
    UartRegex(UartRegexAssertion),
    ExpectedStopReason(StopReasonAssertion),
    MemoryValue(MemoryValueAssertion),
    UdsTester(UdsTesterAssertion),
}

/// Where a fault is applied. Either a peripheral (by `id`, optionally narrowed
/// to a `register` and `bit`) or a raw memory `address`. Resolved against the
/// built chip when the run starts.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct FaultTarget {
    #[serde(default)]
    pub peripheral: Option<String>,
    #[serde(default)]
    pub register: Option<String>,
    #[serde(default)]
    pub bit: Option<u8>,
    #[serde(default)]
    pub address: Option<u64>,
}

/// The access mode a `permission_flip` fault forces a register into.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    ReadOnly,
    WriteOnly,
}

/// The access direction a `permission_violation` fault denies.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessDirection {
    Read,
    Write,
}

/// When a fault takes effect. Mirrors the declarative peripheral trigger
/// vocabulary so peripheral-class faults reuse the same evaluator.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum FaultTrigger {
    /// Applied while the bus is built, before the firmware runs.
    #[default]
    AtStart,
    /// Applied once, `cycles` cycles into the run.
    AfterCycles { cycles: u64 },
    /// Applied when the firmware writes `register` (optionally matching value/mask).
    OnWrite {
        register: String,
        #[serde(default)]
        value: Option<u64>,
        #[serde(default)]
        mask: Option<u64>,
    },
    /// Applied when the firmware reads `register`.
    OnRead { register: String },
}

/// The taxonomy of injectable faults. Each maps to a documented silicon failure
/// mode; see the per-kind required parameters enforced in [`TestScript::validate`].
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FaultKind {
    MissingClock,
    StuckAtBit,
    WrongResetValue,
    PermissionFlip,
    BoundViolation,
    PermissionViolation,
    MemoryCorruption,
    DelayedIrq,
    NeverIrq,
    PeripheralErrorState,
    PeripheralTimeout,
}

/// A single injected fault. `kind`-specific parameters are the optional fields;
/// which are required is enforced structurally by [`TestScript::validate`].
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FaultSpec {
    pub id: String,
    pub kind: FaultKind,
    #[serde(default)]
    pub target: FaultTarget,
    #[serde(default)]
    pub trigger: FaultTrigger,
    /// `stuck_at_bit`: the level (0 or 1) the bit is held at.
    #[serde(default)]
    pub level: Option<u8>,
    /// `wrong_reset_value` / `memory_corruption`: the value written.
    #[serde(default)]
    pub value: Option<u64>,
    /// `memory_corruption`: XOR mask applied to the target instead of `value`.
    #[serde(default)]
    pub xor: Option<u64>,
    /// `permission_flip`: the mode to force the register into.
    #[serde(default)]
    pub to: Option<AccessMode>,
    /// `permission_violation`: the direction to deny.
    #[serde(default)]
    pub deny: Option<AccessDirection>,
    /// `delayed_irq`: how many cycles to delay the interrupt.
    #[serde(default)]
    pub delay_cycles: Option<u64>,
    /// `delayed_irq` / `never_irq`: the interrupt name on the peripheral.
    #[serde(default)]
    pub interrupt: Option<String>,
    /// `peripheral_error_state` / `peripheral_timeout`: the status bits to set.
    #[serde(default)]
    pub bits: Option<u64>,
    /// Memory-class faults: access width in bytes (1/2/4).
    #[serde(default)]
    pub size: Option<u8>,
}

/// The safe-behaviour judgment for a fault-injection run. `safe_when` reuses the
/// ordinary assertion vocabulary; the firmware passes iff every entry holds.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Verdict {
    #[serde(default)]
    pub safe_when: Vec<TestAssertion>,
    /// If true (default), the run is invalid — not a pass — unless every fault
    /// is observed to actually fire. The false-pass gate.
    #[serde(default = "default_true")]
    pub require_fault_fired: bool,
}

/// Which input channel a stimulus drives. `channel` is the `sim_input`
/// channel key (e.g. `x` on an accelerometer); `component`, when given,
/// narrows resolution to the device owned by the peripheral with that bus
/// name (or the sensor id for directly-attached sensors) — the disambiguator
/// when two devices expose the same channel key. Without `component`,
/// resolution is by unique channel key and an ambiguous channel is a run-time
/// error rather than silently picking one.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StimulusTarget {
    /// Optional owning peripheral name / sensor id to disambiguate `channel`.
    #[serde(default)]
    pub component: Option<String>,
    /// The input channel key to drive.
    pub channel: String,
}

/// A declarative input stimulus (schema_version 1.2+): drive `target` to
/// `value` (in the channel's engineering unit) when `trigger` fires. Reuses the
/// [`FaultTrigger`] vocabulary; the first cut supports `at_start` and
/// `after_cycles` (the time-based triggers). The runner applies each stimulus
/// via the generic `Machine::set_input` path, so it works for any input device
/// without per-type wiring.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StimulusSpec {
    pub target: StimulusTarget,
    #[serde(default)]
    pub trigger: FaultTrigger,
    /// The value to set the channel to, in its engineering unit.
    pub value: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct TestScript {
    pub schema_version: String,
    pub inputs: TestInputs,
    pub limits: TestLimits,
    #[serde(default)]
    pub assertions: Vec<TestAssertion>,
    /// Faults to inject into the simulated silicon (schema_version 1.1+).
    #[serde(default)]
    pub faults: Vec<FaultSpec>,
    /// The safe-behaviour verdict for a fault-injection run (schema_version 1.1+).
    #[serde(default)]
    pub verdict: Option<Verdict>,
    /// Input stimuli to drive during the run (schema_version 1.2+).
    #[serde(default)]
    pub stimuli: Vec<StimulusSpec>,
}

fn reject_explicit_memory_nodes(assertions: &[TestAssertion], script_kind: &str) -> Result<()> {
    for (index, assertion) in assertions.iter().enumerate() {
        if let TestAssertion::MemoryValue(memory) = assertion {
            if memory.memory_value.node.is_some() {
                anyhow::bail!(
                    "{script_kind} test scripts do not support 'node' on memory_value assertions (assertions[{index}])"
                );
            }
        }
    }
    Ok(())
}

impl TestScript {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let f = std::fs::File::open(&path)
            .with_context(|| format!("Failed to open test script at {:?}", path.as_ref()))?;
        let script: Self =
            serde_yaml::from_reader(f).context("Failed to parse Test Script YAML")?;
        script.validate()?;
        Ok(script)
    }

    pub fn validate(&self) -> Result<()> {
        if !matches!(self.schema_version.as_str(), "1.0" | "1.1" | "1.2") {
            anyhow::bail!(
                "Unsupported schema_version '{}'. Supported versions: '1.0', '1.1', '1.2'",
                self.schema_version
            );
        }

        if self.inputs.firmware.trim().is_empty() {
            anyhow::bail!("Input 'firmware' path cannot be empty");
        }

        if self.limits.max_steps == 0 {
            anyhow::bail!("Limit 'max_steps' must be greater than zero");
        }

        reject_explicit_memory_nodes(&self.assertions, "single-node")?;

        // Fault injection requires schema_version 1.1+.
        if self.schema_version == "1.0" && (!self.faults.is_empty() || self.verdict.is_some()) {
            anyhow::bail!(
                "'faults'/'verdict' require schema_version '1.1' (got '{}')",
                self.schema_version
            );
        }

        // Input stimuli require schema_version 1.2+.
        if !self.stimuli.is_empty() && matches!(self.schema_version.as_str(), "1.0" | "1.1") {
            anyhow::bail!(
                "'stimuli' require schema_version '1.2' (got '{}')",
                self.schema_version
            );
        }
        for (i, s) in self.stimuli.iter().enumerate() {
            if s.target.channel.trim().is_empty() {
                anyhow::bail!("stimuli[{}]: target.channel cannot be empty", i);
            }
            // Only the time-based triggers are wired for stimuli today; the
            // register-access triggers need a write/read hook we haven't added
            // for the input path. Fail loud rather than silently never firing.
            match &s.trigger {
                FaultTrigger::AtStart | FaultTrigger::AfterCycles { .. } => {}
                other => anyhow::bail!(
                    "stimuli[{}]: trigger {:?} is not yet supported for stimuli \
                     (use at_start or after_cycles)",
                    i,
                    other
                ),
            }
            if !s.value.is_finite() {
                anyhow::bail!("stimuli[{}]: value must be a finite number", i);
            }
        }

        // Structural fault-compiler guardrails. Deeper checks that need the
        // built chip (target resolution, bit-within-register) run when the bus
        // is available; these catch malformed specs up front.
        let mut seen = std::collections::HashSet::new();
        for fault in &self.faults {
            if fault.id.trim().is_empty() {
                anyhow::bail!("Every fault needs a non-empty 'id'");
            }
            if !seen.insert(fault.id.as_str()) {
                anyhow::bail!("Duplicate fault id '{}'", fault.id);
            }
            validate_fault(fault)?;
        }

        Ok(())
    }
}

/// A strict v1.0 script for a multi-node environment world.
///
/// The explicit fault, verdict, and stimulus fields are parsed so validation
/// can reject them diagnostically rather than silently treating them as
/// unknown or ignoring them in the environment runner.
#[derive(Debug, Clone)]
pub struct EnvTestScript {
    pub schema_version: String,
    pub inputs: EnvTestInputs,
    pub limits: TestLimits,
    pub assertions: Vec<TestAssertion>,
    pub faults: Vec<FaultSpec>,
    pub verdict: Option<Verdict>,
    pub stimuli: Vec<StimulusSpec>,
    explicit_limits: EnvExplicitLimits,
    explicit_unsupported_fields: EnvExplicitUnsupportedFields,
}

/// A field whose parser records the difference between being absent and being
/// explicitly configured to a default value (or `null`). Environment scripts
/// use it to preserve their strict serialization contract and to distinguish
/// an absent setting from an invalid explicit `null`.
#[derive(Debug, Clone, Copy, Default)]
enum FieldPresence<T> {
    #[default]
    Absent,
    Present(Option<T>),
}

impl<T> FieldPresence<T> {
    fn is_present(&self) -> bool {
        matches!(self, Self::Present(_))
    }

    fn into_value(self) -> Option<T> {
        match self {
            Self::Absent | Self::Present(None) => None,
            Self::Present(Some(value)) => Some(value),
        }
    }
}

impl<'de, T> Deserialize<'de> for FieldPresence<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self::Present(Option::<T>::deserialize(deserializer)?))
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct EnvExplicitLimits {
    no_progress_steps: FieldPresence<u64>,
    max_vcd_bytes: FieldPresence<u64>,
    stop_when_assertions_pass: FieldPresence<bool>,
    stop_when_assertions_pass_settle_steps: FieldPresence<u64>,
    stop_when_assertions_pass_min_steps: FieldPresence<u64>,
}

#[derive(Debug, Clone, Default)]
struct EnvExplicitUnsupportedFields {
    faults: FieldPresence<Vec<FaultSpec>>,
    verdict: FieldPresence<Verdict>,
    stimuli: FieldPresence<Vec<StimulusSpec>>,
}

/// Serialization keeps the user-visible environment contract strict in both
/// directions. Valid scripts omit defaulted limits; invalid parsed or
/// programmatically-mutated unsupported fields remain visible so a
/// serialize/parse cycle cannot make them look valid.
#[derive(Serialize)]
struct SerializableEnvTestScript<'a> {
    schema_version: &'a str,
    inputs: &'a EnvTestInputs,
    limits: SerializableEnvTestLimits,
    assertions: &'a [TestAssertion],
    #[serde(skip_serializing_if = "Option::is_none")]
    faults: Option<Option<&'a [FaultSpec]>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verdict: Option<Option<&'a Verdict>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stimuli: Option<Option<&'a [StimulusSpec]>>,
}

#[derive(Serialize)]
struct SerializableEnvTestLimits {
    max_steps: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_cycles: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_uart_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    no_progress_steps: Option<Option<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    wall_time_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_vcd_bytes: Option<Option<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_when_assertions_pass: Option<Option<bool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_when_assertions_pass_settle_steps: Option<Option<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_when_assertions_pass_min_steps: Option<Option<u64>>,
}

fn serialize_unsupported_option_limit(
    explicit: FieldPresence<u64>,
    value: Option<u64>,
) -> Option<Option<u64>> {
    match value {
        Some(value) => Some(Some(value)),
        None if explicit.is_present() => Some(None),
        None => None,
    }
}

fn serialize_explicit_bool_limit(
    explicit: FieldPresence<bool>,
    value: bool,
) -> Option<Option<bool>> {
    if value {
        Some(Some(true))
    } else {
        match explicit {
            FieldPresence::Absent => None,
            FieldPresence::Present(None) => Some(None),
            FieldPresence::Present(Some(_)) => Some(Some(false)),
        }
    }
}

fn serialize_explicit_defaulted_limit(
    explicit: FieldPresence<u64>,
    value: u64,
    default: u64,
) -> Option<Option<u64>> {
    if value != default {
        Some(Some(value))
    } else {
        match explicit {
            FieldPresence::Absent => None,
            FieldPresence::Present(None) => Some(None),
            FieldPresence::Present(Some(_)) => Some(Some(value)),
        }
    }
}

fn serialize_unsupported_sequence<'a, T>(
    explicit: &FieldPresence<Vec<T>>,
    value: &'a [T],
) -> Option<Option<&'a [T]>> {
    if !value.is_empty() {
        Some(Some(value))
    } else {
        match explicit {
            FieldPresence::Absent => None,
            FieldPresence::Present(None) => Some(None),
            FieldPresence::Present(Some(_)) => Some(Some(value)),
        }
    }
}

fn serialize_unsupported_verdict<'a>(
    explicit: &FieldPresence<Verdict>,
    value: Option<&'a Verdict>,
) -> Option<Option<&'a Verdict>> {
    match value {
        Some(value) => Some(Some(value)),
        None if explicit.is_present() => Some(None),
        None => None,
    }
}

impl Serialize for EnvTestScript {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        SerializableEnvTestScript {
            schema_version: &self.schema_version,
            inputs: &self.inputs,
            limits: SerializableEnvTestLimits {
                max_steps: self.limits.max_steps,
                max_cycles: self.limits.max_cycles,
                max_uart_bytes: self.limits.max_uart_bytes,
                no_progress_steps: serialize_unsupported_option_limit(
                    self.explicit_limits.no_progress_steps,
                    self.limits.no_progress_steps,
                ),
                wall_time_ms: self.limits.wall_time_ms,
                max_vcd_bytes: serialize_unsupported_option_limit(
                    self.explicit_limits.max_vcd_bytes,
                    self.limits.max_vcd_bytes,
                ),
                stop_when_assertions_pass: serialize_explicit_bool_limit(
                    self.explicit_limits.stop_when_assertions_pass,
                    self.limits.stop_when_assertions_pass,
                ),
                stop_when_assertions_pass_settle_steps: serialize_explicit_defaulted_limit(
                    self.explicit_limits.stop_when_assertions_pass_settle_steps,
                    self.limits.stop_when_assertions_pass_settle_steps,
                    default_stop_settle_steps(),
                ),
                stop_when_assertions_pass_min_steps: serialize_explicit_defaulted_limit(
                    self.explicit_limits.stop_when_assertions_pass_min_steps,
                    self.limits.stop_when_assertions_pass_min_steps,
                    0,
                ),
            },
            assertions: &self.assertions,
            faults: serialize_unsupported_sequence(
                &self.explicit_unsupported_fields.faults,
                &self.faults,
            ),
            verdict: serialize_unsupported_verdict(
                &self.explicit_unsupported_fields.verdict,
                self.verdict.as_ref(),
            ),
            stimuli: serialize_unsupported_sequence(
                &self.explicit_unsupported_fields.stimuli,
                &self.stimuli,
            ),
        }
        .serialize(serializer)
    }
}

/// Wire shape for an environment limits block. It resolves to `TestLimits`
/// after retaining presence information for settings whose explicit defaults
/// and nullability need a stable public contract.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvTestLimits {
    max_steps: u64,
    #[serde(default)]
    max_cycles: Option<u64>,
    #[serde(default)]
    max_uart_bytes: Option<u64>,
    #[serde(default)]
    no_progress_steps: FieldPresence<u64>,
    #[serde(default)]
    wall_time_ms: Option<u64>,
    #[serde(default)]
    max_vcd_bytes: FieldPresence<u64>,
    #[serde(default)]
    stop_when_assertions_pass: FieldPresence<bool>,
    #[serde(default)]
    stop_when_assertions_pass_settle_steps: FieldPresence<u64>,
    #[serde(default)]
    stop_when_assertions_pass_min_steps: FieldPresence<u64>,
}

impl EnvTestLimits {
    fn into_parts(self) -> (TestLimits, EnvExplicitLimits) {
        let explicit_limits = EnvExplicitLimits {
            no_progress_steps: self.no_progress_steps,
            max_vcd_bytes: self.max_vcd_bytes,
            stop_when_assertions_pass: self.stop_when_assertions_pass,
            stop_when_assertions_pass_settle_steps: self.stop_when_assertions_pass_settle_steps,
            stop_when_assertions_pass_min_steps: self.stop_when_assertions_pass_min_steps,
        };
        let limits = TestLimits {
            max_steps: self.max_steps,
            max_cycles: self.max_cycles,
            max_uart_bytes: self.max_uart_bytes,
            no_progress_steps: self.no_progress_steps.into_value(),
            wall_time_ms: self.wall_time_ms,
            max_vcd_bytes: self.max_vcd_bytes.into_value(),
            stop_when_assertions_pass: self.stop_when_assertions_pass.into_value().unwrap_or(false),
            stop_when_assertions_pass_settle_steps: self
                .stop_when_assertions_pass_settle_steps
                .into_value()
                .unwrap_or_else(default_stop_settle_steps),
            stop_when_assertions_pass_min_steps: self
                .stop_when_assertions_pass_min_steps
                .into_value()
                .unwrap_or_default(),
        };
        (limits, explicit_limits)
    }
}

/// Strict deserialization wire form for `EnvTestScript`. The public type keeps
/// `TestLimits` for runners, while this shape preserves explicit values needed
/// for strict serialization and validation.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EnvTestScriptWire {
    schema_version: String,
    inputs: EnvTestInputs,
    limits: EnvTestLimits,
    #[serde(default)]
    assertions: Vec<TestAssertion>,
    #[serde(default)]
    faults: FieldPresence<Vec<FaultSpec>>,
    #[serde(default)]
    verdict: FieldPresence<Verdict>,
    #[serde(default)]
    stimuli: FieldPresence<Vec<StimulusSpec>>,
}

impl<'de> Deserialize<'de> for EnvTestScript {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = EnvTestScriptWire::deserialize(deserializer)?;
        let EnvTestScriptWire {
            schema_version,
            inputs,
            limits: wire_limits,
            assertions,
            faults,
            verdict,
            stimuli,
        } = wire;
        let (limits, explicit_limits) = wire_limits.into_parts();
        let explicit_unsupported_fields = EnvExplicitUnsupportedFields {
            faults: faults.clone(),
            verdict: verdict.clone(),
            stimuli: stimuli.clone(),
        };
        Ok(Self {
            schema_version,
            inputs,
            limits,
            assertions,
            faults: faults.into_value().unwrap_or_default(),
            verdict: verdict.into_value(),
            stimuli: stimuli.into_value().unwrap_or_default(),
            explicit_limits,
            explicit_unsupported_fields,
        })
    }
}

impl EnvTestScript {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != "1.0" {
            anyhow::bail!(
                "Environment test scripts require schema_version '1.0' (got '{}')",
                self.schema_version
            );
        }

        if self.inputs.env.trim().is_empty() {
            anyhow::bail!("Input 'env' path cannot be empty");
        }

        if self.limits.max_steps == 0 {
            anyhow::bail!("Limit 'max_steps' must be greater than zero");
        }

        if self.explicit_limits.no_progress_steps.is_present()
            || self.limits.no_progress_steps.is_some()
        {
            anyhow::bail!("Environment test scripts do not support 'limits.no_progress_steps'");
        }
        if self.explicit_limits.max_vcd_bytes.is_present() || self.limits.max_vcd_bytes.is_some() {
            anyhow::bail!("Environment test scripts do not support 'limits.max_vcd_bytes'");
        }
        if matches!(
            self.explicit_limits.stop_when_assertions_pass,
            FieldPresence::Present(None)
        ) {
            anyhow::bail!(
                "Environment test script limit 'stop_when_assertions_pass' must not be null"
            );
        }
        if matches!(
            self.explicit_limits.stop_when_assertions_pass_settle_steps,
            FieldPresence::Present(None)
        ) {
            anyhow::bail!(
                "Environment test script limit 'stop_when_assertions_pass_settle_steps' must not be null"
            );
        }
        if matches!(
            self.explicit_limits.stop_when_assertions_pass_min_steps,
            FieldPresence::Present(None)
        ) {
            anyhow::bail!(
                "Environment test script limit 'stop_when_assertions_pass_min_steps' must not be null"
            );
        }
        if self.explicit_unsupported_fields.faults.is_present() || !self.faults.is_empty() {
            anyhow::bail!("Environment test scripts do not support 'faults'");
        }
        if self.explicit_unsupported_fields.verdict.is_present() || self.verdict.is_some() {
            anyhow::bail!("Environment test scripts do not support 'verdict'");
        }
        if self.explicit_unsupported_fields.stimuli.is_present() || !self.stimuli.is_empty() {
            anyhow::bail!("Environment test scripts do not support 'stimuli'");
        }

        if self.assertions.is_empty() {
            anyhow::bail!("Environment test scripts require at least one assertion");
        }

        for (index, assertion) in self.assertions.iter().enumerate() {
            let TestAssertion::MemoryValue(memory) = assertion else {
                anyhow::bail!(
                    "Environment test scripts support only memory_value assertions (assertions[{index}])"
                );
            };
            let has_node =
                memory.memory_value.node.as_deref().is_some_and(|node| {
                    !node.trim().is_empty() && !is_explicit_null_node(Some(node))
                });
            if !has_node {
                anyhow::bail!(
                    "Environment memory_value assertion at assertions[{index}] requires a non-empty 'node'"
                );
            }
        }

        Ok(())
    }
}

/// Per-kind structural validation of a fault spec: that the target shape and the
/// kind-specific parameters required to lower the fault are present. This is the
/// config-side half of the fault compiler; silicon-resolution guardrails (does
/// the peripheral exist, is the bit within the register) run against the built
/// bus at run time.
fn validate_fault(f: &FaultSpec) -> Result<()> {
    let needs_peripheral = || -> Result<()> {
        if f.target.peripheral.is_none() {
            anyhow::bail!("Fault '{}' ({:?}) needs target.peripheral", f.id, f.kind);
        }
        Ok(())
    };
    let needs_register = || -> Result<()> {
        if f.target.register.is_none() {
            anyhow::bail!("Fault '{}' ({:?}) needs target.register", f.id, f.kind);
        }
        Ok(())
    };
    let needs_address = || -> Result<()> {
        if f.target.address.is_none() {
            anyhow::bail!("Fault '{}' ({:?}) needs target.address", f.id, f.kind);
        }
        Ok(())
    };

    match f.kind {
        FaultKind::MissingClock => needs_peripheral()?,
        FaultKind::StuckAtBit => {
            needs_peripheral()?;
            needs_register()?;
            if f.target.bit.is_none() {
                anyhow::bail!("Fault '{}' (stuck_at_bit) needs target.bit", f.id);
            }
            match f.level {
                Some(0) | Some(1) => {}
                _ => anyhow::bail!("Fault '{}' (stuck_at_bit) needs level: 0 or 1", f.id),
            }
        }
        FaultKind::WrongResetValue => {
            needs_peripheral()?;
            needs_register()?;
            if f.value.is_none() {
                anyhow::bail!("Fault '{}' (wrong_reset_value) needs 'value'", f.id);
            }
        }
        FaultKind::PermissionFlip => {
            needs_peripheral()?;
            needs_register()?;
            if f.to.is_none() {
                anyhow::bail!("Fault '{}' (permission_flip) needs 'to'", f.id);
            }
        }
        FaultKind::BoundViolation => needs_address()?,
        FaultKind::PermissionViolation => {
            needs_address()?;
            if f.deny.is_none() {
                anyhow::bail!("Fault '{}' (permission_violation) needs 'deny'", f.id);
            }
        }
        FaultKind::MemoryCorruption => {
            needs_address()?;
            if f.value.is_none() && f.xor.is_none() {
                anyhow::bail!(
                    "Fault '{}' (memory_corruption) needs 'value' or 'xor'",
                    f.id
                );
            }
        }
        FaultKind::DelayedIrq => {
            needs_peripheral()?;
            if f.interrupt.is_none() {
                anyhow::bail!("Fault '{}' (delayed_irq) needs 'interrupt'", f.id);
            }
            if f.delay_cycles.is_none() {
                anyhow::bail!("Fault '{}' (delayed_irq) needs 'delay_cycles'", f.id);
            }
        }
        FaultKind::NeverIrq => {
            needs_peripheral()?;
            if f.interrupt.is_none() {
                anyhow::bail!("Fault '{}' (never_irq) needs 'interrupt'", f.id);
            }
        }
        FaultKind::PeripheralErrorState | FaultKind::PeripheralTimeout => {
            needs_peripheral()?;
            needs_register()?;
            if f.bits.is_none() {
                anyhow::bail!("Fault '{}' ({:?}) needs 'bits'", f.id, f.kind);
            }
        }
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
enum LegacySchemaVersion {
    Int(u64),
    Str(String),
}

impl LegacySchemaVersion {
    fn is_v1(&self) -> bool {
        match self {
            LegacySchemaVersion::Int(v) => *v == 1,
            LegacySchemaVersion::Str(s) => s.trim() == "1",
        }
    }
}

/// Deprecated legacy script format (schema_version: 1).
///
/// This format predates the v1.0 `inputs`/`limits` nesting. It remains supported for backward
/// compatibility, but should be migrated to v1.0.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct LegacyTestScriptV1 {
    schema_version: LegacySchemaVersion,
    #[serde(default)]
    pub firmware: Option<String>,
    #[serde(default)]
    pub system: Option<String>,
    pub max_steps: u64,
    #[serde(default)]
    pub wall_time_ms: Option<u64>,
    #[serde(default)]
    pub assertions: Vec<TestAssertion>,
}

impl LegacyTestScriptV1 {
    pub fn validate(&self) -> Result<()> {
        if !self.schema_version.is_v1() {
            anyhow::bail!(
                "Unsupported legacy schema_version. Supported legacy versions: 1 (deprecated)"
            );
        }
        reject_explicit_memory_nodes(&self.assertions, "legacy")?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum LoadedTestScript {
    V1_0(TestScript),
    LegacyV1(LegacyTestScriptV1),
    Env(EnvTestScript),
}

/// Load a CI test script from YAML.
///
/// Supported formats:
/// - v1.0 environment: `schema_version: "1.0"` with `inputs.env`.
/// - v1.0 (frozen): `schema_version: \"1.0\"` with `inputs` + `limits` + `assertions`.
/// - legacy v1 (deprecated): `schema_version: 1` with `max_steps` at the top level.
pub fn load_test_script<P: AsRef<Path>>(path: P) -> Result<LoadedTestScript> {
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read test script at {:?}", path.as_ref()))?;

    // Probe the raw YAML before trying the strict single-node schema. That
    // keeps TestInputs' deny_unknown_fields boundary intact while making the
    // two v1.0 input shapes unambiguous.
    let raw_script: serde_yaml::Value =
        serde_yaml::from_str(&contents).context("Failed to parse Test Script YAML")?;
    let raw_inputs = raw_script.get("inputs");
    let raw_env = raw_inputs.and_then(|inputs| inputs.get("env"));
    let raw_firmware = raw_inputs.and_then(|inputs| inputs.get("firmware"));

    if raw_env.is_some() && raw_firmware.is_some() {
        anyhow::bail!(
            "Test script inputs cannot contain both 'env' and 'firmware'; choose exactly one"
        );
    }

    if raw_env.is_some_and(serde_yaml::Value::is_string) {
        let env_script: EnvTestScript = serde_yaml::from_str(&contents)
            .context("Failed to parse environment Test Script YAML")?;
        env_script.validate()?;
        return Ok(LoadedTestScript::Env(env_script));
    }

    if raw_inputs.is_some() && raw_env.is_none() && raw_firmware.is_none() {
        anyhow::bail!("Test script inputs must contain exactly one of 'env' or 'firmware'");
    }

    match serde_yaml::from_str::<TestScript>(&contents) {
        Ok(script) => {
            script.validate()?;
            Ok(LoadedTestScript::V1_0(script))
        }
        Err(v1_err) => {
            let looks_like_legacy_v1 = raw_script
                .get("schema_version")
                .cloned()
                .map(|v| match v {
                    serde_yaml::Value::Number(n) => n.as_i64() == Some(1) || n.as_u64() == Some(1),
                    serde_yaml::Value::String(s) => s.trim() == "1",
                    _ => false,
                })
                .unwrap_or(false);

            if !looks_like_legacy_v1 {
                return Err(v1_err).context(
                    "Failed to parse Test Script YAML (expected schema_version: \"1.0\")",
                );
            }

            let legacy: LegacyTestScriptV1 = serde_yaml::from_str(&contents)
                .context("Failed to parse legacy Test Script YAML (schema_version: 1)")?;
            legacy.validate()?;
            Ok(LoadedTestScript::LegacyV1(legacy))
        }
    }
}

pub fn parse_size(size_str: &str) -> Result<u64> {
    use human_size::{Byte, Size, SpecificSize};
    let trimmed = size_str.trim();
    // A bare integer is a raw byte count. `human_size` rejects unit-less values
    // with "no multiple", but many chip configs give sizes as plain bytes
    // (e.g. `1048576`), so accept those directly before falling back to the
    // unit-aware parser ("512KB", "1.5 MiB", …).
    if let Ok(bytes) = trimmed.parse::<u64>() {
        return Ok(bytes);
    }
    let s: Size = trimmed
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid size format: {}", e))?;
    let bytes: SpecificSize<Byte> = s.into();
    Ok(bytes.value() as u64)
}

#[cfg(test)]
mod parse_size_tests {
    use super::parse_size;

    #[test]
    fn bare_integers_are_byte_counts() {
        assert_eq!(parse_size("1048576").unwrap(), 1_048_576);
        assert_eq!(parse_size("262144").unwrap(), 262_144);
        assert_eq!(parse_size("  4096  ").unwrap(), 4096);
    }

    #[test]
    fn unit_suffixes_still_parse() {
        assert_eq!(parse_size("512KB").unwrap(), 524_288);
        assert_eq!(parse_size("1564672B").unwrap(), 1_564_672);
        assert_eq!(parse_size("1KB").unwrap(), 1024);
    }

    #[test]
    fn garbage_still_errors() {
        assert!(parse_size("not-a-size").is_err());
    }
}

#[cfg(test)]
mod stimuli_tests {
    use super::*;

    fn script(schema: &str, stimuli_block: &str) -> String {
        format!(
            r#"
schema_version: "{schema}"
inputs:
  firmware: "cow.elf"
  system: "sys.yaml"
limits:
  max_steps: 1000
{stimuli_block}
"#
        )
    }

    #[test]
    fn stimuli_parse_and_validate_on_1_2() {
        let yaml = script(
            "1.2",
            r#"stimuli:
  - target: { component: fxos8700, channel: x }
    trigger: !after_cycles { cycles: 800000 }
    value: 2.0
  - target: { channel: z }
    value: 1.0
"#,
        );
        let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
        s.validate().unwrap();
        assert_eq!(s.stimuli.len(), 2);
        assert_eq!(s.stimuli[0].target.channel, "x");
        assert_eq!(s.stimuli[0].target.component.as_deref(), Some("fxos8700"));
        assert_eq!(s.stimuli[0].value, 2.0);
        // Default trigger is at_start.
        assert!(matches!(s.stimuli[1].trigger, FaultTrigger::AtStart));
    }

    #[test]
    fn stimuli_require_schema_1_2() {
        for schema in ["1.0", "1.1"] {
            let yaml = script(
                schema,
                "stimuli:\n  - target: { channel: x }\n    value: 1.0\n",
            );
            let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
            let err = s.validate().unwrap_err().to_string();
            assert!(err.contains("require schema_version '1.2'"), "{err}");
        }
    }

    #[test]
    fn empty_channel_rejected() {
        let yaml = script(
            "1.2",
            "stimuli:\n  - target: { channel: \"\" }\n    value: 1.0\n",
        );
        let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
        assert!(s
            .validate()
            .unwrap_err()
            .to_string()
            .contains("channel cannot be empty"));
    }

    #[test]
    fn register_triggers_rejected_for_stimuli() {
        let yaml = script(
            "1.2",
            r#"stimuli:
  - target: { channel: x }
    trigger: !on_write { register: "FOO" }
    value: 1.0
"#,
        );
        let s: TestScript = serde_yaml::from_str(&yaml).unwrap();
        assert!(s
            .validate()
            .unwrap_err()
            .to_string()
            .contains("not yet supported for stimuli"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_valid_script() {
        let yaml = r#"
schema_version: "1.0"
inputs:
  firmware: "path/to/fw.elf"
  system: "path/to/sys.yaml"
limits:
  max_steps: 1000
  wall_time_ms: 5000
assertions:
  - uart_contains: "Hello"
  - expected_stop_reason: halt
"#;
        let script: TestScript = serde_yaml::from_str(yaml).unwrap();
        assert!(script.validate().is_ok());
        assert_eq!(script.inputs.firmware, "path/to/fw.elf");
        assert_eq!(script.limits.max_steps, 1000);
        assert_eq!(script.assertions.len(), 2);
    }

    #[test]
    fn test_fault_injection_script_roundtrips() {
        let yaml = r#"
schema_version: "1.1"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 1000
faults:
  - id: usart1_no_clock
    kind: missing_clock
    target: { peripheral: usart1 }
  - id: sr_stuck
    kind: stuck_at_bit
    target: { peripheral: usart1, register: sr, bit: 7 }
    level: 1
    trigger: at_start
verdict:
  safe_when:
    - uart_contains: "FAULT_HANDLED"
  require_fault_fired: true
"#;
        let script: TestScript = serde_yaml::from_str(yaml).unwrap();
        script.validate().expect("valid 1.1 fault script");
        assert_eq!(script.faults.len(), 2);
        assert_eq!(script.faults[0].kind, FaultKind::MissingClock);
        assert!(script.verdict.as_ref().unwrap().require_fault_fired);
    }

    #[test]
    fn test_faults_require_v1_1() {
        let yaml = r#"
schema_version: "1.0"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 100
faults:
  - id: x
    kind: missing_clock
    target: { peripheral: usart1 }
"#;
        let script: TestScript = serde_yaml::from_str(yaml).unwrap();
        let err = script.validate().unwrap_err();
        assert!(err.to_string().contains("require schema_version '1.1'"));
    }

    #[test]
    fn test_fault_missing_required_param_rejected() {
        // stuck_at_bit without a level.
        let yaml = r#"
schema_version: "1.1"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 100
faults:
  - id: bad
    kind: stuck_at_bit
    target: { peripheral: usart1, register: sr, bit: 7 }
"#;
        let script: TestScript = serde_yaml::from_str(yaml).unwrap();
        let err = script.validate().unwrap_err();
        assert!(err.to_string().contains("level"));
    }

    #[test]
    fn test_duplicate_fault_id_rejected() {
        let yaml = r#"
schema_version: "1.1"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 100
faults:
  - id: dup
    kind: missing_clock
    target: { peripheral: a }
  - id: dup
    kind: missing_clock
    target: { peripheral: b }
"#;
        let script: TestScript = serde_yaml::from_str(yaml).unwrap();
        let err = script.validate().unwrap_err();
        assert!(err.to_string().contains("Duplicate fault id"));
    }

    #[test]
    fn test_v1_0_script_still_valid_without_faults() {
        let yaml = r#"
schema_version: "1.0"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 100
"#;
        let script: TestScript = serde_yaml::from_str(yaml).unwrap();
        assert!(script.validate().is_ok());
        assert!(script.faults.is_empty());
    }

    #[test]
    fn test_invalid_version() {
        let yaml = r#"
schema_version: "2.0"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 100
"#;
        let script: TestScript = serde_yaml::from_str(yaml).unwrap();
        let err = script.validate().unwrap_err();
        assert!(err.to_string().contains("Unsupported schema_version"));
    }

    #[test]
    fn test_invalid_max_steps() {
        let yaml = r#"
schema_version: "1.0"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 0
"#;
        let script: TestScript = serde_yaml::from_str(yaml).unwrap();
        let err = script.validate().unwrap_err();
        assert!(err.to_string().contains("max_steps"));
    }

    #[test]
    fn test_empty_firmware() {
        let yaml = r#"
schema_version: "1.0"
inputs:
  firmware: ""
limits:
  max_steps: 100
"#;
        let script: TestScript = serde_yaml::from_str(yaml).unwrap();
        let err = script.validate().unwrap_err();
        assert!(err.to_string().contains("firmware"));
    }

    #[test]
    fn test_system_manifest_accepts_uart_device_board_io_kind() {
        let yaml = r#"
name: "uart-device-smoke"
chip: "inline"
board_io:
  - id: "iolink_master"
    kind: "uart_device"
    peripheral: "uart2"
    pin: 2
    signal: "output"
    active_high: true
"#;

        let manifest: SystemManifest = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(manifest.board_io[0].kind, BoardIoKind::UartDevice);
    }

    #[test]
    fn test_external_device_preserves_target_neutral_bus_signal_route() {
        let yaml = r#"
name: "stm32-i2c-route-shape"
chip: "inline"
external_devices:
  - id: "oled"
    type: "oled-ssd1306-128x32"
    connection: "i2c1"
    route:
      sda: "PB7"
      scl: "PB6"
    config:
      i2c_address: 0x3c
"#;

        let manifest: SystemManifest = serde_yaml::from_str(yaml).unwrap();
        let route = &manifest.external_devices[0].route;
        assert_eq!(route.get("sda").map(String::as_str), Some("PB7"));
        assert_eq!(route.get("scl").map(String::as_str), Some("PB6"));

        let round_trip = serde_yaml::to_string(&manifest).unwrap();
        assert!(round_trip.contains("route:"));
        assert!(round_trip.contains("sda: PB7"));
        assert!(round_trip.contains("scl: PB6"));
    }

    fn write_temp_file(prefix: &str, contents: &str) -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push("labwired-config-tests");
        let _ = std::fs::create_dir_all(&dir);

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = dir.join(format!("{}-{}.yaml", prefix, nonce));
        std::fs::write(&path, contents).expect("Failed to write temp file");
        path
    }

    #[test]
    fn test_load_legacy_v1_script() {
        let script_path = write_temp_file(
            "legacy-v1",
            r#"
schema_version: 1
max_steps: 0
assertions: []
"#,
        );

        let loaded = load_test_script(&script_path).unwrap();
        assert!(matches!(loaded, LoadedTestScript::LegacyV1(_)));
    }

    #[test]
    fn legacy_script_rejects_node_qualified_memory_assertions() {
        for (name, node) in [("legacy-node", "tester"), ("legacy-null-node", "null")] {
            let script_path = write_temp_file(
                name,
                &format!(
                    r#"
schema_version: 1
max_steps: 10
assertions:
  - memory_value:
      node: {node}
      address: 0x20010000
      expected_value: 1
"#
                ),
            );

            let err = load_test_script(&script_path).unwrap_err().to_string();
            assert!(err.contains("node"), "unexpected error: {err}");
            assert!(err.contains("legacy"), "unexpected error: {err}");
        }
    }

    #[test]
    fn legacy_explicit_null_memory_node_round_trips_as_invalid() {
        let script: LegacyTestScriptV1 = serde_yaml::from_str(
            r#"
schema_version: 1
max_steps: 10
assertions:
  - memory_value:
      node: null
      address: 0x20010000
      expected_value: 1
"#,
        )
        .unwrap();
        let err = script.validate().unwrap_err().to_string();
        assert!(err.contains("legacy"), "unexpected error: {err}");

        let serialized = serde_yaml::to_string(&script).unwrap();
        assert!(
            serialized.contains("node: null"),
            "explicit null node was lost during serialization: {serialized}"
        );
        let round_tripped: LegacyTestScriptV1 = serde_yaml::from_str(&serialized).unwrap();
        let err = round_tripped.validate().unwrap_err().to_string();
        assert!(err.contains("legacy"), "unexpected error: {err}");
    }

    #[test]
    fn load_env_script_selects_env_variant_and_preserves_allowed_fields() {
        let script_path = write_temp_file(
            "env-script",
            r#"
schema_version: "1.0"
inputs:
  env: "twonode-env.yaml"
limits:
  max_steps: 50000
  max_cycles: 75000
  max_uart_bytes: 2048
  wall_time_ms: 3000
assertions:
  - memory_value:
      node: tester
      address: 0x20010000
      expected_value: 0xA5
      size: 1
"#,
        );

        let loaded = load_test_script(&script_path).unwrap();
        match loaded {
            LoadedTestScript::Env(script) => {
                assert_eq!(script.inputs.env, "twonode-env.yaml");
                assert_eq!(script.limits.max_steps, 50_000);
                assert_eq!(script.limits.max_cycles, Some(75_000));
                assert_eq!(script.limits.max_uart_bytes, Some(2_048));
                assert_eq!(script.limits.wall_time_ms, Some(3_000));
                let TestAssertion::MemoryValue(assertion) = &script.assertions[0] else {
                    panic!("expected memory_value assertion");
                };
                assert_eq!(assertion.memory_value.node.as_deref(), Some("tester"));
            }
            other => panic!("expected environment script, got {other:?}"),
        }
    }

    #[test]
    fn env_script_serialization_does_not_introduce_unsupported_runner_options() {
        let script: EnvTestScript = serde_yaml::from_str(
            r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits:
  max_steps: 10
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
"#,
        )
        .unwrap();
        script.validate().unwrap();

        let serialized = serde_yaml::to_string(&script).unwrap();
        for option in [
            "no_progress_steps",
            "max_vcd_bytes",
            "stop_when_assertions_pass",
            "stop_when_assertions_pass_settle_steps",
            "stop_when_assertions_pass_min_steps",
            "faults:",
            "verdict:",
            "stimuli:",
        ] {
            assert!(
                !serialized.contains(option),
                "unexpected serialized script: {serialized}"
            );
        }

        let round_tripped: EnvTestScript = serde_yaml::from_str(&serialized).unwrap();
        round_tripped.validate().unwrap();
    }

    #[test]
    fn env_script_accepts_and_round_trips_assertion_completion_limits() {
        let script: EnvTestScript = serde_yaml::from_str(
            r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits:
  max_steps: 10
  stop_when_assertions_pass: true
  stop_when_assertions_pass_settle_steps: 7
  stop_when_assertions_pass_min_steps: 3
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
"#,
        )
        .unwrap();

        script.validate().unwrap();
        assert!(script.limits.stop_when_assertions_pass);
        assert_eq!(script.limits.stop_when_assertions_pass_settle_steps, 7);
        assert_eq!(script.limits.stop_when_assertions_pass_min_steps, 3);

        let serialized = serde_yaml::to_string(&script).unwrap();
        for field in [
            "stop_when_assertions_pass: true",
            "stop_when_assertions_pass_settle_steps: 7",
            "stop_when_assertions_pass_min_steps: 3",
        ] {
            assert!(serialized.contains(field), "missing {field}: {serialized}");
        }
        let round_tripped: EnvTestScript = serde_yaml::from_str(&serialized).unwrap();
        round_tripped.validate().unwrap();
        assert_eq!(
            round_tripped.limits.stop_when_assertions_pass,
            script.limits.stop_when_assertions_pass
        );
        assert_eq!(
            round_tripped.limits.stop_when_assertions_pass_settle_steps,
            script.limits.stop_when_assertions_pass_settle_steps
        );
        assert_eq!(
            round_tripped.limits.stop_when_assertions_pass_min_steps,
            script.limits.stop_when_assertions_pass_min_steps
        );
    }

    #[test]
    fn env_script_preserves_explicit_assertion_completion_defaults() {
        let script: EnvTestScript = serde_yaml::from_str(
            r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits:
  max_steps: 10
  stop_when_assertions_pass: false
  stop_when_assertions_pass_settle_steps: 100000
  stop_when_assertions_pass_min_steps: 0
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
"#,
        )
        .unwrap();

        script.validate().unwrap();
        let serialized = serde_yaml::to_string(&script).unwrap();
        for field in [
            "stop_when_assertions_pass: false",
            "stop_when_assertions_pass_settle_steps: 100000",
            "stop_when_assertions_pass_min_steps: 0",
        ] {
            assert!(serialized.contains(field), "missing {field}: {serialized}");
        }

        let round_tripped: EnvTestScript = serde_yaml::from_str(&serialized).unwrap();
        round_tripped.validate().unwrap();
        assert!(!round_tripped.limits.stop_when_assertions_pass);
        assert_eq!(
            round_tripped.limits.stop_when_assertions_pass_settle_steps,
            100_000
        );
        assert_eq!(round_tripped.limits.stop_when_assertions_pass_min_steps, 0);
    }

    #[test]
    fn env_script_rejects_null_assertion_completion_limits() {
        for (name, extra, field) in [
            (
                "early-pass-null",
                "  stop_when_assertions_pass: null",
                "stop_when_assertions_pass",
            ),
            (
                "early-pass-settle-null",
                "  stop_when_assertions_pass_settle_steps: null",
                "stop_when_assertions_pass_settle_steps",
            ),
            (
                "early-pass-minimum-null",
                "  stop_when_assertions_pass_min_steps: null",
                "stop_when_assertions_pass_min_steps",
            ),
        ] {
            let script_path = write_temp_file(
                name,
                &format!(
                    r#"
schema_version: "1.0"
inputs: {{ env: "twonode-env.yaml" }}
limits:
  max_steps: 10
{extra}
assertions:
  - memory_value: {{ node: tester, address: 0x20010000, expected_value: 1 }}
"#
                ),
            );
            let err = load_test_script(&script_path).unwrap_err().to_string();
            assert!(err.contains(field), "unexpected error: {err}");
            assert!(err.contains("must not be null"), "unexpected error: {err}");
        }
    }

    #[test]
    fn env_script_preserves_invalid_null_assertion_completion_limits() {
        for (field, value) in [
            ("stop_when_assertions_pass", "null"),
            ("stop_when_assertions_pass_settle_steps", "null"),
            ("stop_when_assertions_pass_min_steps", "null"),
        ] {
            let script: EnvTestScript = serde_yaml::from_str(&format!(
                r#"
schema_version: "1.0"
inputs: {{ env: "twonode-env.yaml" }}
limits:
  max_steps: 10
  {field}: {value}
assertions:
  - memory_value: {{ node: tester, address: 0x20010000, expected_value: 1 }}
"#
            ))
            .unwrap();

            let serialized = serde_yaml::to_string(&script).unwrap();
            assert!(
                serialized.contains(&format!("{field}: {value}")),
                "explicit null {field} was lost during serialization: {serialized}"
            );
            let round_tripped: EnvTestScript = serde_yaml::from_str(&serialized).unwrap();
            let err = round_tripped.validate().unwrap_err().to_string();
            assert!(err.contains(field), "unexpected error: {err}");
            assert!(err.contains("must not be null"), "unexpected error: {err}");
        }
    }

    fn valid_env_script_for_mutation() -> EnvTestScript {
        serde_yaml::from_str(
            r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits:
  max_steps: 10
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
"#,
        )
        .unwrap()
    }

    #[test]
    fn env_validation_and_serialization_reject_publicly_mutated_unsupported_values() {
        macro_rules! assert_rejected_and_round_trips_invalid {
            ($field:literal, $mutate:expr) => {{
                let mut script = valid_env_script_for_mutation();
                $mutate(&mut script);

                let err = script.validate().unwrap_err().to_string();
                assert!(err.contains($field), "unexpected error: {err}");

                let serialized = serde_yaml::to_string(&script).unwrap();
                assert!(
                    serialized.contains(&format!("{}:", $field)),
                    "missing unsupported field after serialization: {serialized}"
                );
                let round_tripped: EnvTestScript = serde_yaml::from_str(&serialized).unwrap();
                let err = round_tripped.validate().unwrap_err().to_string();
                assert!(err.contains($field), "unexpected error: {err}");
            }};
        }

        assert_rejected_and_round_trips_invalid!(
            "no_progress_steps",
            |script: &mut EnvTestScript| script.limits.no_progress_steps = Some(1)
        );
        assert_rejected_and_round_trips_invalid!("max_vcd_bytes", |script: &mut EnvTestScript| {
            script.limits.max_vcd_bytes = Some(1)
        });
        assert_rejected_and_round_trips_invalid!("faults", |script: &mut EnvTestScript| {
            script.faults.push(
                serde_yaml::from_str(
                    r#"
id: unsupported
kind: missing_clock
"#,
                )
                .unwrap(),
            )
        });
        assert_rejected_and_round_trips_invalid!("verdict", |script: &mut EnvTestScript| {
            script.verdict = Some(serde_yaml::from_str("{}").unwrap())
        });
        assert_rejected_and_round_trips_invalid!("stimuli", |script: &mut EnvTestScript| {
            script.stimuli.push(
                serde_yaml::from_str(
                    r#"
target: { channel: x }
value: 1.0
"#,
                )
                .unwrap(),
            )
        });
    }

    #[test]
    fn explicitly_unsupported_env_fields_round_trip_as_invalid() {
        for (limits_extra, top_level_extra, expected_serialized, diagnostic) in [
            (
                "  no_progress_steps: null",
                "",
                "no_progress_steps: null",
                "no_progress_steps",
            ),
            (
                "  max_vcd_bytes: null",
                "",
                "max_vcd_bytes: null",
                "max_vcd_bytes",
            ),
            ("", "faults: null", "faults: null", "faults"),
            ("", "faults: []", "faults: []", "faults"),
            ("", "verdict: null", "verdict: null", "verdict"),
            ("", "stimuli: null", "stimuli: null", "stimuli"),
            ("", "stimuli: []", "stimuli: []", "stimuli"),
        ] {
            let yaml = format!(
                r#"
schema_version: "1.0"
inputs: {{ env: "twonode-env.yaml" }}
limits:
  max_steps: 10
{limits_extra}
assertions:
  - memory_value: {{ node: tester, address: 0x20010000, expected_value: 1 }}
{top_level_extra}
"#
            );
            let script: EnvTestScript = serde_yaml::from_str(&yaml).unwrap();

            let err = script.validate().unwrap_err().to_string();
            assert!(err.contains(diagnostic), "unexpected error: {err}");

            let serialized = serde_yaml::to_string(&script).unwrap();
            assert!(
                serialized.contains(expected_serialized),
                "missing explicit unsupported field after serialization: {serialized}"
            );
            let round_tripped: EnvTestScript = serde_yaml::from_str(&serialized).unwrap();
            let err = round_tripped.validate().unwrap_err().to_string();
            assert!(err.contains(diagnostic), "unexpected error: {err}");
        }
    }

    #[test]
    fn single_node_and_legacy_validation_reject_public_memory_node_mutations() {
        let mut single_node: TestScript = serde_yaml::from_str(
            r#"
schema_version: "1.0"
inputs: { firmware: "fw.elf" }
limits: { max_steps: 10 }
assertions:
  - memory_value: { address: 0x20010000, expected_value: 1 }
"#,
        )
        .unwrap();
        {
            let TestAssertion::MemoryValue(assertion) = &mut single_node.assertions[0] else {
                panic!("expected memory_value assertion");
            };
            assertion.memory_value.node = Some("tester".to_string());
        }
        let err = single_node.validate().unwrap_err().to_string();
        assert!(err.contains("single-node"), "unexpected error: {err}");

        let mut legacy: LegacyTestScriptV1 = serde_yaml::from_str(
            r#"
schema_version: 1
max_steps: 10
assertions:
  - memory_value: { address: 0x20010000, expected_value: 1 }
"#,
        )
        .unwrap();
        {
            let TestAssertion::MemoryValue(assertion) = &mut legacy.assertions[0] else {
                panic!("expected memory_value assertion");
            };
            assertion.memory_value.node = Some("tester".to_string());
        }
        let err = legacy.validate().unwrap_err().to_string();
        assert!(err.contains("legacy"), "unexpected error: {err}");
    }

    #[test]
    fn load_single_node_script_still_selects_v1_0_variant() {
        let script_path = write_temp_file(
            "single-node-script",
            r#"
schema_version: "1.0"
inputs:
  firmware: "fw.elf"
  system: "system.yaml"
limits:
  max_steps: 1000
"#,
        );

        assert!(matches!(
            load_test_script(&script_path).unwrap(),
            LoadedTestScript::V1_0(_)
        ));
    }

    #[test]
    fn single_node_script_rejects_node_qualified_memory_assertions() {
        let script_path = write_temp_file(
            "single-node-memory-node",
            r#"
schema_version: "1.0"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 1000
assertions:
  - memory_value:
      node: tester
      address: 0x20010000
      expected_value: 1
"#,
        );

        let err = load_test_script(&script_path).unwrap_err().to_string();
        assert!(err.contains("node"), "unexpected error: {err}");
        assert!(err.contains("single-node"), "unexpected error: {err}");
    }

    #[test]
    fn single_node_script_rejects_explicit_null_memory_node() {
        let script_path = write_temp_file(
            "single-node-memory-null-node",
            r#"
schema_version: "1.0"
inputs:
  firmware: "fw.elf"
limits:
  max_steps: 1000
assertions:
  - memory_value:
      node: null
      address: 0x20010000
      expected_value: 1
"#,
        );

        let err = load_test_script(&script_path).unwrap_err().to_string();
        assert!(err.contains("node"), "unexpected error: {err}");
        assert!(err.contains("single-node"), "unexpected error: {err}");
    }

    #[test]
    fn single_node_explicit_null_memory_node_round_trips_as_invalid() {
        let script: TestScript = serde_yaml::from_str(
            r#"
schema_version: "1.0"
inputs: { firmware: "fw.elf" }
limits: { max_steps: 10 }
assertions:
  - memory_value:
      node: null
      address: 0x20010000
      expected_value: 1
"#,
        )
        .unwrap();
        let err = script.validate().unwrap_err().to_string();
        assert!(err.contains("single-node"), "unexpected error: {err}");

        let serialized = serde_yaml::to_string(&script).unwrap();
        assert!(
            serialized.contains("node: null"),
            "explicit null node was lost during serialization: {serialized}"
        );
        let round_tripped: TestScript = serde_yaml::from_str(&serialized).unwrap();
        let err = round_tripped.validate().unwrap_err().to_string();
        assert!(err.contains("single-node"), "unexpected error: {err}");
    }

    #[test]
    fn ordinary_memory_assertion_without_node_round_trips_without_node_key() {
        let script: TestScript = serde_yaml::from_str(
            r#"
schema_version: "1.0"
inputs: { firmware: "fw.elf" }
limits: { max_steps: 10 }
assertions:
  - memory_value: { address: 0x20010000, expected_value: 1 }
"#,
        )
        .unwrap();
        script.validate().unwrap();

        let serialized = serde_yaml::to_string(&script).unwrap();
        assert!(
            !serialized.contains("node:"),
            "unexpected serialized script: {serialized}"
        );

        let round_tripped: TestScript = serde_yaml::from_str(&serialized).unwrap();
        round_tripped.validate().unwrap();
    }

    #[test]
    fn env_script_rejects_missing_or_blank_env_path() {
        for (name, yaml) in [
            (
                "blank-env",
                r#"
schema_version: "1.0"
inputs: { env: "   " }
limits: { max_steps: 10 }
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
"#,
            ),
            (
                "missing-env",
                r#"
schema_version: "1.0"
inputs: {}
limits: { max_steps: 10 }
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
"#,
            ),
        ] {
            let script_path = write_temp_file(name, yaml);
            let err = load_test_script(&script_path).unwrap_err().to_string();
            assert!(err.contains("env"), "unexpected error: {err}");
        }
    }

    #[test]
    fn env_script_requires_schema_v1_0_and_positive_step_limit() {
        for (name, yaml, diagnostic) in [
            (
                "wrong-env-schema",
                r#"
schema_version: "1.1"
inputs: { env: "twonode-env.yaml" }
limits: { max_steps: 10 }
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
"#,
                "schema_version",
            ),
            (
                "zero-env-steps",
                r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits: { max_steps: 0 }
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
"#,
                "max_steps",
            ),
        ] {
            let script_path = write_temp_file(name, yaml);
            let err = load_test_script(&script_path).unwrap_err().to_string();
            assert!(err.contains(diagnostic), "unexpected error: {err}");
        }
    }

    #[test]
    fn env_script_requires_memory_assertions_with_nodes() {
        for (name, yaml, diagnostic) in [
            (
                "no-assertions",
                r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits: { max_steps: 10 }
"#,
                "at least one assertion",
            ),
            (
                "missing-node",
                r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits: { max_steps: 10 }
assertions:
  - memory_value: { address: 0x20010000, expected_value: 1 }
"#,
                "node",
            ),
            (
                "blank-node",
                r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits: { max_steps: 10 }
assertions:
  - memory_value: { node: " ", address: 0x20010000, expected_value: 1 }
"#,
                "node",
            ),
            (
                "unsupported-assertion",
                r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml" }
limits: { max_steps: 10 }
assertions:
  - uart_contains: "PASS"
"#,
                "memory_value",
            ),
        ] {
            let script_path = write_temp_file(name, yaml);
            let err = load_test_script(&script_path).unwrap_err().to_string();
            assert!(err.contains(diagnostic), "unexpected error: {err}");
        }
    }

    #[test]
    fn env_script_rejects_combined_firmware_and_env_inputs() {
        let script_path = write_temp_file(
            "combined-inputs",
            r#"
schema_version: "1.0"
inputs: { env: "twonode-env.yaml", firmware: "fw.elf" }
limits: { max_steps: 10 }
assertions:
  - memory_value: { node: tester, address: 0x20010000, expected_value: 1 }
"#,
        );

        let err = load_test_script(&script_path).unwrap_err().to_string();
        assert!(err.contains("both") && err.contains("env") && err.contains("firmware"));
    }

    #[test]
    fn env_script_rejects_runner_options_it_cannot_honor() {
        for (name, extra, diagnostic) in [
            ("no-progress", "  no_progress_steps: 5", "no_progress_steps"),
            (
                "no-progress-null",
                "  no_progress_steps: null",
                "no_progress_steps",
            ),
            ("vcd", "  max_vcd_bytes: 1024", "max_vcd_bytes"),
            ("vcd-null", "  max_vcd_bytes: null", "max_vcd_bytes"),
            (
                "faults",
                "faults:\n  - id: x\n    kind: missing_clock",
                "faults",
            ),
            ("faults-empty", "faults: []", "faults"),
            ("faults-null", "faults: null", "faults"),
            ("verdict", "verdict: {}", "verdict"),
            ("verdict-null", "verdict: null", "verdict"),
            (
                "stimuli",
                "stimuli:\n  - target: { channel: x }\n    value: 1.0",
                "stimuli",
            ),
            ("stimuli-empty", "stimuli: []", "stimuli"),
            ("stimuli-null", "stimuli: null", "stimuli"),
        ] {
            let script_path = write_temp_file(
                name,
                &format!(
                    r#"
schema_version: "1.0"
inputs: {{ env: "twonode-env.yaml" }}
limits:
  max_steps: 10
{extra}
assertions:
  - memory_value: {{ node: tester, address: 0x20010000, expected_value: 1 }}
"#
                ),
            );

            let err = load_test_script(&script_path).unwrap_err().to_string();
            assert!(err.contains(diagnostic), "unexpected error: {err}");
        }
    }

    #[test]
    fn test_peripheral_descriptor_parsing() {
        let yaml = r#"
peripheral: "SPI"
version: "1.0"
registers:
  - id: "CR1"
    address_offset: 0x00
    size: 16
    access: "R/W"
    reset_value: 0x0000
    fields:
      - name: "SPE"
        bit_range: [6, 6]
        description: "SPI Enable"
  - id: "DR"
    address_offset: 0x0C
    size: 16
    access: "R/W"
    reset_value: 0x0000
    side_effects:
      on_read: "clear_rxne"
      on_write: "start_tx"
"#;
        let desc = PeripheralDescriptor::from_yaml(yaml).unwrap();
        assert_eq!(desc.peripheral, "SPI");
        assert_eq!(desc.registers.len(), 2);
        assert_eq!(desc.registers[0].id, "CR1");
        assert_eq!(desc.registers[0].access, Access::ReadWrite);
        assert_eq!(
            desc.registers[1].side_effects.as_ref().unwrap().on_read,
            Some("clear_rxne".to_string())
        );
    }

    #[test]
    fn uds_tester_assertion_parses_result_done() {
        let yaml = r#"
- uds_tester:
    id: "uds-tester"
    result: done
"#;
        let assertions: Vec<TestAssertion> = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(assertions.len(), 1);
        match &assertions[0] {
            TestAssertion::UdsTester(a) => {
                assert_eq!(a.uds_tester.id, "uds-tester");
                assert!(matches!(a.uds_tester.result, UdsTesterResult::Done));
            }
            other => panic!("expected UdsTester variant, got {:?}", other),
        }
    }
}

#[cfg(test)]
mod pin_map_tests {
    use super::*;

    #[test]
    fn chip_descriptor_parses_pins_and_ignores_extra_fields() {
        let yaml = r#"
name: "test-chip"
arch: "arm"
flash: { base: 0, size: "64K" }
ram: { base: 0x20000000, size: "16K" }
peripherals: []
pins:
  PC0: { gpio: gpioc, bit: 0, functions: [{ type: gpio, peripheral: gpioc }] }
  PB6: { gpio: gpioc, bit: 2 }
"#;
        let chip: ChipDescriptor = serde_yaml::from_str(yaml).expect("parse chip with pins");
        assert_eq!(chip.pins.len(), 2);
        assert_eq!(chip.pins["PC0"].gpio, "gpioc");
        assert_eq!(chip.pins["PC0"].bit, 0);
        // `functions:` in the YAML is ignored by PinLoc (serde ignores unknown fields).
        assert_eq!(chip.pins["PB6"].gpio, "gpioc");
        assert_eq!(chip.pins["PB6"].bit, 2);
    }

    #[test]
    fn chip_without_pins_defaults_to_empty() {
        let yaml = r#"
name: "no-pins"
arch: "arm"
flash: { base: 0, size: "64K" }
ram: { base: 0x20000000, size: "16K" }
peripherals: []
"#;
        let chip: ChipDescriptor = serde_yaml::from_str(yaml).expect("parse");
        assert!(chip.pins.is_empty());
    }
}

#[cfg(test)]
mod can_player_path_inline_tests {
    use super::*;

    /// `SystemManifest::from_file` inlines a `can-player` device's `path:`
    /// (resolved relative to the system yaml on disk) into `data:`, so the
    /// sim core itself only ever consumes inline text (keeps `std::fs` out
    /// of the wasm-safe core).
    #[test]
    fn from_file_inlines_can_player_path_into_data() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("s.log"), "(1.0) can0 123#11\n").unwrap();
        let yaml = r#"
name: "t"
chip: "chip.yaml"
external_devices:
  - type: "can-player"
    id: "p"
    connection: "bxcan1"
    config:
      path: "./s.log"
board_io: []
"#;
        let sys_path = dir.path().join("system.yaml");
        std::fs::write(&sys_path, yaml).unwrap();
        let m = SystemManifest::from_file(&sys_path).unwrap();
        let cfg = &m.external_devices[0].config;
        assert!(cfg.get("path").is_none());
        assert_eq!(
            cfg.get("data").unwrap().as_str().unwrap(),
            "(1.0) can0 123#11\n"
        );
    }

    /// Setting both `path:` and `data:` on a `can-player` device is
    /// ambiguous — silently letting `path` overwrite `data` (the prior
    /// behavior) hides a config mistake. Must error naming the device id
    /// and both keys.
    #[test]
    fn from_file_errors_when_both_path_and_data_set() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("s.log"), "(1.0) can0 123#11\n").unwrap();
        let yaml = r#"
name: "t"
chip: "chip.yaml"
external_devices:
  - type: "can-player"
    id: "p"
    connection: "bxcan1"
    config:
      path: "./s.log"
      data: "(1.0) can0 123#11\n"
board_io: []
"#;
        let sys_path = dir.path().join("system.yaml");
        std::fs::write(&sys_path, yaml).unwrap();
        let err = SystemManifest::from_file(&sys_path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("'p'"), "unexpected error: {msg}");
        assert!(msg.contains("path"), "unexpected error: {msg}");
        assert!(msg.contains("data"), "unexpected error: {msg}");
    }

    /// A `path:` pointing at a file that doesn't exist fails with an error
    /// that names the (resolved) path, not just an opaque io::Error.
    #[test]
    fn from_file_errors_on_nonexistent_path() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = r#"
name: "t"
chip: "chip.yaml"
external_devices:
  - type: "can-player"
    id: "p"
    connection: "bxcan1"
    config:
      path: "./missing.log"
board_io: []
"#;
        let sys_path = dir.path().join("system.yaml");
        std::fs::write(&sys_path, yaml).unwrap();
        let err = SystemManifest::from_file(&sys_path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("missing.log"), "unexpected error: {msg}");
    }

    /// A non-string `path:` value fails with an error naming the device id.
    #[test]
    fn from_file_errors_on_non_string_path() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = r#"
name: "t"
chip: "chip.yaml"
external_devices:
  - type: "can-player"
    id: "p"
    connection: "bxcan1"
    config:
      path: 123
board_io: []
"#;
        let sys_path = dir.path().join("system.yaml");
        std::fs::write(&sys_path, yaml).unwrap();
        let err = SystemManifest::from_file(&sys_path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("'p'"), "unexpected error: {msg}");
    }
}
