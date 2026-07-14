// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
pub struct NodeConfig {
    pub id: String,
    pub system: String,   // Path to SystemManifest
    pub firmware: String, // Path to ELF
    #[serde(default)]
    pub config_overrides: HashMap<String, serde_yaml::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EnvironmentManifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub name: String,
    pub nodes: Vec<NodeConfig>,
    #[serde(default)]
    pub interconnects: Vec<InterconnectConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InterconnectConfig {
    pub r#type: String,     // "uart_cross_link", "virtual_switch", etc.
    pub nodes: Vec<String>, // List of node IDs
    #[serde(default)]
    pub config: HashMap<String, serde_yaml::Value>,
}

impl EnvironmentManifest {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let f = std::fs::File::open(path)?;
        serde_yaml::from_reader(f).context("Failed to parse Environment Manifest")
    }
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

#[derive(Debug, Serialize, Clone)]
pub struct MemoryValueDetails {
    pub address: u64,
    pub expected_value: u64,
    #[serde(default)]
    pub mask: Option<u64>,
    /// Value width to read at `address`. Accepts bytes (1/2/4) or the
    /// equivalent bit width (8/16/32); both map to a u8/u16/u32 read.
    /// Defaults to a 32-bit (u32) word.
    #[serde(default)]
    pub size: Option<u8>,
    /// Target node for a multi-node environment assertion. Single-node scripts
    /// leave this unset and continue to use the existing machine path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    #[serde(skip)]
    node_was_explicit: bool,
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
        let node_was_explicit = wire.node.is_present();
        Ok(Self {
            address: wire.address,
            expected_value: wire.expected_value,
            mask: wire.mask,
            size: wire.size,
            node: wire.node.into_value(),
            node_was_explicit,
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
            if memory.memory_value.node_was_explicit {
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
#[derive(Debug, Serialize, Clone)]
pub struct EnvTestScript {
    pub schema_version: String,
    pub inputs: EnvTestInputs,
    #[serde(serialize_with = "serialize_env_test_limits")]
    pub limits: TestLimits,
    #[serde(default)]
    pub assertions: Vec<TestAssertion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub faults: Vec<FaultSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stimuli: Vec<StimulusSpec>,
    #[serde(skip)]
    explicit_limits: EnvExplicitLimits,
    #[serde(skip)]
    explicit_unsupported_fields: EnvExplicitUnsupportedFields,
}

/// A field whose parser records the difference between being absent and being
/// explicitly configured to a default value (or `null`). Environment scripts
/// use this for single-node-only early-stop tuning: every explicit occurrence
/// is rejected instead of being normalized away during deserialization.
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
    no_progress_steps: bool,
    max_vcd_bytes: bool,
    stop_when_assertions_pass: bool,
    stop_when_assertions_pass_settle_steps: bool,
    stop_when_assertions_pass_min_steps: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct EnvExplicitUnsupportedFields {
    faults: bool,
    verdict: bool,
    stimuli: bool,
}

/// Serialize only settings that environment scripts can honor. In particular,
/// this prevents a valid script from gaining explicit single-node early-stop
/// defaults and then being rejected when it is loaded again.
fn serialize_env_test_limits<S>(
    limits: &TestLimits,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    #[derive(Serialize)]
    struct SerializableEnvTestLimits {
        max_steps: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_cycles: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        max_uart_bytes: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        wall_time_ms: Option<u64>,
    }

    SerializableEnvTestLimits {
        max_steps: limits.max_steps,
        max_cycles: limits.max_cycles,
        max_uart_bytes: limits.max_uart_bytes,
        wall_time_ms: limits.wall_time_ms,
    }
    .serialize(serializer)
}

/// Wire shape for an environment limits block. It resolves to `TestLimits`
/// after retaining presence information for settings the environment runner
/// intentionally does not implement.
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
            no_progress_steps: self.no_progress_steps.is_present(),
            max_vcd_bytes: self.max_vcd_bytes.is_present(),
            stop_when_assertions_pass: self.stop_when_assertions_pass.is_present(),
            stop_when_assertions_pass_settle_steps: self
                .stop_when_assertions_pass_settle_steps
                .is_present(),
            stop_when_assertions_pass_min_steps: self
                .stop_when_assertions_pass_min_steps
                .is_present(),
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
/// `TestLimits` for runners, while this shape preserves which unsupported
/// early-stop fields were explicitly supplied in YAML.
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
        let (limits, explicit_limits) = wire.limits.into_parts();
        let explicit_unsupported_fields = EnvExplicitUnsupportedFields {
            faults: wire.faults.is_present(),
            verdict: wire.verdict.is_present(),
            stimuli: wire.stimuli.is_present(),
        };
        Ok(Self {
            schema_version: wire.schema_version,
            inputs: wire.inputs,
            limits,
            assertions: wire.assertions,
            faults: wire.faults.into_value().unwrap_or_default(),
            verdict: wire.verdict.into_value(),
            stimuli: wire.stimuli.into_value().unwrap_or_default(),
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

        if self.explicit_limits.no_progress_steps {
            anyhow::bail!("Environment test scripts do not support 'limits.no_progress_steps'");
        }
        if self.explicit_limits.max_vcd_bytes {
            anyhow::bail!("Environment test scripts do not support 'limits.max_vcd_bytes'");
        }
        if self.explicit_limits.stop_when_assertions_pass {
            anyhow::bail!(
                "Environment test scripts do not support 'limits.stop_when_assertions_pass'"
            );
        }
        if self.explicit_limits.stop_when_assertions_pass_settle_steps {
            anyhow::bail!(
                "Environment test scripts do not support 'limits.stop_when_assertions_pass_settle_steps'"
            );
        }
        if self.explicit_limits.stop_when_assertions_pass_min_steps {
            anyhow::bail!(
                "Environment test scripts do not support 'limits.stop_when_assertions_pass_min_steps'"
            );
        }
        if self.explicit_unsupported_fields.faults {
            anyhow::bail!("Environment test scripts do not support 'faults'");
        }
        if self.explicit_unsupported_fields.verdict {
            anyhow::bail!("Environment test scripts do not support 'verdict'");
        }
        if self.explicit_unsupported_fields.stimuli {
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
            let has_node = memory
                .memory_value
                .node
                .as_deref()
                .is_some_and(|node| !node.trim().is_empty());
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
                "early-pass",
                "  stop_when_assertions_pass: true",
                "stop_when_assertions_pass",
            ),
            (
                "early-pass-false",
                "  stop_when_assertions_pass: false",
                "stop_when_assertions_pass",
            ),
            (
                "early-pass-null",
                "  stop_when_assertions_pass: null",
                "stop_when_assertions_pass",
            ),
            (
                "early-pass-settle",
                "  stop_when_assertions_pass_settle_steps: 100000",
                "stop_when_assertions_pass_settle_steps",
            ),
            (
                "early-pass-settle-null",
                "  stop_when_assertions_pass_settle_steps: null",
                "stop_when_assertions_pass_settle_steps",
            ),
            (
                "early-pass-minimum",
                "  stop_when_assertions_pass_min_steps: 0",
                "stop_when_assertions_pass_min_steps",
            ),
            (
                "early-pass-minimum-null",
                "  stop_when_assertions_pass_min_steps: null",
                "stop_when_assertions_pass_min_steps",
            ),
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
