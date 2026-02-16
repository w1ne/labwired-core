// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

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
    Unknown,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MemoryRange {
    pub base: u64,
    pub size: String, // e.g. "128KB"
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PeripheralConfig {
    pub id: String,
    pub r#type: String, // "uart", "timer", "gpio", etc.
    pub base_address: u64,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub irq: Option<u32>,
    #[serde(default)]
    pub config: HashMap<String, serde_yaml::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChipDescriptor {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub name: String,
    pub arch: Arch, // Parsed from string
    pub flash: MemoryRange,
    pub ram: MemoryRange,
    pub peripherals: Vec<PeripheralConfig>,
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
#[serde(rename_all = "lowercase")]
pub enum BoardIoKind {
    Led,
    Button,
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
        let f = std::fs::File::open(path)?;
        serde_yaml::from_reader(f).context("Failed to parse System Manifest")
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
        let arch = match ir.arch.to_uppercase().as_str() {
            "CM3" | "CM4" | "CM7" | "ARM" => Arch::Arm,
            "RISCV" | "RV32" => Arch::RiscV,
            _ => Arch::Arm, // Default to Arm for CMSIS-SVD
        };

        Self {
            name: ir.name,
            arch,
            flash: MemoryRange {
                base: 0,
                size: "0".to_string(),
            }, // IR doesn't carry memory map yet
            ram: MemoryRange {
                base: 0,
                size: "0".to_string(),
            },
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
                        config: std::collections::HashMap::from([(
                            "internal_ir_peripheral".to_string(),
                            serde_yaml::to_value(p).unwrap(),
                        )]),
                    }
                })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct TestInputs {
    pub firmware: String,
    pub system: Option<String>,
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
    MemoryViolation,
    DecodeError,
    Halt,
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

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct MemoryValueDetails {
    pub address: u64,
    pub expected_value: u64,
    #[serde(default)]
    pub mask: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct MemoryValueAssertion {
    pub memory_value: MemoryValueDetails,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum TestAssertion {
    UartContains(UartContainsAssertion),
    UartRegex(UartRegexAssertion),
    ExpectedStopReason(StopReasonAssertion),
    MemoryValue(MemoryValueAssertion),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct TestScript {
    pub schema_version: String,
    pub inputs: TestInputs,
    pub limits: TestLimits,
    #[serde(default)]
    pub assertions: Vec<TestAssertion>,
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
        if self.schema_version != "1.0" {
            anyhow::bail!(
                "Unsupported schema_version '{}'. Supported versions: '1.0'",
                self.schema_version
            );
        }

        if self.inputs.firmware.trim().is_empty() {
            anyhow::bail!("Input 'firmware' path cannot be empty");
        }

        if self.limits.max_steps == 0 {
            anyhow::bail!("Limit 'max_steps' must be greater than zero");
        }

        Ok(())
    }
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
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum LoadedTestScript {
    V1_0(TestScript),
    LegacyV1(LegacyTestScriptV1),
}

/// Load a CI test script from YAML.
///
/// Supported formats:
/// - v1.0 (frozen): `schema_version: \"1.0\"` with `inputs` + `limits` + `assertions`.
/// - legacy v1 (deprecated): `schema_version: 1` with `max_steps` at the top level.
pub fn load_test_script<P: AsRef<Path>>(path: P) -> Result<LoadedTestScript> {
    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read test script at {:?}", path.as_ref()))?;

    match serde_yaml::from_str::<TestScript>(&contents) {
        Ok(script) => {
            script.validate()?;
            Ok(LoadedTestScript::V1_0(script))
        }
        Err(v1_err) => {
            let looks_like_legacy_v1 = serde_yaml::from_str::<serde_yaml::Value>(&contents)
                .ok()
                .and_then(|v| v.get("schema_version").cloned())
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
    let s: Size = size_str
        .parse()
        .map_err(|e| anyhow::anyhow!("Invalid size format: {}", e))?;
    let bytes: SpecificSize<Byte> = s.into();
    Ok(bytes.value() as u64)
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
}
