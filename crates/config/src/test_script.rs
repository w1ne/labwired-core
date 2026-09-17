// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

#![allow(dead_code)]
use crate::*;

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct TestInputs {
    pub firmware: String,
    pub system: Option<String>,
    /// A bare chip name (`stm32f103`), for firmware with no external devices
    /// or board I/O. Mutually exclusive with `system`; it saves authoring a
    /// manifest whose entire content would be a chip pointer and two empty
    /// lists. Anything with a device attached needs `system`.
    pub chip: Option<String>,
    /// Optional boot profile. Omitted (the default) means the faithful path:
    /// the skipped-BROM DRAM seed and a real dual-core release, with no
    /// firmware flash-thunks.
    ///
    /// `arduino-esp32` selects the FAST BOOT instead — the same profile
    /// `labwired snapshot capture` uses, which redirects a set of ROM/IDF
    /// entry points so an Arduino-ESP32 sketch reaches `setup()` quickly.
    /// Classic-ESP32 Arduino firmware does not boot on the faithful path yet,
    /// so without this a sketch like Ryan's bay-occupancy rig produced ~47
    /// bytes of UART and stopped — which meant the runner that owns `stimuli:`
    /// could not exercise the firmware at all.
    ///
    /// It is opt-in and named in the script on purpose: a run that took the
    /// fast boot should say so, rather than leaving a reader to infer which
    /// of two boot paths produced the result.
    pub profile: Option<String>,
}

/// A system, resolved once: the manifest plus the directory that relative
/// paths inside it (chip descriptor, firmware, peripheral schemas) resolve
/// against.
///
/// Runners take this rather than a manifest path so a run described only by
/// `inputs.chip` — a built-in chip with nothing attached — is a first-class
/// case instead of a file that has to be conjured on disk. It also means the
/// manifest is parsed once per run rather than re-read at every site that
/// needs to know the chip.
#[derive(Debug, Clone)]
pub struct ResolvedSystem {
    pub manifest: SystemManifest,
    base_dir: PathBuf,
    /// The manifest file this came from, if any. `None` for a built-in chip,
    /// where no such file exists — artifacts report it that way rather than
    /// inventing a path.
    source_path: Option<PathBuf>,
}

impl ResolvedSystem {
    pub fn from_manifest_file(path: &Path) -> Result<Self> {
        let manifest = SystemManifest::from_file(path)?;
        Ok(Self {
            manifest,
            base_dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            source_path: Some(path.to_path_buf()),
        })
    }

    /// A bare MCU: the named built-in chip, no external devices, no board I/O.
    pub fn from_builtin_chip(chip: &str) -> Result<Self> {
        Self::from_builtin_chip_with_plugins(chip, &|_| None)
    }

    /// Like [`Self::from_builtin_chip`], but bare names not found among the
    /// built-ins are offered to `plugin_chips` (chip name → embedded YAML)
    /// before giving up.
    pub fn from_builtin_chip_with_plugins(
        chip: &str,
        plugin_chips: &dyn Fn(&str) -> Option<&'static str>,
    ) -> Result<Self> {
        // Fail here rather than at first use, so an unknown name is a config
        // error before the run starts.
        ChipDescriptor::resolve_with(chip, Path::new("."), plugin_chips)?;
        Ok(Self {
            manifest: SystemManifest {
                parts: Vec::new(),
                schema_version: default_schema_version(),
                name: chip.to_string(),
                chip: chip.to_string(),
                cpu_hz: None,
                ..SystemManifest::default()
            },
            base_dir: PathBuf::from("."),
            source_path: None,
        })
    }

    /// The chip descriptor this system runs on.
    pub fn chip(&self) -> Result<ChipDescriptor> {
        self.chip_with_plugins(&|_| None)
    }

    /// Like [`Self::chip`], but bare names not found among the built-ins are
    /// offered to `plugin_chips` (chip name → embedded YAML) before giving up.
    pub fn chip_with_plugins(
        &self,
        plugin_chips: &dyn Fn(&str) -> Option<&'static str>,
    ) -> Result<ChipDescriptor> {
        ChipDescriptor::resolve_with(&self.manifest.chip, &self.base_dir, plugin_chips)
    }

    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }

    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }
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

pub(crate) fn default_stop_settle_steps() -> u64 {
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
    /// The **firmware** ended its own run by writing to the `simctl` device.
    ///
    /// Deliberately distinct from [`Self::Halt`]: a halt is the machine
    /// stopping, this is the firmware reporting a result. The exit code travels
    /// beside it in the run result's `firmware_exit_code`; a run that ends this
    /// way passed only if that code is `0`.
    FirmwareExit,
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
pub struct UartOrderedAssertion {
    pub uart_ordered: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct MotorSpeedReachedDetails {
    pub id: String,
    pub min_abs_rpm: f64,
    pub max_abs_rpm: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct MotorSpeedReachedAssertion {
    pub motor_speed_reached: MotorSpeedReachedDetails,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct MotorStateDetails {
    pub id: String,
    pub control_state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fault_contains: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct MotorStateAssertion {
    pub motor_state: MotorStateDetails,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ShutdownLatencyDetails {
    pub from_stimulus: StimulusTarget,
    /// One-based successful application occurrence for `from_stimulus`.
    #[serde(default = "default_first_occurrence")]
    pub stimulus_occurrence: u32,
    pub to_uart: String,
    /// One-based matching UART occurrence at or after the selected stimulus.
    #[serde(default = "default_first_occurrence")]
    pub uart_occurrence: u32,
    pub max_cycles: u64,
}

pub(crate) fn default_first_occurrence() -> u32 {
    1
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ShutdownLatencyAssertion {
    pub shutdown_latency: ShutdownLatencyDetails,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct StopReasonAssertion {
    pub expected_stop_reason: StopReason,
}

/// Assert the firmware ended its own run with a specific exit code.
///
/// ```yaml
/// assertions:
///   - firmware_exit: 0
/// ```
///
/// This is the assertion the `simctl` device exists for. `uart_contains: "PASS"`
/// proves some bytes reached a serial line; this proves the firmware reached
/// its own success path and said so. A run that ends any other way — timeout,
/// halt, fault — fails this assertion rather than passing by silence.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct FirmwareExitAssertion {
    pub firmware_exit: u32,
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
pub(crate) const EXPLICIT_NULL_NODE_SENTINEL: &str = "\u{0}labwired:explicit-null-node";

pub(crate) fn is_explicit_null_node(node: Option<&str>) -> bool {
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
pub(crate) struct SerializableMemoryValueDetails<'a> {
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
pub(crate) struct MemoryValueDetailsWire {
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

/// Assert SimMqttFabric collected a publish (send→collect), not only UART text.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct MqttFabricDetails {
    /// Exact topic that must appear on the fabric.
    pub topic: String,
    /// Optional substring that must appear in the latest payload for that topic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_contains: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct MqttFabricAssertion {
    pub mqtt_fabric: MqttFabricDetails,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceBudgetAssertion {
    pub resource_budget: ResourceBudgetDetails,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceBudgetDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_flash_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_ram_static_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_main_stack_bytes: Option<u64>,
}

impl ResourceBudgetDetails {
    pub fn validate(&self, index: usize) -> anyhow::Result<()> {
        let n = self.max_flash_bytes.is_some() as u8
            + self.max_ram_static_bytes.is_some() as u8
            + self.max_main_stack_bytes.is_some() as u8;
        if n != 1 {
            anyhow::bail!(
                "assertions[{index}]: resource_budget must set exactly one of \
                 max_flash_bytes, max_ram_static_bytes, max_main_stack_bytes"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum TestAssertion {
    /// Early for untagged serde: unique `resource_budget` key disambiguates.
    ResourceBudget(ResourceBudgetAssertion),
    UartContains(UartContainsAssertion),
    UartRegex(UartRegexAssertion),
    UartOrdered(UartOrderedAssertion),
    MotorSpeedReached(MotorSpeedReachedAssertion),
    MotorState(MotorStateAssertion),
    ShutdownLatency(ShutdownLatencyAssertion),
    ExpectedStopReason(StopReasonAssertion),
    FirmwareExit(FirmwareExitAssertion),
    MemoryValue(MemoryValueAssertion),
    UdsTester(UdsTesterAssertion),
    MqttFabric(MqttFabricAssertion),
    DisplayRegion(DisplayRegionAssertion),
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

/// A declarative stimulus (schema_version 1.2+), applied when `trigger` fires.
/// Reuses the [`FaultTrigger`] vocabulary; the first cut supports `at_start`
/// and `after_cycles` (the time-based triggers).
///
/// Two shapes, one per [`StimulusAction`]:
///
/// ```yaml
/// stimuli:
///   # drive a `sim_input` channel of an attached device
///   - target: { component: "ina219", channel: "current" }
///     trigger: !after_cycles { cycles: 50000 }
///     value: 1.5
///   # set a co-simulation signal a `cosim_models` input reads
///   - cosim_signal: { path: ui.touch.pressed, value: 1 }
///     trigger: !after_cycles { cycles: 8000000 }
/// ```
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(try_from = "StimulusSpecYaml", into = "StimulusSpecYaml")]
pub struct StimulusSpec {
    /// What the stimulus drives, and to what.
    pub action: StimulusAction,
    pub trigger: FaultTrigger,
}

/// What one [`StimulusSpec`] drives.
#[derive(Debug, Clone, PartialEq)]
pub enum StimulusAction {
    /// `target:` + `value:` — a `sim_input` channel, applied through the
    /// generic `Machine::set_input` path, so it works for any input device
    /// without per-type wiring. `value` is in the channel's engineering unit.
    Input { target: StimulusTarget, value: f64 },
    /// `cosim_signal: { path, value }` — a co-simulation signal store path
    /// (`ui.<part>.<field>`) that a `cosim_models` input reads.
    CosimSignal(CosimSignalStimulus),
}

/// `cosim_signal: { path, value }`: set the co-simulation signal `path` to
/// `value`. The run's co-simulation session applies it, so `path` must be one a
/// declared model input reads. For a boolean input 0 is false and anything else
/// true; a numeric input takes the number as given.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CosimSignalStimulus {
    pub path: String,
    pub value: f64,
}

impl StimulusSpec {
    /// The `sim_input` target, for an [`StimulusAction::Input`] stimulus.
    pub fn input_target(&self) -> Option<&StimulusTarget> {
        match &self.action {
            StimulusAction::Input { target, .. } => Some(target),
            StimulusAction::CosimSignal(_) => None,
        }
    }

    /// The value the stimulus sets, whichever shape it has.
    pub fn value(&self) -> f64 {
        match &self.action {
            StimulusAction::Input { value, .. } => *value,
            StimulusAction::CosimSignal(signal) => signal.value,
        }
    }
}

/// The YAML shape of a [`StimulusSpec`]: both forms' keys, exactly one form set.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StimulusSpecYaml {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<StimulusTarget>,
    #[serde(default)]
    trigger: FaultTrigger,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cosim_signal: Option<CosimSignalStimulus>,
}

impl TryFrom<StimulusSpecYaml> for StimulusSpec {
    type Error = String;

    fn try_from(raw: StimulusSpecYaml) -> Result<Self, Self::Error> {
        let action = match (raw.target, raw.value, raw.cosim_signal) {
            (Some(target), Some(value), None) => StimulusAction::Input { target, value },
            (None, None, Some(signal)) => StimulusAction::CosimSignal(signal),
            (Some(_), None, None) => return Err("missing field `value`".to_string()),
            (None, Some(_), None) => return Err("missing field `target`".to_string()),
            (None, None, None) => {
                return Err(
                    "a stimulus needs `target` + `value` (a device input channel) or \
                     `cosim_signal: { path, value }` (a co-simulation signal)"
                        .to_string(),
                )
            }
            (_, _, Some(_)) => {
                return Err(
                    "a `cosim_signal` stimulus carries its own `path` and `value`; it cannot \
                     also set `target` or `value`"
                        .to_string(),
                )
            }
        };
        Ok(Self {
            action,
            trigger: raw.trigger,
        })
    }
}

impl From<StimulusSpec> for StimulusSpecYaml {
    fn from(spec: StimulusSpec) -> Self {
        match spec.action {
            StimulusAction::Input { target, value } => Self {
                target: Some(target),
                trigger: spec.trigger,
                value: Some(value),
                cosim_signal: None,
            },
            StimulusAction::CosimSignal(signal) => Self {
                target: None,
                trigger: spec.trigger,
                value: None,
                cosim_signal: Some(signal),
            },
        }
    }
}

/// The bytes an [`UartInjectionSpec`] delivers: either a UTF-8 string (the
/// common case — command text, a line to echo) or an explicit byte array for
/// binary payloads that aren't valid UTF-8. Untagged so a script author writes
/// whichever is natural: `bytes: "hello\n"` or `bytes: [0x01, 0xFF, 0x00]`.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum UartInjectionBytes {
    Text(String),
    Raw(Vec<u8>),
}

impl UartInjectionBytes {
    /// Lower to the raw bytes that get pushed into the UART's RX queue.
    pub fn as_bytes(&self) -> Vec<u8> {
        match self {
            UartInjectionBytes::Text(s) => s.as_bytes().to_vec(),
            UartInjectionBytes::Raw(b) => b.clone(),
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            UartInjectionBytes::Text(s) => s.is_empty(),
            UartInjectionBytes::Raw(b) => b.is_empty(),
        }
    }
}

/// A declarative UART RX injection (schema_version 1.2+): push `bytes` into
/// the named `uart` peripheral's receive queue when `trigger` fires. Reuses
/// the [`FaultTrigger`] vocabulary; only the time-based triggers (`at_start`,
/// `after_cycles`) are wired today, mirroring [`StimulusSpec`].
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct UartInjectionSpec {
    /// The UART peripheral's bus name (e.g. `"uart1"`), resolved against the
    /// built machine when the run starts.
    pub uart: String,
    /// The bytes to deliver.
    pub bytes: UartInjectionBytes,
    #[serde(default)]
    pub trigger: FaultTrigger,
}

pub(crate) fn default_stack_paint() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct TestScript {
    pub schema_version: String,
    pub inputs: TestInputs,
    pub limits: TestLimits,
    #[serde(default)]
    pub assertions: Vec<TestAssertion>,
    /// When true (default), paint the main stack before run for high-water tracking.
    #[serde(default = "default_stack_paint")]
    pub stack_paint: bool,
    /// Faults to inject into the simulated silicon (schema_version 1.1+).
    #[serde(default)]
    pub faults: Vec<FaultSpec>,
    /// The safe-behaviour verdict for a fault-injection run (schema_version 1.1+).
    #[serde(default)]
    pub verdict: Option<Verdict>,
    /// Input stimuli to drive during the run (schema_version 1.2+).
    #[serde(default)]
    pub stimuli: Vec<StimulusSpec>,
    /// UART RX byte injections to deliver during the run (schema_version 1.2+).
    #[serde(default)]
    pub uart_injections: Vec<UartInjectionSpec>,
}

pub(crate) fn reject_explicit_memory_nodes(
    assertions: &[TestAssertion],
    script_kind: &str,
) -> Result<()> {
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

        // NOTE: an empty `inputs.firmware` is permitted. The schema requires the
        // key, but the faithful ESP32-C3 rom-boot path carries no debug ELF (the
        // flash image is the program the real mask ROM loads), so the builder
        // emits `firmware: ""`. `labwired test` resolves this: empty + --rom-boot
        // on an esp32c3 → the ELF-less rom-boot path; empty otherwise → a
        // "Missing firmware path" config error. Rejecting it here would 500 every
        // ELF-less rom-boot run before it starts.

        if self.limits.max_steps == 0 {
            anyhow::bail!("Limit 'max_steps' must be greater than zero");
        }

        if self.inputs.system.is_some() && self.inputs.chip.is_some() {
            anyhow::bail!(
                "inputs may set 'system' or 'chip', not both. Use 'chip' for a bare MCU \
                 and 'system' when external devices or board I/O are wired up."
            );
        }
        if let Some(chip) = &self.inputs.chip {
            if !is_builtin_chip_spec(chip) {
                anyhow::bail!(
                    "inputs.chip takes a built-in chip name such as 'stm32f103', not a path. \
                     Use 'system' with a manifest to point at your own descriptor."
                );
            }
            if embedded_chip_yaml(chip).is_none() {
                anyhow::bail!(
                    "unknown built-in chip '{chip}'. Available: {}",
                    BUILTIN_CHIP_NAMES.join(", ")
                );
            }
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
            match &s.action {
                StimulusAction::Input { target, .. } => {
                    if target.channel.trim().is_empty() {
                        anyhow::bail!("stimuli[{}]: target.channel cannot be empty", i);
                    }
                }
                StimulusAction::CosimSignal(signal) => {
                    if signal.path.trim().is_empty() {
                        anyhow::bail!("stimuli[{}]: cosim_signal.path cannot be empty", i);
                    }
                }
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
            if !s.value().is_finite() {
                anyhow::bail!("stimuli[{}]: value must be a finite number", i);
            }
        }
        for (index, assertion) in self.assertions.iter().enumerate() {
            if let TestAssertion::ShutdownLatency(assertion) = assertion {
                let details = &assertion.shutdown_latency;
                if details.stimulus_occurrence == 0 || details.uart_occurrence == 0 {
                    anyhow::bail!(
                        "assertions[{index}]: shutdown_latency occurrences are one-based"
                    );
                }
                if details.to_uart.is_empty() {
                    anyhow::bail!("assertions[{index}]: shutdown_latency.to_uart cannot be empty");
                }
                let available = self
                    .stimuli
                    .iter()
                    .filter(|stimulus| stimulus.input_target() == Some(&details.from_stimulus))
                    .count();
                if available < details.stimulus_occurrence as usize {
                    anyhow::bail!(
                        "assertions[{index}]: shutdown_latency selects stimulus occurrence {} but only {} matching stimuli are configured",
                        details.stimulus_occurrence,
                        available
                    );
                }
            }
            if let TestAssertion::DisplayRegion(assertion) = assertion {
                validate_display_region(index, &assertion.display_region)?;
            }
            if let TestAssertion::ResourceBudget(assertion) = assertion {
                assertion.resource_budget.validate(index)?;
            }
        }

        // UART RX injections require schema_version 1.2+.
        if !self.uart_injections.is_empty() && matches!(self.schema_version.as_str(), "1.0" | "1.1")
        {
            anyhow::bail!(
                "'uart_injections' require schema_version '1.2' (got '{}')",
                self.schema_version
            );
        }
        for (i, u) in self.uart_injections.iter().enumerate() {
            if u.uart.trim().is_empty() {
                anyhow::bail!("uart_injections[{}]: 'uart' cannot be empty", i);
            }
            if u.bytes.is_empty() {
                anyhow::bail!("uart_injections[{}]: 'bytes' cannot be empty", i);
            }
            // Only the time-based triggers are wired for injections today; the
            // register-access triggers need a write/read hook we haven't added
            // for the input path. Fail loud rather than silently never firing.
            match &u.trigger {
                FaultTrigger::AtStart | FaultTrigger::AfterCycles { .. } => {}
                other => anyhow::bail!(
                    "uart_injections[{}]: trigger {:?} is not yet supported for uart_injections \
                     (use at_start or after_cycles)",
                    i,
                    other
                ),
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
    pub uart_injections: Vec<UartInjectionSpec>,
    explicit_limits: EnvExplicitLimits,
    explicit_unsupported_fields: EnvExplicitUnsupportedFields,
}

/// A field whose parser records the difference between being absent and being
/// explicitly configured to a default value (or `null`). Environment scripts
/// use it to preserve their strict serialization contract and to distinguish
/// an absent setting from an invalid explicit `null`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) enum FieldPresence<T> {
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
pub(crate) struct EnvExplicitLimits {
    no_progress_steps: FieldPresence<u64>,
    max_vcd_bytes: FieldPresence<u64>,
    stop_when_assertions_pass: FieldPresence<bool>,
    stop_when_assertions_pass_settle_steps: FieldPresence<u64>,
    stop_when_assertions_pass_min_steps: FieldPresence<u64>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct EnvExplicitUnsupportedFields {
    faults: FieldPresence<Vec<FaultSpec>>,
    verdict: FieldPresence<Verdict>,
    stimuli: FieldPresence<Vec<StimulusSpec>>,
    uart_injections: FieldPresence<Vec<UartInjectionSpec>>,
}

/// Serialization keeps the user-visible environment contract strict in both
/// directions. Valid scripts omit defaulted limits; invalid parsed or
/// programmatically-mutated unsupported fields remain visible so a
/// serialize/parse cycle cannot make them look valid.
#[derive(Serialize)]
pub(crate) struct SerializableEnvTestScript<'a> {
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
    #[serde(skip_serializing_if = "Option::is_none")]
    uart_injections: Option<Option<&'a [UartInjectionSpec]>>,
}

#[derive(Serialize)]
pub(crate) struct SerializableEnvTestLimits {
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

pub(crate) fn serialize_unsupported_option_limit(
    explicit: FieldPresence<u64>,
    value: Option<u64>,
) -> Option<Option<u64>> {
    match value {
        Some(value) => Some(Some(value)),
        None if explicit.is_present() => Some(None),
        None => None,
    }
}

pub(crate) fn serialize_explicit_bool_limit(
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

pub(crate) fn serialize_explicit_defaulted_limit(
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

pub(crate) fn serialize_unsupported_sequence<'a, T>(
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

pub(crate) fn serialize_unsupported_verdict<'a>(
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
            uart_injections: serialize_unsupported_sequence(
                &self.explicit_unsupported_fields.uart_injections,
                &self.uart_injections,
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
pub(crate) struct EnvTestLimits {
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
pub(crate) struct EnvTestScriptWire {
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
    #[serde(default)]
    uart_injections: FieldPresence<Vec<UartInjectionSpec>>,
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
            uart_injections,
        } = wire;
        let (limits, explicit_limits) = wire_limits.into_parts();
        let explicit_unsupported_fields = EnvExplicitUnsupportedFields {
            faults: faults.clone(),
            verdict: verdict.clone(),
            stimuli: stimuli.clone(),
            uart_injections: uart_injections.clone(),
        };
        Ok(Self {
            schema_version,
            inputs,
            limits,
            assertions,
            faults: faults.into_value().unwrap_or_default(),
            verdict: verdict.into_value(),
            stimuli: stimuli.into_value().unwrap_or_default(),
            uart_injections: uart_injections.into_value().unwrap_or_default(),
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
        if self
            .explicit_unsupported_fields
            .uart_injections
            .is_present()
            || !self.uart_injections.is_empty()
        {
            anyhow::bail!("Environment test scripts do not support 'uart_injections'");
        }

        // An environment script MAY assert nothing. Such a run is
        // observational: it reports what each node printed and whether anything
        // faulted, and its `status` is decided by the safety stop alone (an
        // empty assertion list is vacuously satisfied). That is the same
        // contract a single-machine script has always had, and it is what a
        // hosted verify needs — "compile every chip and run the world without
        // faulting" is a claim about faults, not about an author's oracle.
        //
        // A script that DOES carry assertions is still held to all of the rules
        // below, so a gate cannot quietly weaken itself: it either states what
        // it checks, or it visibly checks nothing.

        for (index, assertion) in self.assertions.iter().enumerate() {
            // UART assertions carry no node id, so there is no node rule to
            // enforce here. The world runner evaluates them against every
            // node's captured stream.
            if matches!(
                assertion,
                TestAssertion::UartContains(_)
                    | TestAssertion::UartRegex(_)
                    | TestAssertion::UartOrdered(_)
            ) {
                continue;
            }
            let TestAssertion::MemoryValue(memory) = assertion else {
                anyhow::bail!(
                    "Environment test scripts support only uart_contains / uart_regex / \
                     uart_ordered and node-qualified memory_value assertions (assertions[{index}]); \
                     the world runner cannot observe the others"
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

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub(crate) enum LegacySchemaVersion {
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

#[cfg(test)]
#[path = "lib_stimuli_tests.rs"]
mod stimuli_tests;

#[cfg(test)]
#[path = "lib_uart_injection_tests.rs"]
mod uart_injection_tests;
