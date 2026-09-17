// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

pub mod expr;
pub mod rules;

pub use rules::{
    compile_rules, validate_rule_names, Action, BitFieldSpec, CompiledAction, CompiledRule, Event,
    FifoOverflow, FifoSpec, FrameSpec, PinEdge, RegBits, Rule, RuleCompileError, RuleNames,
};

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

/// [`deserialize_u64_lax`] for an optional field: absent ⇒ `None`, present ⇒
/// the same int-or-underscored-string parse. YAML 1.2 does not accept `_` in a
/// number, so `cpu_hz: 160_000_000` arrives as a *string* — the corpus is
/// written that way throughout and this is what makes it a clock.
fn deserialize_opt_u64_lax<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let raw = Option::<serde_yaml::Value>::deserialize(deserializer)?;
    match raw {
        None | Some(serde_yaml::Value::Null) => Ok(None),
        Some(v) => deserialize_u64_lax(v).map(Some).map_err(|e| {
            serde::de::Error::custom(format!("cpu_hz is not a whole number of hertz: {e}"))
        }),
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
    /// AVR8 (ATmega328P / classic Arduino Nano).
    #[serde(alias = "avr8", alias = "atmega328p")]
    Avr,
    Unknown,
}

/// Deserialize a memory size, in bytes, from the human form the chip YAMLs use.
///
/// The wire format is unchanged — `128KB`, `1.5 MiB`, `0x20000` and a bare
/// `131072` all still load. What changed is that the parse happens HERE, once,
/// at the boundary, instead of at each of the 39 places that used to call
/// `parse_size(&chip.ram.size)` on a `String` field.
///
/// That mattered: 18 of those call sites ended `.unwrap_or(0)`. A size that
/// failed to parse did not fail the run — it silently became **zero bytes of
/// RAM**, and the eleven ESP32-C3 suites computing `sp_top = ram.base + size`
/// got a stack pointer at the very bottom of RAM. Wrong, and green. Making the
/// field a `u64` deletes the fallible read, so that state cannot be
/// constructed: a bad size is now a load error naming the field.
fn deserialize_size<'de, D>(deserializer: D) -> Result<u64, D::Error>
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
        IntOrString::String(s) => parse_size(&s).map_err(serde::de::Error::custom),
    }
}

/// Serialize a size back as a bare byte count.
///
/// Deliberately NOT re-rendered in a unit, because the units here do not mean
/// what they look like. Measured against the real parser:
///
/// ```text
///   1KB  -> 1024          1KiB -> 1024
///   1MB  -> 1_000_000     1MiB -> 1_048_576
/// ```
///
/// `KB` is BINARY and `MB` is DECIMAL — inconsistent with each other, inside
/// one parser. So re-rendering `1048576` as `1MB` would read back as
/// `1_000_000` and quietly shrink a chip's flash by 4.9% on every round trip.
/// A bare byte count says exactly one thing and `parse_size` reads it back
/// unchanged.
///
/// (That asymmetry was also a live fidelity bug: nine committed chips spelled
/// flash in `MB`, so e.g. esp32s3 modelled 16_000_000 bytes where the part has
/// 16 MiB = 16_777_216. All nine have since been rewritten in `KB`, and
/// `labwired_core::tests::chip_memory_sizes` fails the build if a new chip
/// reintroduces the spelling.)
fn serialize_size<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_u64(*value)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MemoryRange {
    #[serde(deserialize_with = "deserialize_u64_lax")]
    pub base: u64,
    /// Size in BYTES. Parsed from the YAML's human form at load time.
    #[serde(
        deserialize_with = "deserialize_size",
        serialize_with = "serialize_size"
    )]
    pub size: u64,
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
    /// Size in BYTES — same boundary parse as [`MemoryRange::size`].
    #[serde(
        deserialize_with = "deserialize_size",
        serialize_with = "serialize_size"
    )]
    pub size: u64,
    /// Optional env var naming a path to a raw binary loaded into this region at
    /// `base` (e.g. a chip's mask ROM dump). Used for copyrighted vendor blobs
    /// that can't be committed — the region stays zero-filled if unset/missing.
    #[serde(default)]
    pub image_env: Option<String>,
    /// Fill the region with 0xFF instead of 0x00.
    ///
    /// ⚠️ A FLASH REGION IS NOT A RAM HOLE. Regions install as zeros, which is
    /// right for a RAM window and WRONG for flash: an erased flash byte is
    /// 0xFF, and a blank user-data page on a real EFR32MG26 reads
    /// `ffffffff ffffffff …` (measured over SWD on BRD2709A, 2026-09-03).
    /// Without this, firmware that reads its settings page before writing one
    /// sees zeros in the twin and ones on the bench — and "is this page blank?"
    /// is the first question any persistence routine asks.
    #[serde(default)]
    pub erased: bool,
}

/// One clock-enable bit a peripheral's clock depends on.
///
/// `reg` is a symbolic enable-register name — either a peripheral-enable
/// register ("apb1enr", "apb2enr", "ahbenr", "ahb2enr", "APBCMASK", …) or a
/// clock-source register ("cr", "crrcr", …). The bus maps it to the named
/// controller's actual offset at build time, so the same name resolves
/// correctly on F1 vs L4 vs L0 (RCC) and on SAMD21 PM / SAMD51 MCLK. A name the
/// active controller does not expose is a hard build error, never a silent
/// "never gate".
///
/// `controller` selects which peripheral owns that register (default `"rcc"`
/// preserves STM32 configs; SAMD21 uses `"pm"`).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClockGate {
    /// Symbolic enable-register name, e.g. "apb1enr" / "apb2enr" / "ahbenr" / "crrcr" / "APBCMASK".
    pub reg: String,
    /// Bit position within that register that must be **set** for the
    /// peripheral to be clocked.
    pub bit: u8,
    /// Clock controller peripheral id. Default `"rcc"` preserves STM32 configs.
    #[serde(default = "default_clock_controller")]
    pub controller: String,
}

fn default_clock_controller() -> String {
    "rcc".into()
}

/// A peripheral's `clock:` declaration: **every** listed RCC bit must be set for
/// the peripheral to answer the CPU.
///
/// One key, one mechanism. Silicon can withhold a peripheral's clock for more
/// than one reason — the bus-enable bit in an `xxxENR` register is the common
/// one, but a peripheral with its own *kernel* clock (the STM32L0 RNG runs off
/// HSI48) is equally dead when that source was never started. Both are "an RCC
/// bit that must be set", so both are entries in this one list rather than a
/// second config key and a second check somewhere else in the engine. See
/// [`crate::PeripheralConfig::clock`].
///
/// Accepts either shape in yaml, so every config written against the original
/// single-gate form keeps working verbatim:
///
/// ```yaml
/// clock: { reg: "apb1enr", bit: 21 }              # one bit
/// clock:                                          # several, all required
///   - { reg: "ahbenr", bit: 20 }
///   - { reg: "crrcr",  bit: 1  }
/// ```
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum ClockGates {
    /// A single required bit — the original `clock: { reg, bit }` form.
    One(ClockGate),
    /// Several required bits, ANDed together.
    All(Vec<ClockGate>),
}

impl ClockGates {
    /// Every bit this declaration requires, in declaration order.
    pub fn as_slice(&self) -> &[ClockGate] {
        match self {
            Self::One(gate) => std::slice::from_ref(gate),
            Self::All(gates) => gates.as_slice(),
        }
    }
}

/// Parsed `irq` YAML: a line number plus optional `controller@line` prefix.
#[derive(Default)]
struct IrqTarget {
    line: Option<u32>,
    controller: Option<String>,
}

fn irq_line_from_number(n: &serde_yaml::Number) -> Result<u32, String> {
    if let Some(u) = n.as_u64() {
        u32::try_from(u).map_err(|_| format!("irq: line {u} is out of range"))
    } else if let Some(i) = n.as_i64() {
        u32::try_from(i).map_err(|_| format!("irq: {i} is not a valid line number"))
    } else {
        Err(format!("irq: expected an integer line number, got {n}"))
    }
}

fn parse_irq_string(s: &str) -> Result<IrqTarget, String> {
    let s = s.trim();
    if let Some((controller, line)) = s.split_once('@') {
        if controller.is_empty() || line.is_empty() || !line.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("irq: expected controller@<line>, got `{s}`"));
        }
        let line: u32 = line
            .parse()
            .map_err(|_| format!("irq: line `{line}` is out of range"))?;
        Ok(IrqTarget {
            line: Some(line),
            controller: Some(controller.to_string()),
        })
    } else if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        let line: u32 = s
            .parse()
            .map_err(|_| format!("irq: line `{s}` is out of range"))?;
        Ok(IrqTarget {
            line: Some(line),
            controller: None,
        })
    } else {
        Err(format!(
            "irq: expected a line number or controller@line, got `{s}`"
        ))
    }
}

fn parse_irq_value(value: serde_yaml::Value) -> Result<IrqTarget, String> {
    match value {
        serde_yaml::Value::Null => Ok(IrqTarget::default()),
        serde_yaml::Value::Number(n) => Ok(IrqTarget {
            line: Some(irq_line_from_number(&n)?),
            controller: None,
        }),
        serde_yaml::Value::String(s) => parse_irq_string(&s),
        other => Err(format!(
            "irq: expected a line number or controller@line, got {other:?}"
        )),
    }
}

fn deserialize_irq_target<'de, D>(deserializer: D) -> Result<IrqTarget, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_yaml::Value::deserialize(deserializer)?;
    parse_irq_value(value).map_err(serde::de::Error::custom)
}

#[derive(Deserialize)]
struct PeripheralConfigWire {
    id: String,
    r#type: String,
    #[serde(deserialize_with = "deserialize_u64_lax")]
    base_address: u64,
    #[serde(default)]
    size: Option<String>,
    #[serde(default, deserialize_with = "deserialize_irq_target")]
    irq: IrqTarget,
    #[serde(default)]
    irq_controller: Option<String>,
    #[serde(default)]
    clock: Option<ClockGates>,
    #[serde(default)]
    config: HashMap<String, serde_yaml::Value>,
    #[serde(flatten)]
    extra: HashMap<String, serde_yaml::Value>,
}

/// One MMIO peripheral instance in a chip descriptor.
///
/// `irq` is a line number. `controller@line` also sets [`Self::irq_controller`]:
///
/// ```yaml
/// irq: 2
/// irq: nvic@2
/// ```
///
/// Unknown instance keys flatten into [`Self::config`]. Nested `config:` wins
/// on a colliding key:
///
/// ```yaml
/// - id: uart0
///   type: nrf52840_uart
///   base_address: 0x40002000
///   easyDMA: true
/// ```
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(from = "PeripheralConfigWire")]
pub struct PeripheralConfig {
    pub id: String,
    pub r#type: String, // "uart", "timer", "gpio", etc.
    #[serde(deserialize_with = "deserialize_u64_lax")]
    pub base_address: u64,
    #[serde(default)]
    pub size: Option<String>,
    /// IRQ line. YAML `irq: 2` or `irq: nvic@2`.
    #[serde(default)]
    pub irq: Option<u32>,
    /// Controller id from `irq: nvic@2` sugar. `None` when YAML is a bare line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub irq_controller: Option<String>,
    /// Optional RCC clock-gate: the RCC bits that must ALL be set for this
    /// peripheral to answer the CPU. `None` → the peripheral is never gated
    /// (the safe default — existing configs and firmware that never enable a
    /// clock keep working unchanged).
    #[serde(default)]
    pub clock: Option<ClockGates>,
    /// Instance knobs. Unknown top-level keys flatten here (`easyDMA: true`).
    /// Nested `config:` wins on a colliding key.
    #[serde(default)]
    pub config: HashMap<String, serde_yaml::Value>,
}

impl From<PeripheralConfigWire> for PeripheralConfig {
    fn from(wire: PeripheralConfigWire) -> Self {
        let mut config = wire.config;
        for (key, value) in wire.extra {
            config.entry(key).or_insert(value);
        }
        Self {
            id: wire.id,
            r#type: wire.r#type,
            base_address: wire.base_address,
            size: wire.size,
            irq: wire.irq.line,
            irq_controller: wire.irq.controller.or(wire.irq_controller),
            clock: wire.clock,
            config,
        }
    }
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

/// The analog input function of one pad: the ADC peripheral (by descriptor
/// `id`) that samples it, and the input channel number within that ADC.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdcPinFn {
    pub peripheral: String,
    pub channel: u8,
}

/// A chip's digital input thresholds, as ratios of its I/O supply
/// ([`ChipDescriptor::io_voltage_v`]): a pad reads low at or below `vil` and
/// high at or above `vih`, and the band between is where a Schmitt input keeps
/// its previous level.
///
/// Transcribed from the datasheet's DC characteristics (the guaranteed VIL
/// maximum and VIH minimum), never guessed: co-simulation turns a model's node
/// voltage into the level the firmware reads through these two numbers, so a
/// wrong ratio moves every edge a circuit produces.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(try_from = "GpioInputThresholdsYaml", into = "GpioInputThresholdsYaml")]
pub struct GpioInputThresholds {
    /// Highest input voltage guaranteed to read low, as a fraction of VDD.
    pub vil: f64,
    /// Lowest input voltage guaranteed to read high, as a fraction of VDD.
    pub vih: f64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GpioInputThresholdsYaml {
    vil: f64,
    vih: f64,
}

impl TryFrom<GpioInputThresholdsYaml> for GpioInputThresholds {
    type Error = String;

    fn try_from(raw: GpioInputThresholdsYaml) -> Result<Self, Self::Error> {
        let GpioInputThresholdsYaml { vil, vih } = raw;
        if !(vil.is_finite() && vih.is_finite() && 0.0 < vil && vil < vih && vih < 1.0) {
            return Err(format!(
                "gpio_input_thresholds must satisfy 0 < vil < vih < 1 (ratios of io_voltage_v); \
                 got vil {vil}, vih {vih}"
            ));
        }
        Ok(Self { vil, vih })
    }
}

impl From<GpioInputThresholds> for GpioInputThresholdsYaml {
    fn from(thresholds: GpioInputThresholds) -> Self {
        Self {
            vil: thresholds.vil,
            vih: thresholds.vih,
        }
    }
}

/// `io_voltage_v`: a positive, finite supply voltage.
fn deserialize_io_voltage<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;
    let volts = Option::<f64>::deserialize(deserializer)?;
    match volts {
        Some(v) if !(v.is_finite() && v > 0.0) => Err(D::Error::custom(format!(
            "io_voltage_v must be a positive number of volts; got {v}"
        ))),
        other => Ok(other),
    }
}

/// Which family's atomic register aliases a chip implements.
///
/// Both families alias every peripheral register three more times at a 0x1000
/// stride inside the peripheral's window, and both let firmware do a bit-level
/// read-modify-write with one store. They do NOT agree on which alias means
/// what, and the two orders overlap enough that using the wrong one is silent:
/// an RP2040 SET (`+0x2000`) is an EFR32 CLR, so a driver "enabling" a clock
/// would disable it and every later access to that block would read zero.
#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AtomicAliasFlavour {
    /// No aliases. An alias address is ordinary (usually unmapped) MMIO.
    #[default]
    None,
    /// RP2040 / RP2350: `+0x1000` XOR, `+0x2000` SET, `+0x3000` CLR
    /// (`hw_xor_bits` / `hw_set_bits` / `hw_clear_bits`, pico-sdk
    /// `hardware/address_mapped.h`). The HAL drives nearly all register setup
    /// through them, so without this an unmodified image faults on the first
    /// `hw_set_bits`.
    Rp2040,
    /// Silicon Labs EFR32/EFM32 Series 2: `+0x1000` SET, `+0x2000` CLR,
    /// `+0x3000` TGL (EFR32xG26 Reference Manual rev 1.0, "Peripheral Bit Set
    /// and Clear"; `emlib` writes `PERIPH->REG_SET = mask`). Series-2 emlib and
    /// the Gecko SDK use the aliases for essentially every enable bit — CMU
    /// clock gating, GPIO ROUTEEN, USART/EUSART enables — so a Series-2 image
    /// cannot configure a single peripheral without them.
    Efr32s2,
}

impl AtomicAliasFlavour {
    /// The op an alias index (`(addr >> 12) & 0x3`) means for this family, or
    /// `None` for index 0 (the register itself) and for a chip with no aliases.
    #[inline]
    pub fn op_for_index(self, index: u64) -> Option<AtomicAliasOp> {
        match (self, index) {
            (Self::None, _) | (_, 0) => None,
            (Self::Rp2040, 1) => Some(AtomicAliasOp::Xor),
            (Self::Rp2040, 2) => Some(AtomicAliasOp::Set),
            (Self::Rp2040, _) => Some(AtomicAliasOp::Clr),
            (Self::Efr32s2, 1) => Some(AtomicAliasOp::Set),
            (Self::Efr32s2, 2) => Some(AtomicAliasOp::Clr),
            (Self::Efr32s2, _) => Some(AtomicAliasOp::Xor),
        }
    }

    /// Whether this chip decodes atomic aliases at all.
    #[inline]
    pub fn is_enabled(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// The read-modify-write an atomic alias performs. `Xor` doubles as Series-2
/// TGL: toggling IS an XOR of the written mask, and keeping one op spares the
/// bus a second identical arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtomicAliasOp {
    /// Write XORs the bits (RP2040 `+0x1000`, EFR32 Series-2 TGL `+0x3000`).
    Xor,
    /// Write sets (ORs) the bits.
    Set,
    /// Write clears (AND-NOTs) the bits.
    Clr,
}

/// `false` / `true` / `"none"` / `"rp2040"` / `"efr32s2"`. The bool spelling is
/// what every RP2040 descriptor in the tree already carries; `true` keeps
/// meaning RP2040 rather than becoming ambiguous the day a second family
/// arrived.
fn deserialize_atomic_alias_flavour<'de, D>(d: D) -> Result<AtomicAliasFlavour, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum BoolOrName {
        Bool(bool),
        Name(String),
    }
    match BoolOrName::deserialize(d)? {
        BoolOrName::Bool(false) => Ok(AtomicAliasFlavour::None),
        BoolOrName::Bool(true) => Ok(AtomicAliasFlavour::Rp2040),
        BoolOrName::Name(name) => match name.trim().to_ascii_lowercase().as_str() {
            "none" | "false" => Ok(AtomicAliasFlavour::None),
            "rp2040" | "rp2350" | "true" => Ok(AtomicAliasFlavour::Rp2040),
            "efr32s2" | "efm32s2" | "efr32_series2" | "efr32xg2" => Ok(AtomicAliasFlavour::Efr32s2),
            other => Err(D::Error::custom(format!(
                "unknown atomic_register_aliases '{other}'; expected false, true, rp2040 or efr32s2"
            ))),
        },
    }
}

/// `include: nrf52-common.yaml` or `include: [a.yaml, b.yaml]`.
///
/// [`ChipDescriptor::from_file`] (and path [`ChipDescriptor::resolve`]) expand
/// this relative to the including file. `serde_yaml::from_str` stores it and
/// does not load files.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ChipInclude {
    One(String),
    Many(Vec<String>),
}

impl ChipInclude {
    fn paths(&self) -> &[String] {
        match self {
            Self::One(path) => std::slice::from_ref(path),
            Self::Many(paths) => paths.as_slice(),
        }
    }
}

/// Chip silicon descriptor (`chips/<name>.yaml`).
///
/// Path-loaded YAML may `include:` another file (or a list). Paths are relative
/// to the including file. Built-in `from_str` and bundled chips do **not**
/// expand includes — there is no filesystem.
///
/// ```yaml
/// include: nrf52-common.yaml
/// name: nrf52840
/// ```
///
/// or `include: [a.yaml, b.yaml]`. Includes load first; local keys win.
/// `peripherals` union by `id`.
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
    /// The clock this chip's core runs at in the simulator, in Hz — the single
    /// source of truth for every "how many cycles is a microsecond" question.
    ///
    /// Self-timing devices (DHT22, the rotary encoder, WS2812) convert their
    /// datasheet µs/ns windows to simulated cycles with this number, so it has
    /// to agree with the clock the firmware was built against or the firmware
    /// decodes noise. It used to be four inline literals and two board-name
    /// string comparisons spread over two languages; it is now declared once,
    /// here, per chip.
    ///
    /// This is the chip's *default* rate. A board that runs the part slower —
    /// the NUCLEO-L476RG never leaves the 4 MHz MSI reset clock — overrides it
    /// with [`SystemManifest::cpu_hz`].
    ///
    /// `0` means "undeclared". Every in-tree chip declares it and
    /// `every_chip_descriptor_declares_a_cpu_hz` fails the build if a new one does
    /// not, but the field stays defaulted so an out-of-tree chip YAML written before
    /// this existed still loads.
    #[serde(default, deserialize_with = "deserialize_u64_lax")]
    pub cpu_hz: u64,
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
    /// Atomic register aliases: the 0x1000-strided aliases of every peripheral
    /// register that a family's HAL uses for read-modify-write without a
    /// critical section. Two families do this with the SAME stride and
    /// DIFFERENT ops, so the key names which — see [`AtomicAliasFlavour`].
    /// Accepts `false`/`true` (historical spelling: `true` == `rp2040`) or the
    /// flavour name. Default: none, i.e. an alias address is unmapped MMIO and
    /// faults, which is correct for STM32/nRF/etc.
    #[serde(default, deserialize_with = "deserialize_atomic_alias_flavour")]
    pub atomic_register_aliases: AtomicAliasFlavour,
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
    /// Pad label → the ADC input that samples it, transcribed from the
    /// datasheet pinout (e.g. `PA0: { peripheral: adc1, channel: 0 }`).
    ///
    /// Kept apart from [`Self::pins`] on purpose. `pins:` is the AUTHORITATIVE
    /// GPIO map: once a chip declares it, every pad not listed stops resolving
    /// (see `chip_pins_ratchet`). Recording analog functions there would force
    /// a full GPIO transcription on every chip that only wants its ADC inputs
    /// named, or silently break every pad left out.
    ///
    /// Absent on a chip means "no analog pad is modelled", not "channel 0":
    /// the co-simulation `board.analog.<pad>_volts` path refuses such a pad
    /// rather than guessing, because the pad → channel assignment differs
    /// between families (PA0 is ADC1_IN0 on an F401 and ADC1_IN5 on an L476).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub analog_pins: std::collections::BTreeMap<String, AdcPinFn>,
    /// The supply the chip's GPIO pads run from, in volts (5.0 on an ATmega328P
    /// Nano, 3.3 on the STM32 boards). The reference [`Self::gpio_input_thresholds`]
    /// are ratios of.
    ///
    /// Absent means "not transcribed", and nothing defaults it: a board that
    /// runs the part at another supply would get thresholds for the wrong rail.
    #[serde(
        default,
        deserialize_with = "deserialize_io_voltage",
        skip_serializing_if = "Option::is_none"
    )]
    pub io_voltage_v: Option<f64>,
    /// Datasheet digital input thresholds, as ratios of [`Self::io_voltage_v`].
    ///
    /// Co-simulation needs them to turn a voltage routed to
    /// `board.gpio_in.<pad>` into the level the firmware reads. A chip without
    /// them refuses such a route when the session is built, naming this key,
    /// rather than comparing against a made-up midpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpio_input_thresholds: Option<GpioInputThresholds>,
    /// Path-loaded YAML only (`include: common.yaml` or a list). Built-in
    /// `from_str` does not expand includes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<ChipInclude>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExternalDevice {
    pub id: String,
    pub r#type: String,
    /// What this device hangs off. Normally a controller peripheral id
    /// (`"uart1"`, `"i2c1"`), but it may also name ANOTHER external device's
    /// `id` — that is how a slave is placed behind an I²C bus switch
    /// (TCA9548A). The loader resolves the peripheral name first; only if no
    /// peripheral answers to it does it look for a matching external device.
    pub connection: String,
    /// Downstream channel on the device named by `connection`, when that device
    /// is a bus switch (TCA9548A: 0..=7). Meaningless — and ignored — when
    /// `connection` names a controller.
    ///
    /// Optional so every pre-existing manifest deserializes and behaves
    /// exactly as before. `None` behind a switch means channel 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<u8>,
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

/// Typed, unit-explicit configuration for deterministic motor plants.
///
/// These DTOs live in `labwired-config` because the physics crate already
/// depends on this crate. The engine converts them to its `*MotorParams` types
/// at the construction boundary; raw YAML maps never reach the plant models.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum MotorModelConfig {
    Dc(Box<BrushedMotorConfig>),
    Bldc(Box<BldcMotorConfig>),
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BrushedMotorConfig {
    pub id: String,
    pub resistance_ohm: f64,
    pub inductance_h: f64,
    pub torque_constant_nm_per_a: f64,
    pub back_emf_constant_v_per_rad_s: f64,
    pub rotor_inertia_kg_m2: f64,
    pub viscous_friction_nm_per_rad_s: f64,
    pub supply_voltage_v: f64,
    pub load_torque_nm: f64,
    pub encoder_cpr: u32,
    #[serde(default = "default_motor_simulation_clock_hz")]
    pub simulation_clock_hz: u64,
    pub pwm_pin: String,
    pub direction_pin: String,
    pub brake_pin: String,
    pub enable_pin: String,
    pub encoder_a_pin: String,
    pub encoder_b_pin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoder_index_pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fault_pin: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BldcMotorConfig {
    pub id: String,
    pub resistance_ohm: f64,
    pub inductance_h: f64,
    pub torque_constant_nm_per_a: f64,
    pub back_emf_constant_v_per_rad_s: f64,
    pub rotor_inertia_kg_m2: f64,
    pub viscous_friction_nm_per_rad_s: f64,
    pub supply_voltage_v: f64,
    pub load_torque_nm: f64,
    pub encoder_cpr: u32,
    pub pole_pairs: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_limit_a: Option<f64>,
    #[serde(default = "default_overcurrent_trip_steps")]
    pub overcurrent_trip_steps: u32,
    #[serde(default = "default_motor_simulation_clock_hz")]
    pub simulation_clock_hz: u64,
    /// Chip-descriptor peripheral name for the advanced timer that owns the
    /// six complementary PWM legs (default `tim1` for STM32 advanced timers).
    #[serde(default = "default_bldc_timer_name")]
    pub timer_name: String,
    pub phase_a_high_pin: String,
    pub phase_a_low_pin: String,
    pub phase_b_high_pin: String,
    pub phase_b_low_pin: String,
    pub phase_c_high_pin: String,
    pub phase_c_low_pin: String,
    pub enable_pin: String,
    pub hall_a_pin: String,
    pub hall_b_pin: String,
    pub hall_c_pin: String,
    pub encoder_a_pin: String,
    pub encoder_b_pin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoder_index_pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motor_fault_pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inverter_fault_pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overcurrent_fault_pin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undervoltage_fault_pin: Option<String>,
}

fn default_motor_simulation_clock_hz() -> u64 {
    80_000_000
}

fn default_bldc_timer_name() -> String {
    "tim1".to_owned()
}

fn default_overcurrent_trip_steps() -> u32 {
    3
}

impl MotorModelConfig {
    /// Converts the canonical `external_devices` representation at the loader
    /// boundary into the typed plant DTO consumed by the engine.
    pub fn from_external_device(device: &ExternalDevice) -> Result<Option<Self>> {
        let kind = match device.r#type.as_str() {
            "dc-motor" | "dc_motor" => "dc",
            "bldc-motor" | "bldc_motor" => "bldc",
            _ => return Ok(None),
        };
        for reserved in ["kind", "id"] {
            if device.config.contains_key(reserved) {
                return Err(anyhow::anyhow!(
                    "external_devices[{}].config.{reserved} is reserved; motor identity and type come from the external device",
                    device.id
                ));
            }
        }
        let mut mapping = serde_yaml::Mapping::new();
        mapping.insert(
            serde_yaml::Value::String("kind".to_owned()),
            serde_yaml::Value::String(kind.to_owned()),
        );
        mapping.insert(
            serde_yaml::Value::String("id".to_owned()),
            serde_yaml::Value::String(device.id.clone()),
        );
        for (key, value) in &device.config {
            mapping.insert(serde_yaml::Value::String(key.clone()), value.clone());
        }
        let config: Self = serde_yaml::from_value(serde_yaml::Value::Mapping(mapping))
            .with_context(|| format!("invalid {} motor config '{}'", kind, device.id))?;
        let issues = config.validate();
        if issues.is_empty() {
            Ok(Some(config))
        } else {
            Err(anyhow::anyhow!(issues.join("; ")))
        }
    }

    pub fn id(&self) -> &str {
        match self {
            Self::Dc(config) => &config.id,
            Self::Bldc(config) => &config.id,
        }
    }

    /// Returns every configuration issue with a stable, field-qualified path.
    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();
        match self {
            Self::Dc(config) => {
                validate_motor_common(
                    &config.id,
                    config.resistance_ohm,
                    config.inductance_h,
                    config.torque_constant_nm_per_a,
                    config.back_emf_constant_v_per_rad_s,
                    config.rotor_inertia_kg_m2,
                    config.viscous_friction_nm_per_rad_s,
                    config.supply_voltage_v,
                    config.load_torque_nm,
                    config.encoder_cpr,
                    &mut issues,
                );
                validate_required_motor_pins(
                    &config.id,
                    [
                        ("pwm_pin", config.pwm_pin.as_str()),
                        ("direction_pin", config.direction_pin.as_str()),
                        ("brake_pin", config.brake_pin.as_str()),
                        ("enable_pin", config.enable_pin.as_str()),
                        ("encoder_a_pin", config.encoder_a_pin.as_str()),
                        ("encoder_b_pin", config.encoder_b_pin.as_str()),
                    ],
                    config.encoder_index_pin.as_deref(),
                    &mut issues,
                );
                validate_optional_motor_pin(
                    &config.id,
                    "fault_pin",
                    config.fault_pin.as_deref(),
                    &mut issues,
                );
                if config.simulation_clock_hz == 0 {
                    issues.push(format!(
                        "motor_models[{}].simulation_clock_hz must be greater than zero",
                        config.id
                    ));
                }
            }
            Self::Bldc(config) => {
                validate_motor_common(
                    &config.id,
                    config.resistance_ohm,
                    config.inductance_h,
                    config.torque_constant_nm_per_a,
                    config.back_emf_constant_v_per_rad_s,
                    config.rotor_inertia_kg_m2,
                    config.viscous_friction_nm_per_rad_s,
                    config.supply_voltage_v,
                    config.load_torque_nm,
                    config.encoder_cpr,
                    &mut issues,
                );
                if config.pole_pairs == 0 {
                    issues.push(format!(
                        "motor_models[{}].pole_pairs must be between 1 and 255 inclusive",
                        config.id
                    ));
                }
                if config.timer_name.trim().is_empty() {
                    issues.push(format!(
                        "motor_models[{}].timer_name must be nonblank",
                        config.id
                    ));
                }
                if config
                    .current_limit_a
                    .is_some_and(|limit| !limit.is_finite() || limit <= 0.0)
                {
                    issues.push(format!(
                        "motor_models[{}].current_limit_a must be finite and greater than zero",
                        config.id
                    ));
                }
                if config.current_limit_a.is_some() && config.overcurrent_trip_steps == 0 {
                    issues.push(format!(
                        "motor_models[{}].overcurrent_trip_steps must be greater than zero",
                        config.id
                    ));
                }
                validate_required_motor_pins(
                    &config.id,
                    [
                        ("phase_a_high_pin", config.phase_a_high_pin.as_str()),
                        ("phase_a_low_pin", config.phase_a_low_pin.as_str()),
                        ("phase_b_high_pin", config.phase_b_high_pin.as_str()),
                        ("phase_b_low_pin", config.phase_b_low_pin.as_str()),
                        ("phase_c_high_pin", config.phase_c_high_pin.as_str()),
                        ("phase_c_low_pin", config.phase_c_low_pin.as_str()),
                        ("enable_pin", config.enable_pin.as_str()),
                        ("hall_a_pin", config.hall_a_pin.as_str()),
                        ("hall_b_pin", config.hall_b_pin.as_str()),
                        ("hall_c_pin", config.hall_c_pin.as_str()),
                        ("encoder_a_pin", config.encoder_a_pin.as_str()),
                        ("encoder_b_pin", config.encoder_b_pin.as_str()),
                    ],
                    config.encoder_index_pin.as_deref(),
                    &mut issues,
                );
                validate_optional_motor_pin(
                    &config.id,
                    "motor_fault_pin",
                    config.motor_fault_pin.as_deref(),
                    &mut issues,
                );
                validate_optional_motor_pin(
                    &config.id,
                    "overcurrent_fault_pin",
                    config.overcurrent_fault_pin.as_deref(),
                    &mut issues,
                );
                validate_optional_motor_pin(
                    &config.id,
                    "undervoltage_fault_pin",
                    config.undervoltage_fault_pin.as_deref(),
                    &mut issues,
                );
                validate_optional_motor_pin(
                    &config.id,
                    "inverter_fault_pin",
                    config.inverter_fault_pin.as_deref(),
                    &mut issues,
                );
                if config.simulation_clock_hz == 0 {
                    issues.push(format!(
                        "motor_models[{}].simulation_clock_hz must be greater than zero",
                        config.id
                    ));
                }
            }
        }
        issues
    }
}

fn validate_optional_motor_pin(id: &str, field: &str, pin: Option<&str>, issues: &mut Vec<String>) {
    if pin.is_some_and(|pin| pin.trim().is_empty()) {
        issues.push(format!(
            "motor_models[{id}].{field} must be nonblank when present"
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_motor_common(
    id: &str,
    resistance_ohm: f64,
    inductance_h: f64,
    torque_constant_nm_per_a: f64,
    back_emf_constant_v_per_rad_s: f64,
    rotor_inertia_kg_m2: f64,
    viscous_friction_nm_per_rad_s: f64,
    supply_voltage_v: f64,
    load_torque_nm: f64,
    encoder_cpr: u32,
    issues: &mut Vec<String>,
) {
    let path = |field: &str| format!("motor_models[{id}].{field}");
    if id.trim().is_empty() {
        issues.push(format!("{} must be nonblank", path("id")));
    }
    for (field, value) in [
        ("resistance_ohm", resistance_ohm),
        ("inductance_h", inductance_h),
        ("torque_constant_nm_per_a", torque_constant_nm_per_a),
        (
            "back_emf_constant_v_per_rad_s",
            back_emf_constant_v_per_rad_s,
        ),
        ("rotor_inertia_kg_m2", rotor_inertia_kg_m2),
        ("supply_voltage_v", supply_voltage_v),
    ] {
        if !value.is_finite() || value <= 0.0 {
            issues.push(format!(
                "{} must be finite and greater than zero",
                path(field)
            ));
        }
    }
    if !viscous_friction_nm_per_rad_s.is_finite() || viscous_friction_nm_per_rad_s < 0.0 {
        issues.push(format!(
            "{} must be finite and non-negative",
            path("viscous_friction_nm_per_rad_s")
        ));
    }
    if !load_torque_nm.is_finite() {
        issues.push(format!("{} must be finite", path("load_torque_nm")));
    }
    if !(1..=1_000_000).contains(&encoder_cpr) {
        issues.push(format!(
            "{} must be between 1 and 1000000 inclusive",
            path("encoder_cpr")
        ));
    }
}

fn validate_required_motor_pins<'a>(
    id: &str,
    pins: impl IntoIterator<Item = (&'a str, &'a str)>,
    encoder_index_pin: Option<&str>,
    issues: &mut Vec<String>,
) {
    for (field, pin) in pins {
        if pin.trim().is_empty() {
            issues.push(format!("motor_models[{id}].{field} must be nonblank"));
        }
    }
    if encoder_index_pin.is_some_and(|pin| pin.trim().is_empty()) {
        issues.push(format!(
            "motor_models[{id}].encoder_index_pin must be nonblank when present"
        ));
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CosimAdapter {
    ExternalProcess,
    Fmi,
    Mock,
    /// `labwired_core::analog` — the in-core MNA engine. No `model` path: the
    /// circuit is a SPICE netlist under `config.netlist` / `config.netlist_text`.
    /// The only adapter the browser can run, since it spawns no process.
    Analog,
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
    /// Stimulus channel this contact exposes, when it is something other than a
    /// plain `pressed` button.
    ///
    /// A PIR, IR-obstacle, hall or vibration sensor is electrically the same
    /// thing — a digital output asserting a level on one pin — but "pressed" is
    /// the wrong word for motion or a magnetic field. The canvas catalog already
    /// names each one (`obstacle`, `field`, `vibration`), so the compiler stamps
    /// that name here rather than the engine keeping a second copy of the
    /// vocabulary. Absent means `pressed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

fn default_wifi_ap_ssid() -> String {
    "labwired-ap".to_string()
}

fn default_wifi_ap_ip() -> String {
    "192.168.4.1".to_string()
}

fn default_wifi_ap_serves() -> String {
    "labwired-stats".to_string()
}

/// Manifest opt-in for the per-lab virtual WiFi Access Point. Emitted when a
/// diagram contains a `wifi-ap` component. Absent ⇒ no AP (WiFi MACs stay
/// unassociated — honest "no AP present"). Mirrors `debug_uart`'s optional
/// pattern.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct WifiApManifest {
    /// Broadcast SSID of the AP.
    #[serde(default = "default_wifi_ap_ssid")]
    pub ssid: String,
    /// AP's IPv4 address (dotted-quad); its /24 is the DHCP pool.
    #[serde(default = "default_wifi_ap_ip")]
    pub ip: String,
    /// What the AP's HTTP origin serves: "labwired-stats" (default) or "none".
    #[serde(default = "default_wifi_ap_serves")]
    pub serves: String,
    /// Optional network password (PSK). Empty / absent = open AP.
    /// Stored so labs can record credentials that match firmware
    /// `WiFi.begin(ssid, password)`; the frame-level virtual AP does not
    /// model a WPA handshake yet — association still behaves as open.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SystemManifest {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub name: String,
    pub chip: String, // Reference to chip name or file path
    /// Override for [`ChipDescriptor::cpu_hz`] — the clock THIS board runs the
    /// part at, in Hz. Absent ⇒ the chip's declared default.
    ///
    /// The key has been in the corpus for a long time; until now nothing in the
    /// engine read it. Ten system YAMLs declared a `cpu_hz:` that serde threw
    /// away, and `nucleo-l476rg.yaml` documented the discard in a comment —
    /// the firmware there never configures the PLL, so the core really runs at
    /// the 4 MHz MSI reset rate and every self-timed device on that board was
    /// nevertheless being told 80 MHz. Reading the key is the fix.
    #[serde(default, deserialize_with = "deserialize_opt_u64_lax")]
    pub cpu_hz: Option<u64>,
    #[serde(default)]
    pub memory_overrides: HashMap<String, String>,
    #[serde(default)]
    pub external_devices: Vec<ExternalDevice>,
    /// Part packs this system carries — the parts that are NOT built into the
    /// engine. An `external_devices` entry whose `type:` names a pack here is
    /// modelled from that pack, so a private/vendor/customer catalog connects
    /// with no code in this repository and nothing published. See
    /// `docs/part-packs.md` for the `labwired.part/v1` contract.
    ///
    /// Packs ride in the manifest so every transport that already carries a
    /// manifest carries them too: the CLI, the browser wasm build, and the
    /// hosted builder's `/run`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<PartPack>,
    #[serde(default)]
    pub cosim_models: Vec<CosimModelConfig>,
    #[serde(default)]
    pub motor_models: Vec<MotorModelConfig>,
    #[serde(default)]
    pub board_io: Vec<BoardIoBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_uart: Option<String>,
    /// Per-lab virtual WiFi AP config (present ⇒ a `wifi-ap` component is on the
    /// diagram). Absent ⇒ no AP. See [`WifiApManifest`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wifi_ap: Option<WifiApManifest>,
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

/// Optional shared RF medium for a multi-node world (path loss / RSSI floor).
/// Positions are planar metres; co-located (default 0,0) keeps lossless links
/// until nodes are placed. Radios that honor [`labwired_core::peripherals::rf_medium`]
/// attach separately; this block is the env-manifest source of truth for seed + layout.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRfConfig {
    /// Run seed for seeded PER / medium draws (0 is valid).
    #[serde(default)]
    pub seed: u64,
    /// Node id → planar position in metres.
    #[serde(default)]
    pub nodes: HashMap<String, EnvironmentRfNode>,
    /// Minimum RSSI (dBm) for decode; below → drop. Omit for medium default.
    #[serde(default)]
    pub rssi_floor_dbm: Option<f64>,
    /// Path-loss exponent (free space ≈ 2). Omit for medium default.
    #[serde(default)]
    pub path_loss_exponent: Option<f64>,
    /// Reference loss at 1 m (dB). Omit for medium default.
    #[serde(default)]
    pub ref_loss_db: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentRfNode {
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
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
    /// Shared RF medium parameters (optional).
    #[serde(default)]
    pub rf: Option<EnvironmentRfConfig>,
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
    /// Test/helper constructor with no RF block.
    pub fn bare(
        name: impl Into<String>,
        nodes: Vec<NodeConfig>,
        interconnects: Vec<InterconnectConfig>,
    ) -> Self {
        Self {
            schema_version: "1.0".into(),
            name: name.into(),
            nodes,
            interconnects,
            rf: None,
        }
    }

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

        if let Some(rf) = &self.rf {
            for node_id in rf.nodes.keys() {
                if !node_ids.contains(node_id) {
                    anyhow::bail!(
                        "Environment manifest rf.nodes contains unknown node id '{node_id}'"
                    );
                }
            }
            if let Some(exp) = rf.path_loss_exponent {
                if !exp.is_finite() || exp <= 0.0 {
                    anyhow::bail!(
                        "Environment manifest rf.path_loss_exponent must be a positive finite number"
                    );
                }
            }
            if let Some(r) = rf.ref_loss_db {
                if !r.is_finite() {
                    anyhow::bail!("Environment manifest rf.ref_loss_db must be finite");
                }
            }
            if let Some(floor) = rf.rssi_floor_dbm {
                if !floor.is_finite() {
                    anyhow::bail!("Environment manifest rf.rssi_floor_dbm must be finite");
                }
            }
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
                &["peripheral", "endpoints"],
            )?;
            let has_peripheral = interconnect
                .config
                .get("peripheral")
                .and_then(serde_yaml::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some();
            let has_endpoints = interconnect
                .config
                .get("endpoints")
                .and_then(serde_yaml::Value::as_mapping)
                .is_some();
            if !has_peripheral && !has_endpoints {
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

fn yaml_str_key(key: &str) -> serde_yaml::Value {
    serde_yaml::Value::String(key.to_string())
}

/// `from_str` accepts `size: 1024` as a string field; `from_value` does not.
/// Walk mappings and stringify numeric `size` so include-merge can deserialize
/// without a YAML text round-trip.
fn coerce_yaml_size_numbers(value: &mut serde_yaml::Value) {
    match value {
        serde_yaml::Value::Mapping(map) => {
            let size_key = yaml_str_key("size");
            if let Some(serde_yaml::Value::Number(n)) = map.get(&size_key) {
                let s = n.to_string();
                map.insert(size_key, serde_yaml::Value::String(s));
            }
            for (_, v) in map.iter_mut() {
                coerce_yaml_size_numbers(v);
            }
        }
        serde_yaml::Value::Sequence(seq) => {
            for v in seq {
                coerce_yaml_size_numbers(v);
            }
        }
        _ => {}
    }
}

fn mapping_field<'a>(value: &'a serde_yaml::Value, key: &str) -> Option<&'a str> {
    value.as_mapping()?.get(yaml_str_key(key))?.as_str()
}

fn take_includes(doc: &mut serde_yaml::Value) -> Result<Vec<String>> {
    let Some(map) = doc.as_mapping_mut() else {
        return Ok(Vec::new());
    };
    let Some(value) = map.remove(yaml_str_key("include")) else {
        return Ok(Vec::new());
    };
    if matches!(value, serde_yaml::Value::Null) {
        return Ok(Vec::new());
    }
    let include: ChipInclude =
        serde_yaml::from_value(value).context("include must be a string or a list of strings")?;
    Ok(include.paths().to_vec())
}

fn merge_seq_by(
    base: serde_yaml::Value,
    overlay: serde_yaml::Value,
    field: &str,
) -> Result<serde_yaml::Value> {
    let mut items = match base {
        serde_yaml::Value::Null => Vec::new(),
        serde_yaml::Value::Sequence(seq) => seq,
        other => anyhow::bail!("expected a sequence while merging {field}, got {other:?}"),
    };
    let overlay = match overlay {
        serde_yaml::Value::Null => return Ok(serde_yaml::Value::Sequence(items)),
        serde_yaml::Value::Sequence(seq) => seq,
        other => anyhow::bail!("expected a sequence while merging {field}, got {other:?}"),
    };
    for item in overlay {
        if let Some(id) = mapping_field(&item, field).map(str::to_string) {
            if let Some(existing) = items
                .iter_mut()
                .find(|entry| mapping_field(entry, field) == Some(id.as_str()))
            {
                *existing = item;
                continue;
            }
        }
        items.push(item);
    }
    Ok(serde_yaml::Value::Sequence(items))
}

fn merge_yaml_maps(base: serde_yaml::Value, overlay: serde_yaml::Value) -> serde_yaml::Value {
    let mut map = match base {
        serde_yaml::Value::Mapping(map) => map,
        _ => serde_yaml::Mapping::new(),
    };
    if let serde_yaml::Value::Mapping(overlay) = overlay {
        for (key, value) in overlay {
            map.insert(key, value);
        }
    }
    serde_yaml::Value::Mapping(map)
}

fn merge_chip_yaml(
    base: serde_yaml::Value,
    overlay: serde_yaml::Value,
) -> Result<serde_yaml::Value> {
    let mut base_map = match base {
        serde_yaml::Value::Mapping(map) => map,
        serde_yaml::Value::Null => serde_yaml::Mapping::new(),
        other => anyhow::bail!("chip YAML must be a mapping, got {other:?}"),
    };
    let overlay_map = match overlay {
        serde_yaml::Value::Mapping(map) => map,
        serde_yaml::Value::Null => return Ok(serde_yaml::Value::Mapping(base_map)),
        other => anyhow::bail!("chip YAML must be a mapping, got {other:?}"),
    };
    for (key, value) in overlay_map {
        let key_name = key.as_str().unwrap_or("");
        let merged = match key_name {
            "include" => continue,
            "peripherals" => merge_seq_by(
                base_map.remove(&key).unwrap_or(serde_yaml::Value::Null),
                value,
                "id",
            )?,
            "memory_regions" => merge_seq_by(
                base_map.remove(&key).unwrap_or(serde_yaml::Value::Null),
                value,
                "name",
            )?,
            "pins" | "analog_pins" => merge_yaml_maps(
                base_map.remove(&key).unwrap_or(serde_yaml::Value::Null),
                value,
            ),
            _ => value,
        };
        base_map.insert(key, merged);
    }
    Ok(serde_yaml::Value::Mapping(base_map))
}

fn expand_chip_includes(
    path: &Path,
    content: &str,
    stack: &mut Vec<PathBuf>,
) -> Result<serde_yaml::Value> {
    let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if stack.contains(&canon) {
        anyhow::bail!("cycle detected in chip YAML include of {}", path.display());
    }
    stack.push(canon);
    let result = (|| {
        let mut doc: serde_yaml::Value =
            serde_yaml::from_str(content).context("Failed to parse Chip Descriptor YAML")?;
        let includes = take_includes(&mut doc)?;
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        let mut merged = serde_yaml::Value::Mapping(serde_yaml::Mapping::new());
        for rel in includes {
            let inc_path = dir.join(&rel);
            if !inc_path.is_file() {
                anyhow::bail!("chip YAML include not found: {}", inc_path.display());
            }
            let inc_content = std::fs::read_to_string(&inc_path).with_context(|| {
                format!("Failed to read chip YAML include {}", inc_path.display())
            })?;
            let included = expand_chip_includes(&inc_path, &inc_content, stack)?;
            merged = merge_chip_yaml(merged, included)?;
        }
        merge_chip_yaml(merged, doc)
    })();
    stack.pop();
    result
}

impl ChipDescriptor {
    /// Is this an ESP32-S3 (Xtensa LX7) part?
    ///
    /// The S3 needs its own memory map — DROM 0x3C00_xxxx, DRAM 0x3FC8_xxxx,
    /// IROM 0x4200_xxxx, IRAM 0x4037_xxxx — and the classic ESP32 (LX6) setup
    /// loads none of an S3 image's segments. Every caller that has to choose
    /// between those two setups asks this question, and the answer lives here
    /// because it was previously answered twice, differently:
    /// `crates/wasm` matched `name.starts_with("esp32s3")` and the CLI's `test`
    /// command matched `name == "esp32s3"` exactly. So `esp32s3-zero` — a
    /// shipped board variant — booted in the browser and died with a memory
    /// violation under `labwired test`, and no S3 board variant could be
    /// covered by a CLI gate at all.
    pub fn is_esp32s3(&self) -> bool {
        self.arch == Arch::Xtensa && self.name.starts_with("esp32s3")
    }

    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)?;

        if path.extension().is_some_and(|ext| ext == "json") {
            let ir: labwired_ir::IrDevice = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse Strict IR from {:?}", path))?;
            Ok(Self::from(ir))
        } else {
            let parsed: serde_yaml::Value =
                serde_yaml::from_str(&content).context("Failed to parse Chip Descriptor YAML")?;
            if !parsed
                .as_mapping()
                .is_some_and(|m| m.contains_key(yaml_str_key("include")))
            {
                return serde_yaml::from_str(&content)
                    .context("Failed to parse Chip Descriptor YAML");
            }
            let mut stack = Vec::new();
            let mut value = expand_chip_includes(path, &content, &mut stack)?;
            // YAML `size: 1024` is a number; `PeripheralConfig.size` is a
            // string (`"1024"` / `"1KB"`). `from_str` coerces; `from_value`
            // does not. Coerce in-place so we never round-trip through text.
            coerce_yaml_size_numbers(&mut value);
            serde_yaml::from_value(value).context("Failed to parse Chip Descriptor YAML")
        }
    }

    /// Resolve a `chip:` field. A bare name (`stm32f103`) is one of the chips
    /// bundled with the CLI; anything containing a separator or a YAML
    /// extension is a path relative to `base_dir`, for custom silicon.
    ///
    /// Built-in names exist so a project onboarding LabWired does not have to
    /// vendor a copy of our chip descriptor — a copy that silently keeps the
    /// bugs we have since fixed.
    pub fn resolve(spec: &str, base_dir: &Path) -> Result<Self> {
        Self::resolve_with(spec, base_dir, &|_| None)
    }

    /// Like [`Self::resolve`], but bare names not found among the built-ins are
    /// offered to `plugin_chips` (chip name → embedded YAML) before giving up.
    /// Built-ins win over plugin chips; path-like specs bypass the closure
    /// entirely; plugin YAML is parsed with the same [`ChipDescriptor`] schema
    /// as built-ins.
    pub fn resolve_with(
        spec: &str,
        base_dir: &Path,
        plugin_chips: &dyn Fn(&str) -> Option<&'static str>,
    ) -> Result<Self> {
        if is_builtin_chip_spec(spec) {
            let builtin = embedded_chip_yaml(spec);
            let source = if builtin.is_some() {
                "built-in"
            } else {
                "plugin"
            };
            if let Some(yaml) = builtin.or_else(|| plugin_chips(spec)) {
                return serde_yaml::from_str(yaml)
                    .with_context(|| format!("Failed to parse {source} chip descriptor '{spec}'"));
            }
            if MOVED_CHIP_NAMES.contains(&spec) {
                anyhow::bail!(
                    "chip '{spec}' is not part of the open catalog; it requires labwired-pro"
                );
            }
            anyhow::bail!(
                "unknown built-in chip '{spec}'. Available: {}. \
                 To use your own descriptor, give a path such as './chip.yaml'.",
                BUILTIN_CHIP_NAMES.join(", ")
            );
        }
        Self::from_file(base_dir.join(spec))
    }
}

/// True when `spec` names a built-in chip rather than a descriptor file.
pub fn is_builtin_chip_spec(spec: &str) -> bool {
    !spec.contains('/')
        && !spec.contains('\\')
        && !spec.ends_with(".yaml")
        && !spec.ends_with(".yml")
        && !spec.ends_with(".json")
}

// BUILTIN_CHIP_NAMES + embedded_chip_yaml(), generated from configs/chips/ by
// build.rs. They used to be two hand-written lists that had to agree with each
// other and with the directory; the directory is now the registry, so there is
// nothing left to keep in step. See build.rs for why ci-fixture-* is excluded.
include!(concat!(env!("OUT_DIR"), "/builtin_chips.rs"));

/// Chips that moved to the private `labwired-ip` repo. Kept so users get a
/// pointed error instead of "unknown chip". Empty until the first chip
/// migrates; `resolve`/`resolve_with` check it before the unknown-chip error.
pub const MOVED_CHIP_NAMES: &[&str] = &[];

impl SystemManifest {
    /// Parse a System Manifest from a YAML string. Unlike [`Self::from_file`]
    /// this does no filesystem-relative `can-player` `path:` inlining (there is
    /// no base directory), so it is wasm-safe and handy for tests/embedding.
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        serde_yaml::from_str(yaml).context("Failed to parse System Manifest")
    }

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
        // `parts: [- path: ./acme.yaml]` is the same CLI convenience: read it
        // here so the core only ever sees an inline pack (wasm has no fs).
        for entry in &mut manifest.parts {
            let rel = match entry {
                PartPack::Path(p) => p.path.clone(),
                PartPack::Inline(_) => continue,
            };
            let full = base.join(&rel);
            let text = std::fs::read_to_string(&full)
                .map_err(|e| anyhow::anyhow!("part pack {:?}: cannot read {e}", full))?;
            let origin = full.display().to_string();
            *entry = PartPack::Inline(Box::new(parse_part_pack(&text, &origin)?));
        }
        manifest.validate_parts()?;
        Ok(manifest)
    }

    /// The ONE lookup for a manifest-carried part. Returns the pack whose
    /// `type:` matches, or `None` when this system carries no such part (the
    /// caller then falls through to the built-in registries).
    ///
    /// Call [`Self::validate_parts`] before relying on this — it is what
    /// guarantees the match is unambiguous.
    pub fn resolve_part(&self, device_type: &str) -> Option<&DeviceDescriptor> {
        self.parts
            .iter()
            .filter_map(PartPack::descriptor)
            .find(|d| d.r#type == device_type)
    }

    /// Enforce the contract across the whole `parts:` list: every entry is a
    /// resolved, well-formed pack, and no two packs claim the same `type`.
    ///
    /// Two packs for one type is the failure this exists to prevent. Picking a
    /// winner would mean a customer's firmware silently runs against whichever
    /// catalog happened to load last — so it is an error, named, with both
    /// sources in the message.
    pub fn validate_parts(&self) -> Result<()> {
        let mut seen: HashMap<&str, Option<&str>> = HashMap::new();
        for entry in &self.parts {
            let pack = match entry {
                PartPack::Inline(d) => d.as_ref(),
                PartPack::Path(p) => anyhow::bail!(
                    "part pack '{}' was never loaded. `path:` is a CLI convenience that \
                     SystemManifest::from_file inlines; a manifest handed to the engine \
                     directly (browser, hosted runner) must carry the pack inline.",
                    p.path
                ),
            };
            let origin = pack
                .source
                .as_deref()
                .map(|s| format!("source: {s}"))
                .unwrap_or_else(|| "no declared source".to_string());
            validate_part_pack(pack, &origin)?;
            if let Some(prev) = seen.insert(pack.r#type.as_str(), pack.source.as_deref()) {
                anyhow::bail!(
                    "two part packs both define '{}' (sources: {} and {}). \
                     One part is one document — rename one, or namespace them \
                     `vendor:part`.",
                    pack.r#type,
                    prev.unwrap_or("undeclared"),
                    pack.source.as_deref().unwrap_or("undeclared"),
                );
            }
        }
        Ok(())
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
            if model.adapter == CosimAdapter::Analog {
                // The netlist itself is parsed by the core crate, which knows
                // the element subset; see
                // `labwired_core::cosim::validate_analog_models`. What is
                // checkable here is that the manifest declares a circuit and
                // says what to read out of it.
                let has_netlist = model
                    .config
                    .get("netlist")
                    .or_else(|| model.config.get("netlist_text"))
                    .is_some();
                if !has_netlist {
                    issues.push(format!(
                        "{location}.config requires `netlist` (a path) or `netlist_text` \
                         (inline) for the analog adapter"
                    ));
                }
                if !model.config.contains_key("probes") {
                    issues.push(format!(
                        "{location}.config.probes is required for the analog adapter: a \
                         model with no probe produces no outputs"
                    ));
                }
            }
        }

        issues
    }

    /// Resolves both explicitly typed plants and canonical external-device
    /// entries through one typed validation boundary.
    pub fn resolved_motor_models(&self) -> Result<Vec<MotorModelConfig>> {
        let mut models = Vec::new();
        let mut locations = HashMap::<String, String>::new();
        for (index, model) in self.motor_models.iter().enumerate() {
            let location = format!("motor_models[{index}].id");
            if let Some(previous) = locations.insert(model.id().to_owned(), location.clone()) {
                return Err(anyhow::anyhow!(
                    "{location} duplicates motor id declared at {previous}"
                ));
            }
            models.push(model.clone());
        }
        for (index, device) in self.external_devices.iter().enumerate() {
            if let Some(model) = MotorModelConfig::from_external_device(device)? {
                let location = format!("external_devices[{index}].id");
                if let Some(previous) = locations.insert(model.id().to_owned(), location.clone()) {
                    return Err(anyhow::anyhow!(
                        "{location} duplicates motor id declared at {previous}"
                    ));
                }
                models.push(model);
            }
        }
        let issues: Vec<_> = models.iter().flat_map(MotorModelConfig::validate).collect();
        if issues.is_empty() {
            Ok(models)
        } else {
            Err(anyhow::anyhow!(issues.join("; ")))
        }
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

/// What a READ of a register does to it, beyond handing back its value.
///
/// One vocabulary for the whole engine: the MCU register machine reads it out
/// of [`SideEffectsDescriptor`], and a device descriptor reads the same enum
/// out of [`RegisterSpec::on_read`]. SystemRDL names.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReadAction {
    #[default]
    None,
    /// The register reads back its value once and is then zeroed. See
    /// [`RegisterSpec::on_read`] for exactly when "once" is.
    #[serde(alias = "readClear", alias = "read_clear", alias = "clear_on_read")]
    Clear,
}

/// What a WRITE of a register does to the stored word.
///
/// The SystemRDL vocabulary, shared by the MCU register machine
/// ([`SideEffectsDescriptor`]) and device descriptors
/// ([`RegisterSpec::on_write`]). Each spelling a datasheet or an SVD might use
/// is an ALIAS of one value rather than a second value, so `one_to_clear`,
/// `write_one_to_clear` and `oneToClear` cannot drift apart.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum WriteAction {
    #[default]
    None,
    /// `reg &= !data` — a 1 written clears that bit (the interrupt-acknowledge
    /// idiom). Restricted to [`RegisterSpec::write_mask`] when one is declared.
    #[serde(alias = "oneToClear", alias = "one_to_clear", alias = "w1c")]
    WriteOneToClear,
    /// `reg &= data` — a 0 written clears that bit.
    #[serde(alias = "zeroToClear", alias = "zero_to_clear", alias = "w0c")]
    WriteZeroToClear,
    /// `reg |= data` — a 1 written sets that bit and a 0 leaves it alone (the
    /// set/clear register-pair idiom every GPIO port and most interrupt
    /// enablers use).
    #[serde(alias = "oneToSet", alias = "write_one_to_set", alias = "w1s")]
    OneToSet,
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

/// What a timed event does to a register, by NAME.
///
/// Shared by the MCU register machine's [`TimingDescriptor`] and a declarative
/// device's [`DeviceTimer`], so "the silicon changed a register by itself" has
/// one spelling everywhere.
///
/// Accepts BOTH YAML shapes on the way in: the datasheet-shaped single-key map
/// `{ set_bits: { register: STATUS, bits: 0x01 } }`, and serde_yaml's own
/// external tag `!set_bits { … }`. The map form is what anyone writing a part
/// by hand reaches for, and serde_yaml 0.9 rejects it for an externally-tagged
/// enum — the same reason [`AutoIncrement`] carries a hand-written
/// `Deserialize`. Serialization emits the derived form.
#[derive(Debug, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimingAction {
    SetBits { register: String, bits: u32 },
    ClearBits { register: String, bits: u32 },
    WriteValue { register: String, value: u32 },
}

/// Fields of any [`TimingAction`] variant in the single-key map form.
#[derive(Deserialize)]
struct TimingActionFields {
    register: String,
    #[serde(default)]
    bits: Option<u32>,
    #[serde(default)]
    value: Option<u32>,
}

/// The derived shape, used only to accept the `!set_bits` tag form.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum TimingActionTagged {
    SetBits { register: String, bits: u32 },
    ClearBits { register: String, bits: u32 },
    WriteValue { register: String, value: u32 },
}

impl<'de> Deserialize<'de> for TimingAction {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        const VARIANTS: &[&str] = &["set_bits", "clear_bits", "write_value"];
        let value = serde_yaml::Value::deserialize(deserializer)?;
        if let serde_yaml::Value::Mapping(m) = &value {
            if m.len() == 1 {
                if let Some((key, inner)) = m.iter().next() {
                    if let Some(key) = key.as_str() {
                        if VARIANTS.contains(&key) {
                            let f: TimingActionFields =
                                serde_yaml::from_value(inner.clone()).map_err(D::Error::custom)?;
                            let need = |what: &str, v: Option<u32>| {
                                v.ok_or_else(|| {
                                    D::Error::custom(format!(
                                        "timing action '{key}' needs '{what}'"
                                    ))
                                })
                            };
                            return match key {
                                "set_bits" => Ok(TimingAction::SetBits {
                                    register: f.register,
                                    bits: need("bits", f.bits)?,
                                }),
                                "clear_bits" => Ok(TimingAction::ClearBits {
                                    register: f.register,
                                    bits: need("bits", f.bits)?,
                                }),
                                _ => Ok(TimingAction::WriteValue {
                                    register: f.register,
                                    value: need("value", f.value)?,
                                }),
                            };
                        }
                        return Err(D::Error::unknown_variant(key, VARIANTS));
                    }
                }
            }
        }
        // Not a single-key map: fall back to serde_yaml's tagged form.
        let tagged: TimingActionTagged = serde_yaml::from_value(value).map_err(D::Error::custom)?;
        Ok(match tagged {
            TimingActionTagged::SetBits { register, bits } => {
                TimingAction::SetBits { register, bits }
            }
            TimingActionTagged::ClearBits { register, bits } => {
                TimingAction::ClearBits { register, bits }
            }
            TimingActionTagged::WriteValue { register, value } => {
                TimingAction::WriteValue { register, value }
            }
        })
    }
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
    /// auxiliary `board_io`) block. When present, the canvas compiler
    /// (the TypeScript `compile()` emitter) derives the block from this single
    /// spec instead of a hand-mirrored pair.
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
    /// Contract version, `labwired.part/v1`. Required of a pack that arrives
    /// through a manifest's `parts:`; absent on the descriptors bundled in
    /// `configs/devices/` (they predate the contract and are validated by
    /// being in-tree). Declaring it is what lets us change the schema later
    /// without guessing at what an out-of-tree file meant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    /// Provenance: which catalog/vendor/customer shipped this pack. Carried so
    /// an error message and a bug report can name where the model came from —
    /// "which model ran?" must never be answered by a shrug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The built-in part `type` this pack deliberately replaces. Shadowing a
    /// built-in is otherwise a hard error, so a replacement is always explicit
    /// and attributable rather than a silent win for whoever loaded last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<String>,
    /// The app-layer catalog record (pin declarations, device class, ref
    /// prefix). Opaque here — the simulation core has no use for it — but
    /// carried so one pack file describes the part end to end instead of
    /// splitting into a half that simulates and a half that draws.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<serde_yaml::Value>,
}

/// The contract version a manifest-carried part pack must declare.
pub const PART_PACK_SCHEMA: &str = "labwired.part/v1";

/// One entry in a manifest's `parts:` list.
///
/// `path:` is a `labwired` CLI convenience — [`SystemManifest::from_file`]
/// reads the file and replaces the entry with its contents, exactly as it
/// already does for a `can-player`'s `path:`. The simulation core only ever
/// sees [`PartPack::Inline`]: it has no filesystem under wasm, and a contract
/// that works on only one of our three runtimes is not a contract.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum PartPack {
    /// A path to a pack file, relative to the manifest. CLI-only; inlined on load.
    Path(PartPackPath),
    /// The pack itself, verbatim.
    Inline(Box<DeviceDescriptor>),
}

/// The `- path: ./acme-tmp999.yaml` spelling of a [`PartPack`].
///
/// `deny_unknown_fields` is load-bearing: it is what stops serde's untagged
/// matching from swallowing a malformed inline pack into this variant and
/// reporting "missing field: path" for a file that plainly has `type:` and
/// `behavior:` in it.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct PartPackPath {
    pub path: String,
}

impl PartPack {
    /// The pack body, or `None` for a still-unresolved `path:` entry.
    pub fn descriptor(&self) -> Option<&DeviceDescriptor> {
        match self {
            PartPack::Inline(d) => Some(d),
            PartPack::Path(_) => None,
        }
    }
}

/// Parse a part pack from YAML and enforce the `labwired.part/v1` contract.
///
/// Unlike [`DeviceDescriptor::from_yaml`] (which also parses the in-tree
/// `configs/devices/*.yaml` bodies) this REQUIRES the `schema:` declaration,
/// because an out-of-tree file is the one case where we cannot tell by
/// inspection which schema its author was writing against.
pub fn parse_part_pack(yaml: &str, origin: &str) -> Result<DeviceDescriptor> {
    let pack = DeviceDescriptor::from_yaml(yaml)
        .with_context(|| format!("part pack {origin} is not a valid descriptor"))?;
    validate_part_pack(&pack, origin)?;
    Ok(pack)
}

/// Enforce the parts of the contract that are true of every pack, wherever it
/// came from: the schema declaration, a non-empty type, and a `behavior` that
/// names a primitive.
pub fn validate_part_pack(pack: &DeviceDescriptor, origin: &str) -> Result<()> {
    match pack.schema.as_deref() {
        Some(PART_PACK_SCHEMA) => {}
        Some(other) => anyhow::bail!(
            "part pack {origin} declares unknown schema '{other}'; \
             this engine speaks '{PART_PACK_SCHEMA}'"
        ),
        None => anyhow::bail!(
            "part pack {origin} is missing `schema: {PART_PACK_SCHEMA}`. \
             Declaring the contract version is what lets the schema change \
             later without guessing what your file meant."
        ),
    }
    if pack.r#type.trim().is_empty() {
        anyhow::bail!("part pack {origin} has an empty `type:`");
    }
    if pack.behavior.primitive.trim().is_empty() {
        anyhow::bail!(
            "part pack '{}' ({origin}) has an empty `behavior.primitive:`",
            pack.r#type
        );
    }
    Ok(())
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
    /// The named stimulus channels this device accepts. For an `i2c_device`
    /// primitive these are the measurement slots that register/response
    /// `source:` keys read; each `default` seeds the value the part reports
    /// until something drives it, and `min`/`max` bound accepted stimuli.
    #[serde(default)]
    pub inputs: Vec<InputSpec>,
    /// Starter labs that ship a one-click demo using this device, mirrored into
    /// the manifest exactly like a hand-written kit's `labs` (mirrors
    /// `kit::LabRef`). Absent ⇒ the kit advertises no labs.
    #[serde(default)]
    pub labs: Vec<LabDescriptor>,
}

/// A demo lab a declarative device advertises (mirrors `kit::LabRef`). Optional;
/// absent ⇒ the kit advertises no labs.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LabDescriptor {
    pub board_id: String,
    pub chip: String,
    pub example_dir: String,
    pub demo_elf: String,
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
    /// Gaussian noise sigma applied per read, in `unit` (seeded, replay-safe).
    #[serde(default)]
    pub noise_sigma: Option<f64>,
    /// Constant offset applied to the channel value, in `unit`.
    #[serde(default)]
    pub bias: Option<f64>,
    /// First-order thermal-lag time constant in seconds; requires a bus that
    /// drives `advance_time_us` (degrades to no lag elsewhere).
    #[serde(default)]
    pub thermal_tau_s: Option<f64>,
    /// The `external_devices` `config:` key that SEEDS this channel's starting
    /// value, when it differs from `key`. Absent ⇒ the channel key itself,
    /// which is what the declarative kit has always used.
    ///
    /// Needed by a port whose hand-written kit named the two differently: the
    /// MLX90614 kit takes `surface_temp_c` / `ambient_temp_c` in `config:` and
    /// serves `surface_temp` / `ambient_temp` as runtime channels, and three
    /// shipped `system.yaml` files set the former. Without this the seed would
    /// silently do nothing and the part would boot at the descriptor default —
    /// a config key that parses and changes nothing, which is the failure mode
    /// this schema refuses elsewhere.
    #[serde(default)]
    pub config_key: Option<String>,
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
    pub registers: Vec<RegisterSpec>,
    /// Command set. Present ⇒ this is a command device.
    #[serde(default)]
    pub commands: Vec<I2cCommand>,
    /// **Register-pointer width in bytes**: how many bytes after START the
    /// master writes to select an address, big-endian (high byte first — the
    /// order every 16-bit-addressed I²C part on the market uses). `1` (the
    /// default) is the ordinary 8-bit pointer every device written before this
    /// existed has. `2` is the memory-like shape: an AT24C256 EEPROM addresses
    /// 32 KiB with two address bytes, and a 16-bit-mapped sensor does the same.
    ///
    /// Applies to BOTH pointer modes — the named `registers:` map (whose
    /// [`RegisterSpec::addr`] is a `u16` for exactly this reason) and the
    /// byte-addressable `register_file:`.
    #[serde(default = "default_pointer_width")]
    pub pointer_width: u8,
    /// **Write page size in bytes** for a memory-like part. A sequential write
    /// that runs past a page boundary wraps to the START of the same page
    /// instead of spilling into the next one — the single most surprising real
    /// EEPROM behaviour, and the reason a driver that writes a 40-byte record
    /// across a page boundary silently corrupts it on hardware but "worked" in
    /// a model that just incremented. Reads are NOT paged: sequential read
    /// rolls over the whole array. Absent ⇒ no page wrap.
    #[serde(default)]
    pub write_page: Option<u16>,
    /// Mask applied to the pointer byte the master writes in **register-pointer**
    /// mode (`registers:`). Absent ⇒ `0xFF` (no masking). A part whose pointer is
    /// only a few low bits (TMP102 uses `0x03`) sets it so a write of an
    /// out-of-range pointer aliases into the register file exactly as silicon does.
    #[serde(default)]
    pub pointer_mask: Option<u16>,
    /// **Byte-addressable register file** mode. Present ⇒ this is a register-file
    /// device (256 one-byte registers with a write-pointer that walks on
    /// auto-increment, PCA9685-style). Mutually exclusive with `registers:` and
    /// `commands:`.
    #[serde(default)]
    pub register_file: Option<RegisterFileSpec>,
    /// Self-driving state updates fired by bus activity (e.g. the TMP102's
    /// +0.5 °C-per-read drift). Each fires when a full multi-byte read of its
    /// target register completes. Applies to the `registers:` (wide) mode.
    #[serde(default)]
    pub updates: Vec<UpdateRule>,
    /// **Register-pointer auto-increment**: the pointer walks one BYTE per byte
    /// read, so a master can read a contiguous block in one transaction.
    /// Applies to the `registers:` (wide) mode. Default false, which keeps every
    /// device written before this existed byte-identical — without it a read
    /// past the pointed register's width returns `0xFF` forever.
    ///
    /// Byte-wise rather than register-wise deliberately. ST's VL53L0X API reads
    /// a 12-byte block starting at `RESULT_RANGE_STATUS` (0x14) to reach the
    /// range bytes at 0x1E, crossing both declared registers and addresses this
    /// model does not declare. Advancing by whole registers would land on the
    /// wrong byte the moment a gap or a 2-byte register appears in the span.
    #[serde(default)]
    pub auto_increment: bool,
    /// **Hybrid auto-increment**: addresses the byte-wise auto-increment
    /// pointer JUMPS from instead of stepping through. Each entry says "after
    /// serving `from`, the next address is `to`" — only on the auto-increment
    /// walk; an explicit pointer write is never remapped. Empty ⇒ the pointer
    /// always steps by one, which is every device written before this existed.
    ///
    /// This is the datasheet shape for a part whose register map is two blocks
    /// the driver wants as ONE burst. The NXP FXOS8700CQ is the motivating
    /// case: §14.2 "hybrid mode" says that with `M_CTRL_REG2.hyb_autoinc_mode`
    /// set, a read that walks off the end of the accelerometer block (0x06)
    /// continues at the magnetometer block (0x33), so the 6-axis driver reads
    /// all twelve bytes in a single transaction. Without the jump that driver
    /// reads 0x07..0x0C — reserved space — as its magnetometer data.
    ///
    /// The remap is UNCONDITIONAL here: it does not read the enable bit, because
    /// "this map applies only while that bit is set" is a state machine, not a
    /// map. A descriptor that declares the jump therefore models the part in
    /// hybrid mode; see the FXOS8700 descriptor for that stated as a modelled
    /// scope rather than left implicit.
    #[serde(default)]
    pub auto_increment_map: Vec<AddressRemap>,
    /// Byte returned for an address inside an auto-increment block that no
    /// declared register covers. Absent ⇒ `0xFF`, matching what a non-
    /// auto-incrementing device already returns past the end of a register.
    ///
    /// Real parts differ and the difference is observable: a driver may block-read
    /// across reserved addresses and compare. Set it to what the part actually
    /// drives rather than accepting the default by omission.
    #[serde(default)]
    pub unmapped_byte: Option<u8>,
    /// Engineering-unit values derived from the register file, readable by Rust
    /// consumers/tests through the engine's `observable()` API (e.g. the PCA9685
    /// `servo_angle` per channel). Applies to the `register_file:` mode.
    #[serde(default)]
    pub observables: Vec<ObservableSpec>,
    /// Conversion-timing status bits: firmware writes a start bit, the part
    /// takes `conversion_us`, then a status bit reads set until the result is
    /// read. Applies to the `registers:` (wide) mode. See [`DataReady`].
    #[serde(default)]
    pub data_ready: Vec<DataReady>,
    /// **Register bank (page) select**: the pointer address whose written value
    /// selects which bank the rest of the map is decoded in. Absent ⇒ the device
    /// has one flat map (every device written before this existed).
    ///
    /// This is the datasheet shape for parts whose register space is larger than
    /// the 8-bit pointer: ST's VL53L0X API bank-switches through 0xFF, and the
    /// SAME pointer means different registers per bank — 0xB6 is
    /// `GLOBAL_CONFIG_REF_EN_START_SELECT` in bank 0 and
    /// `RESULT_PEAK_SIGNAL_RATE_REF` in bank 1, 0x84 is `GPIO_HV_MUX_ACTIVE_HIGH`
    /// in bank 0 and the oscillator-frequency word in bank 1. Modelling them as
    /// one register would make a vendor driver read back its own configuration
    /// where silicon hands it a measurement.
    ///
    /// A register carrying [`RegisterSpec::page`] decodes only in that bank; a
    /// register without one decodes in every bank (the flat, bank-agnostic core
    /// map), so only the addresses that genuinely alias need to say so.
    #[serde(default)]
    pub page_register: Option<u16>,
    /// **Indexed readout ports**: an index register + a strobe handshake + a
    /// data register, standing in for storage that is not directly pointer-
    /// addressable (a factory NVM / OTP array). See [`IndexedTable`].
    #[serde(default)]
    pub indexed_tables: Vec<IndexedTable>,
    /// Width of the register POINTER in bytes. `1` (the default) is the
    /// ordinary register-pointer part: the first byte of a write selects a
    /// register, the rest are data.
    ///
    /// `0` is the **pointerless** shape: the part has exactly one addressable
    /// register (declared at `addr: 0`) and EVERY byte on the wire is that
    /// register's data — there is no pointer to write and none to read past.
    /// The NXP PCF8574 I/O expander is the canonical one: "the master sends one
    /// byte, which is the port", and a model that insisted on a pointer byte
    /// would consume the port value as an address and then latch the NEXT byte,
    /// which for a single-byte write means the port never changes at all.
    ///
    /// Nothing else in the register-pointer engine changes: `write_mask`,
    /// `bits:`, `source:`, reset values and the Tier-2 rules all behave exactly
    /// as they do for a pointered part.
    #[serde(default = "default_pointer_bytes")]
    pub pointer_bytes: u8,
}

fn default_pointer_bytes() -> u8 {
    1
}

/// One entry of [`I2cSpec::auto_increment_map`]: after the auto-increment
/// pointer has served `from`, the next address it serves is `to`.
#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub struct AddressRemap {
    pub from: u16,
    pub to: u16,
}

/// One **indexed readout port**: the datasheet shape for reading storage that
/// the register map does not expose directly — write an index, hand the device a
/// strobe, wait for the strobe to read back done, then read the latched word out
/// of a data register.
///
/// The VL53L0X factory NVM is read exactly this way by ST's own API
/// (`VL53L0X_get_info_from_device` / `VL53L0X_device_read_strobe`, API 1.0.2):
/// `WrByte(0x94, index)` → `WrByte(0x83, 0x00)` → poll `RdByte(0x83)` until it
/// reads non-zero → `RdDWord(0x90)`. Both `VL53L0X_GetDeviceInfo` and
/// `VL53L0X_StaticInit` go through it, and both give up with
/// `VL53L0X_ERROR_TIME_OUT` after `VL53L0X_DEFAULT_MAX_LOOP` (200) polls if the
/// strobe never comes back — which is what a device that does not model this
/// port looks like from the driver's side.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct IndexedTable {
    /// Diagnostic name for this port (e.g. `nvm`). Not addressable.
    pub name: String,
    /// Register the master writes the entry index to.
    pub index_register: String,
    /// Register the master strobes and then polls. A write of
    /// `strobe_arm_value` arms a fetch; `strobe_mask` reads set once the fetch
    /// has latched into `data_register`. Those bits are model-owned, so the
    /// register's `write_mask` must exclude them.
    pub strobe_register: String,
    /// Value the master writes to `strobe_register` to arm a fetch (VL53L0X:
    /// `0x00` — the driver clears the strobe and waits for the device to raise
    /// it).
    #[serde(default)]
    pub strobe_arm_value: u32,
    /// Bit(s) of `strobe_register` the model raises when the fetch has landed.
    pub strobe_mask: u32,
    /// Register the fetched word is latched into (read normally afterwards).
    pub data_register: String,
    /// Access time in microseconds. `0` ⇒ the fetch lands before the master can
    /// issue the next I²C frame, which is the honest answer for an on-die NVM
    /// array (sub-microsecond) polled over an I²C bus (tens to hundreds of µs
    /// per read frame): silicon has always already finished by the first poll.
    #[serde(default)]
    pub access_us: u64,
    /// The stored words, keyed by index. An index with no entry latches 0 —
    /// the same "unprogrammed" answer an erased NVM cell gives.
    #[serde(default)]
    pub entries: BTreeMap<u8, u32>,
}

/// A read value derived from **how many bits are set** in other registers,
/// scaled by a per-bit constant.
///
/// This is the honest shape for a measurement whose magnitude is proportional to
/// the number of enabled elements rather than to any external stimulus. The
/// VL53L0X reference-signal rate is the case that motivated it: it is the return
/// rate of the internal reference path, so it grows with the number of enabled
/// reference SPADs, and ST's `VL53L0X_perform_ref_spad_management` closes its
/// loop on exactly that relation — it enables one more good SPAD at a time and
/// re-measures until the rate reaches the target. A constant would either stall
/// that loop (rate never reaches the target) or short-circuit it into a branch
/// silicon does not take.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PopcountSource {
    /// Registers whose set bits are counted (in order; the count is their sum).
    pub registers: Vec<String>,
    /// Value contributed by each set bit.
    pub per_bit: u32,
}

/// A **power-gate**: while a bit is set in another register, this register
/// stops reporting a measurement and reads all-zero.
///
/// This is the datasheet shape for a sensor that can be *shut down* while it
/// stays addressable on the bus. The Vishay VEML7700 is the case that motivated
/// it: command code 0 (`ALS_CONF`) powers up at `0x0001`, and bit 0 (`ALS_SD`)
/// is documented as *"0 = ALS power on, 1 = ALS shut down"* — so a part that has
/// never had that bit cleared has never run a conversion, and its `ALS` /
/// `WHITE` result registers cannot hold light data. Without this gate the model
/// would hand a shut-down sensor a plausible reading, and firmware that forgets
/// to power the part on would pass in simulation and read zeros on silicon.
///
/// It is deliberately expressed as data over one shared behaviour rather than a
/// per-device Rust branch: a second part adopts it by naming its own register
/// and mask in YAML.
///
/// **Modelled scope, stated plainly.** The gate makes the register read zero
/// while the bit is set. It does not model conversion restart latency after the
/// bit is cleared (the datasheet's power-on settling time), and it says nothing
/// about what silicon retains in the result register if firmware shuts a
/// *running* part down mid-flight — the datasheet does not specify that, and
/// this model returns zero there too. The power-on case, which is the one
/// firmware actually trips over, is exact.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ZeroWhen {
    /// Register holding the gating bit(s).
    pub register: String,
    /// Bits that, when ANY is set, force this register's read to zero.
    pub mask: u32,
}

/// One **data-ready** rule: a write-triggered, time-gated status bit.
///
/// This is the datasheet shape shared by every "start a conversion, poll a
/// flag, read the result" part — the VCNL4010's `prox_od` → `prox_data_rdy` →
/// `PROX_DATA`, the VL53L0X's `SYSRANGE_START` → `RESULT_INTERRUPT_STATUS` →
/// range bytes, and the ready-flag halves of the Sensirion command devices. It
/// is deliberately expressed as data over one Rust behaviour, not as a
/// per-device trigger/action pair: a second part adopts it by naming its own
/// registers, masks and conversion time in YAML.
///
/// Lifecycle (`name` is for diagnostics only):
///  1. **idle** at power-on — the status bit reads clear.
///  2. A master write to `start_register` that leaves any `start_mask` bit set
///     starts a conversion; the status bit reads clear for `conversion_us` of
///     simulated wall-clock.
///  3. **ready** — the status bit reads set (OR'd over whatever the register
///     stores), and the result registers hold the current measurement.
///  4. A read of any `clear_on_read` register clears the status bit, exactly as
///     the datasheets specify ("this bit will be reset when one of the
///     corresponding result registers is read"). If the start bits are still
///     set in the register at that moment the next conversion starts
///     immediately, so both the on-demand and the periodic/self-timed firmware
///     idiom keep producing fresh results.
///
/// **Holdout degradation.** `conversion_us` is measured on the honest µs source
/// the bus master feeds in through `I2cDevice::advance_time_us` (ESP32
/// SYSTIMER, nRF54L GRTC). On families with no absolute-µs counter — STM32,
/// ESP32-classic, nRF52 — nothing ever advances that clock, and the status bit
/// degrades to *always set*, i.e. exactly the always-ready constant these
/// devices modelled before this primitive existed. The degradation is
/// deliberate: fabricating a µs clock from a pinned core frequency would be a
/// cheat, and a permanently-clear flag would hang firmware that is correct.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DataReady {
    /// Diagnostic name for this conversion (e.g. `proximity`). Not addressable.
    pub name: String,
    /// Register whose bits the master writes to start a conversion.
    pub start_register: String,
    /// Bits in `start_register` that start one. A write leaving ANY of them set
    /// starts a conversion (level, not edge — drivers re-issue the same
    /// on-demand bit for every reading).
    pub start_mask: u32,
    /// Register carrying the sim-driven status bit (often the same register).
    pub ready_register: String,
    /// The status bit(s) within `ready_register`, OR'd into every read of it.
    pub ready_mask: u32,
    /// Datasheet conversion time in microseconds.
    pub conversion_us: u64,
    /// Registers whose read clears the status bit. Empty ⇒ the bit stays set
    /// once the conversion completes.
    #[serde(default)]
    pub clear_on_read: Vec<String>,
    /// Registers whose WRITE clears the status bit — the write-1-to-clear
    /// interrupt idiom.
    ///
    /// Distinct from `clear_on_read` because the parts differ on which event
    /// clears: the VCNL4010 clears when the result register is read, while the
    /// VL53L0X keeps its interrupt asserted until firmware writes
    /// `SYSTEM_INTERRUPT_CLEAR` — reading the range does NOT clear it. Modelling
    /// the second as the first would let a driver that never writes the clear
    /// register appear to work, which is precisely the bug class a faithful
    /// twin exists to catch.
    ///
    /// Any write to a named register clears, regardless of value: the datasheet
    /// idiom is "write anything to acknowledge", and a value-matched variant
    /// would be a guess about which encoding a given part uses.
    #[serde(default)]
    pub clear_on_write: Vec<String>,
}

/// **Byte-addressable register file** for the `register_file` I²C mode: `size`
/// one-byte registers, a write-pointer selected by the first post-START byte,
/// and an auto-increment policy that walks the pointer after each data byte.
/// This is the datasheet shape for PWM expanders / GPIO expanders (PCA9685)
/// whose block writes stream consecutive registers. Mutually exclusive with the
/// named `registers:`/`commands:` shapes.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RegisterFileSpec {
    /// Number of one-byte registers (typically 256).
    pub size: usize,
    /// Sparse non-zero power-on reset values, keyed by register offset.
    #[serde(default)]
    pub reset: BTreeMap<u8, u8>,
    /// Mask applied to the pointer. Absent ⇒ `0xFF` (the 8-bit pointer every
    /// register-file device had before [`I2cSpec::pointer_width`] existed); a
    /// two-byte-pointer part sets the width its address bus really has.
    #[serde(default = "default_pointer_mask")]
    pub pointer_mask: u16,
    /// Value every byte of the file powers up holding, before the sparse
    /// `reset` entries are stamped over it. Absent ⇒ 0. An erased EEPROM cell
    /// reads `0xFF`, and a 32 KiB part cannot say that one `reset` entry at a
    /// time.
    #[serde(default)]
    pub fill: Option<u8>,
    /// The first byte written after START selects the pointer. Default true.
    #[serde(default = "default_true")]
    pub first_write_after_start_sets_pointer: bool,
    /// When the pointer advances after a data byte read/written.
    #[serde(default)]
    pub auto_increment: AutoIncrement,
}

fn default_pointer_mask() -> u16 {
    0xFF
}

fn default_pointer_width() -> u8 {
    1
}

/// Auto-increment policy for a register-file write-pointer. Checked **live**
/// (after the enabling byte is stored), so the very write that sets the enable
/// field also advances the pointer — matching PCA9685 silicon.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum AutoIncrement {
    /// Pointer never advances automatically.
    #[default]
    Never,
    /// Pointer advances after every data byte.
    Always,
    /// Pointer advances while `regs[addr] & mask != 0` (PCA9685 MODE1.AI).
    WhenFieldSet {
        /// Register offset holding the enable field.
        addr: u8,
        /// Bit mask of the enable field.
        mask: u8,
    },
}

/// Serde helper for the `when_field_set` map form of [`AutoIncrement`].
#[derive(Debug, Clone, Serialize, Deserialize)]
struct WhenFieldSetFields {
    addr: u8,
    mask: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AutoIncrementMap {
    #[serde(skip_serializing_if = "Option::is_none")]
    when_field_set: Option<WhenFieldSetFields>,
}

impl Serialize for AutoIncrement {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            AutoIncrement::Never => serializer.serialize_str("never"),
            AutoIncrement::Always => serializer.serialize_str("always"),
            AutoIncrement::WhenFieldSet { addr, mask } => AutoIncrementMap {
                when_field_set: Some(WhenFieldSetFields {
                    addr: *addr,
                    mask: *mask,
                }),
            }
            .serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for AutoIncrement {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let value = serde_yaml::Value::deserialize(deserializer)?;
        match &value {
            serde_yaml::Value::String(s) => match s.as_str() {
                "never" => Ok(AutoIncrement::Never),
                "always" => Ok(AutoIncrement::Always),
                other => Err(D::Error::unknown_variant(
                    other,
                    &["never", "always", "when_field_set"],
                )),
            },
            serde_yaml::Value::Mapping(_) => {
                let m: AutoIncrementMap =
                    serde_yaml::from_value(value).map_err(D::Error::custom)?;
                match m.when_field_set {
                    Some(f) => Ok(AutoIncrement::WhenFieldSet {
                        addr: f.addr,
                        mask: f.mask,
                    }),
                    None => Err(D::Error::custom("unknown auto_increment variant")),
                }
            }
            _ => Err(D::Error::custom(
                "expected string or map for auto_increment",
            )),
        }
    }
}

/// One self-driving update: a trigger paired with an action. See
/// [`I2cSpec::updates`].
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpdateRule {
    pub trigger: UpdateTrigger,
    pub action: UpdateAction,
}

/// What fires an [`UpdateRule`]. Currently only `read_complete` — a full
/// multi-byte read of the identified register finished.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpdateTrigger {
    pub read_complete: ReadComplete,
}

/// Identify the register a `read_complete` trigger watches: exactly one of a
/// register `name` or a `pointer` value.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReadComplete {
    #[serde(default)]
    pub register: Option<String>,
    #[serde(default)]
    pub pointer: Option<u16>,
}

/// What an [`UpdateRule`] does. Currently only `add_wrap`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpdateAction {
    pub add_wrap: AddWrap,
}

/// `value = value.wrapping_add(add) as i16; if value > max { value = reset }`
/// applied to the triggering register's stored word (signed i16 semantics).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AddWrap {
    pub add: i16,
    pub max: i16,
    pub reset: i16,
}

/// A named, channel-indexed engineering-unit value derived from the register
/// file. See [`I2cSpec::observables`].
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ObservableSpec {
    pub name: String,
    /// Number of channels (1 for a scalar observable).
    pub channels: u8,
    /// Register offset of channel 0's block.
    pub base: u8,
    /// Offset between consecutive channel blocks.
    pub stride: u8,
    /// How the raw value is composed from the channel block.
    pub value: ObservableValue,
    /// Optional raw → engineering-units mapping.
    #[serde(default)]
    pub map: Option<ObservableMap>,
}

/// Raw-value composition for an [`ObservableSpec`].
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ObservableValue {
    pub u12_compose: U12Compose,
}

/// `((regs[base+hi_rel] & hi_mask) << 8) | regs[base+lo_rel]`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct U12Compose {
    pub lo_rel: u8,
    pub hi_rel: u8,
    pub hi_mask: u8,
}

/// Raw → engineering-units mapping for an [`ObservableSpec`].
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ObservableMap {
    pub linear: LinearMap,
    /// Return `None` while the raw value is exactly 0 (channel never written).
    #[serde(default)]
    pub none_when_raw_zero: bool,
}

/// `eng = clamp(raw * scale + offset)`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LinearMap {
    pub scale: f64,
    pub offset: f64,
    /// Optional inclusive `[lo, hi]` clamp.
    #[serde(default)]
    pub clamp: Option<(f64, f64)>,
}

/// The `behavior.spi` section of a declarative `spi_device` — datasheet-shaped
/// wire framing for a register-style SPI sensor, interpreted by the engine's
/// generic device. The measurement→word machinery (`endian`/`source`/`encode`/
/// `scale_from` on each [`RegisterSpec`]) is shared verbatim with the I²C
/// primitive; only the leading-command framing differs.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SpiSpec {
    /// How the leading command byte encodes read/write + register address.
    #[serde(default)]
    pub framing: SpiFraming,
    /// Register map addressed by the command byte.
    #[serde(default)]
    pub registers: Vec<RegisterSpec>,
}

/// SPI command-byte framing. Defaults are the ADXL345 convention: one command
/// byte, bit 7 = read/write, bits [5:0] = register address, multi-byte bursts
/// auto-increment the address.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SpiFraming {
    /// Width of the leading command word in bytes. `0` = read-only part with no
    /// command (MAX31855: CS↓, clock out register 0); `1` = ADXL345-style.
    #[serde(default = "default_command_bytes")]
    pub command_bytes: u8,
    /// Bit position in the command byte selecting read vs write. `None` ⇒
    /// direction is fixed by each register's `access`.
    #[serde(default = "default_rw_bit")]
    pub rw_bit: Option<u8>,
    /// If true, `rw_bit` set (=1) means READ (ADXL345). If false, set means write.
    #[serde(default = "default_true")]
    pub rw_read_high: bool,
    /// Mask applied (after `addr_shift`) to the command byte to get the address.
    #[serde(default = "default_addr_mask")]
    pub addr_mask: u8,
    #[serde(default)]
    pub addr_shift: u8,
    /// A multi-byte burst walks ascending register addresses from the selected
    /// one; false ⇒ only the selected register is served.
    #[serde(default = "default_true")]
    pub auto_increment: bool,
}

impl Default for SpiFraming {
    fn default() -> Self {
        Self {
            command_bytes: default_command_bytes(),
            rw_bit: default_rw_bit(),
            rw_read_high: true,
            addr_mask: default_addr_mask(),
            addr_shift: 0,
            auto_increment: true,
        }
    }
}

fn default_command_bytes() -> u8 {
    1
}
fn default_rw_bit() -> Option<u8> {
    Some(7)
}
fn default_addr_mask() -> u8 {
    0x3F
}

/// CRC-8 parameters. Sensirion parts use `poly 0x31`, `init 0xFF`, no final XOR.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct Crc8Spec {
    pub poly: u8,
    pub init: u8,
    /// **What the checksum covers.** See [`Crc8Covers`]. Absent ⇒ `response`,
    /// the per-16-bit-word framing every descriptor written before this had.
    #[serde(default)]
    pub covers: Crc8Covers,
}

/// The two scopes a command device's CRC-8 is computed over.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Crc8Covers {
    /// One checksum byte after EVERY 16-bit word of the response, computed over
    /// that word alone. The Sensirion framing (SHT3x/SCD4x), and the default.
    #[default]
    Response,
    /// ONE checksum byte after the whole response, computed over the ADDRESSED
    /// SMBus frame: `[addr << 1, command, (addr << 1) | 1, data…]`.
    ///
    /// This is the SMBus Packet Error Code (SMBus 3.1 §6.4.1): the checksum
    /// covers the address and command bytes the master drove, not only the
    /// bytes the slave answered, so it cannot be computed from the response in
    /// isolation. The Melexis MLX90614 is the motivating case — its datasheet
    /// §8.4.3 "read word" is exactly `[addr·W, cmd, addr·R, LSB, MSB, PEC]`,
    /// and a driver that validates the PEC (which the good MLX drivers do)
    /// rejects every reading from a model that checksums the word alone.
    ///
    /// The address used is the address the device is ATTACHED at, so a part
    /// moved to a second address by `i2c_address:` still answers a PEC its
    /// driver accepts.
    Transaction,
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
pub enum RegisterAccess {
    R,
    Rw,
}

/// Back-compat aliases: the register/access structs were originally I²C-named.
/// Both declarative engines (I²C register-pointer, SPI CS-framed) share one
/// definition; these keep every existing `I2c*` reference compiling.
pub type I2cRegister = RegisterSpec;
pub type I2cAccess = RegisterAccess;

/// A pointer-addressable register.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RegisterSpec {
    pub name: String,
    /// Pointer the master writes to select this register.
    ///
    /// One byte on almost every part; two on a device that declares
    /// [`I2cSpec::pointer_width`] `2` (an EEPROM-style 16-bit address). The
    /// field is `u16` so both fit, and a YAML written when it was `u8` parses
    /// unchanged — `0x75` is the same number either way.
    pub addr: u16,
    /// Width in bytes streamed on read / accumulated on write.
    pub width: u8,
    pub endian: Endian,
    pub access: RegisterAccess,
    /// Bits of an `rw` register the master may actually change. Bits outside
    /// the mask keep their current value, so one register can mix firmware-owned
    /// configuration bits with silicon-owned read-only bits — a status flag the
    /// model drives (see [`DataReady`]), or a hardwired bit like the VCNL4010's
    /// `config_lock`. Absent ⇒ every bit is writable, which is what `rw` meant
    /// before this field existed (so old descriptors are unchanged).
    #[serde(default)]
    pub write_mask: Option<u32>,
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
    /// The register word is a signed two's-complement quantity of `width`
    /// bytes. A negative sourced measurement is encoded as its two's-complement
    /// bit pattern rather than clamped to zero. Default false (unsigned).
    #[serde(default)]
    pub signed: bool,
    /// Bit-field composition: when non-empty, the register word is ASSEMBLED
    /// from these fields (each a sourced measurement placed at a bit offset)
    /// rather than from the single top-level `source`. Used by parts like the
    /// MAX31855 whose 32-bit frame packs temperature + status sub-fields.
    #[serde(default)]
    pub fields: Vec<FieldSpec>,
    /// Register bank this register decodes in, for a device that declares
    /// [`I2cSpec::page_register`]. Absent ⇒ the register decodes in EVERY bank
    /// (the flat core map). Present ⇒ it decodes only while the bank select
    /// holds this value, so the same pointer can carry a different register per
    /// bank exactly as silicon does.
    #[serde(default)]
    pub page: Option<u8>,
    /// Bits the DEVICE clears itself the moment it has acted on the write —
    /// a momentary "go" bit, not a latch firmware owns.
    ///
    /// The VL53L0X `SYSRANGE_START` bit 0 is one: ST's `VL53L0X_StartMeasurement`
    /// writes it and then polls the register with the comment *"Wait until start
    /// bit has been cleared"*, giving up with `VL53L0X_ERROR_TIME_OUT` after
    /// `VL53L0X_DEFAULT_MAX_LOOP` polls. A plain `rw` register echoes the bit
    /// back forever, so that poll could never succeed.
    #[serde(default)]
    pub self_clearing: Option<u32>,
    /// Read value derived from the number of set bits in other registers rather
    /// than from storage or a measurement channel. See [`PopcountSource`].
    #[serde(default)]
    pub popcount: Option<PopcountSource>,
    /// Power-gate: while any masked bit of the named register is set, a read of
    /// THIS register returns an all-zero word instead of its measurement. See
    /// [`ZeroWhen`] — the VEML7700 `ALS_SD` shutdown bit is the motivating case.
    #[serde(default)]
    pub zero_when: Option<ZeroWhen>,
    /// The INVERTED power-gate: this register reads all-zero **unless** a
    /// masked bit of the named register is set. Same [`ZeroWhen`] shape, and a
    /// register declares at most one of the two (declaring both is a load
    /// error).
    ///
    /// Half the datasheets on the market spell the enable the other way round.
    /// The NXP MMA8451Q is the motivating case: §6.1 `CTRL_REG1.ACTIVE` is the
    /// bit that takes the part OUT of standby, so the output registers read
    /// zero while it is **clear** — the opposite polarity to the VEML7700's
    /// `ALS_SD` shutdown bit that [`ZeroWhen`] was written for.
    ///
    /// ## Why a second key and not `zero_when: { …, negate: true }`
    ///
    /// The polarity ends up in the KEY NAME, where the line reads as the
    /// datasheet sentence — `zero_unless: { register: CTRL_REG1, mask: 0x01 }`
    /// is "reads zero unless ACTIVE is set" — instead of in a boolean whose
    /// absence silently means one of the two. A missing or mistyped `negate:`
    /// would flip a power gate with no error: the part would read plausible
    /// measurements while it is supposed to be asleep, which is precisely the
    /// class of silent-fidelity bug the gate exists to expose. A misspelled
    /// KEY, by contrast, is an unknown field on a struct. The two spellings
    /// share one struct and one engine branch, so there is still exactly one
    /// definition of what a gate is.
    #[serde(default)]
    pub zero_unless: Option<ZeroWhen>,
    /// **Source multiplexer**: the stimulus channel this register reports is
    /// selected by a bit-field of ANOTHER register. See [`SourceFrom`].
    #[serde(default)]
    pub source_from: Option<SourceFrom>,
    /// NAMED bit-fields, so a Tier-2 rule can say `set: INT_STATUS.DATA_RDY`
    /// and `field(CONFIG.GAIN)` instead of carrying a hand-computed mask. Pure
    /// nomenclature: naming bits changes no read or write behaviour, which is
    /// why adding this to a shipped descriptor cannot move its transcript.
    ///
    /// Distinct from [`fields`](Self::fields), which ASSEMBLES a composite
    /// measurement word out of sourced sub-values. See [`BitFieldSpec`].
    #[serde(default)]
    pub bits: Vec<BitFieldSpec>,
    /// Datasheet side effect of a READ of this register, in the SystemRDL
    /// vocabulary the MCU register machine already uses ([`ReadAction`]).
    ///
    /// `clear` zeroes the stored word once the read **completes**, and
    /// "completes" is defined to be the exact moment the engine's existing
    /// [`DataReady::clear_on_read`] acts, so the two primitives can never
    /// disagree about when a read happened:
    ///   * register-pointer mode (no `auto_increment`) — when the pointed read
    ///     LATCHES, i.e. as the first byte of the word is produced. The master
    ///     still receives the pre-clear bytes; the datasheets word it as "reset
    ///     when the corresponding result register is read".
    ///   * byte-wise `auto_increment` mode, and an SPI burst — after the LAST
    ///     byte of the register's word has been handed to the master, so a
    ///     2-byte status is not zeroed while the master is still mid-read.
    ///
    /// A register with a `source:` reports its measurement, not storage, so a
    /// clear is observable only through what else reads that stored word
    /// (`scale_from`, `popcount`, `zero_when`).
    #[serde(default)]
    pub on_read: Option<ReadAction>,
    /// Datasheet side effect of a WRITE to this register ([`WriteAction`]).
    /// `write_one_to_clear` (`reg &= !data`), `write_zero_to_clear`
    /// (`reg &= data`) and `one_to_set` (`reg |= data`) all operate only on the
    /// bits [`RegisterSpec::write_mask`] lets the master touch; bits outside it
    /// keep their value exactly as they do for a plain store. Absent ⇒ a plain
    /// store, which is what every descriptor written before this field meant.
    #[serde(default)]
    pub on_write: Option<WriteAction>,
}

/// One sourced bit-field within a composite register word (see
/// [`RegisterSpec::fields`]). The encoded value occupies `width_bits` bits at
/// bit offset `shift`; `signed` packs negatives as two's-complement within
/// those bits.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FieldSpec {
    pub source: String,
    pub shift: u8,
    pub width_bits: u8,
    #[serde(default)]
    pub signed: bool,
    #[serde(default)]
    pub encode: Option<Encode>,
    /// Zero or more bit-field-selected scale factors, multiplied together and
    /// into `encode.scale` — the SAME shape, and the same engine helper, as
    /// [`RegisterSpec::scale_from`], applied to one field of a composite word.
    ///
    /// This is what makes a **left-justified** output register expressible. The
    /// NXP MMA8451Q is the motivating case: `OUT_X` (0x01) is a 16-bit word
    /// carrying a 14-bit signed count at bit 2, so the low two bits are always
    /// 0 on silicon (§6.2), and the counts-per-g the value is encoded at is
    /// chosen by `XYZ_DATA_CFG.FS` — 4096 / 2048 / 1024 for ±2/±4/±8 g
    /// (Table 5). Placing the value with `shift: 2` / `width_bits: 14` already
    /// rounds to 14 bits BEFORE the shift, which is the half that makes the low
    /// bits zero; without a per-field `scale_from` the full-scale select was
    /// simply unreachable, so a descriptor could have the justification or the
    /// range switch but never both.
    ///
    /// Spelled `scale_from` and not `justify:`/`shift:` because the shift is
    /// already `shift` — a second key meaning "shift" would be two ways to say
    /// one thing, and a descriptor that set both would have to define which
    /// wins.
    #[serde(default, deserialize_with = "de_scale_from_list")]
    pub scale_from: Vec<ScaleFrom>,
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
    /// **Modular wrap**, in RAW COUNTS: the encoded count is reduced
    /// `rem_euclid(wrap)` after rounding, so a register that is a modular
    /// counter rolls over instead of saturating.
    ///
    /// ## Why the counts and not the source value
    ///
    /// The AS5600 is the motivating part: a 12-bit magnetic encoder whose
    /// `RAW_ANGLE` is a 4096-count counter, and 4096 counts is the SAME shaft
    /// position as 0. The hand-written model expressed that as `deg % 360.0`
    /// on the stimulus, which is the same behaviour written in the unit the
    /// host happened to drive. Wrapping the counts is the property the silicon
    /// actually has — the register is N counts wide and rolls — so it is
    /// stated once per register and does not have to be restated for every
    /// unit a channel might carry (degrees, radians, turns). It also cannot
    /// produce a count the part cannot produce: `rem_euclid(4096)` can never
    /// yield 4096, whereas `value % 360.0` followed by a multiply can round up
    /// to exactly full scale.
    ///
    /// Applied AFTER `scale`/`offset`, after any `clamp_min`/`clamp_max`, and
    /// after rounding to an integer count — rounding first is what stops
    /// 359.99° (4095.99 counts) from being wrapped as 4095.99 and then rounded
    /// UP to an out-of-range 4096. A part that wraps normally declares no
    /// clamp: the two say opposite things about what happens at the end of the
    /// range, and `wrap` is the one a counter does.
    ///
    /// `NonZeroU32` rather than `u32` so `wrap: 0` is a load error naming the
    /// field instead of a silently ignored key — a modulus of zero has no
    /// meaning, and a typo that quietly disables a datasheet behaviour is the
    /// failure mode this schema is written to refuse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap: Option<std::num::NonZeroU32>,
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

/// A register-bit-field-keyed **source** map: which measurement channel a
/// register reports is chosen by another register's bit-field.
///
/// This is the datasheet shape for every multiplexed converter. The TI ADS1115
/// is the motivating case: its CONVERSION register (0x00) reports whichever
/// input the CONFIG register's MUX bits [14:12] select (§9.3.3, Table 8) —
/// four single-ended channels and four differential pairs. [`ScaleFrom`]
/// already covers the other half of that datasheet sentence (the PGA bits pick
/// the full scale), and this is the same extraction (`(reg >> shift) & mask`)
/// applied to the QUESTION rather than to the answer's scale.
///
/// The extraction is spelled exactly like [`ScaleFrom`]'s — `register`, `mask`,
/// `shift` — rather than naming a field, because the register map has no named
/// bit-fields to refer to: an author who has written one `scale_from` already
/// knows this, and the two cannot drift apart into two ways of saying "these
/// bits of that register".
///
/// A field value with no `table` entry falls back to the register's own
/// [`RegisterSpec::source`] when it declares one, and reads as 0 otherwise. A
/// table that covers every value the mask can produce therefore cannot be
/// surprised; one that does not says so by leaving `source` set.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SourceFrom {
    /// Name of the register whose bit-field selects the channel.
    pub register: String,
    /// Mask applied after `shift`.
    pub mask: u32,
    #[serde(default)]
    pub shift: u8,
    /// Extracted field value → the input (or [`DerivedChannel`]) key read.
    pub table: std::collections::BTreeMap<u32, String>,
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
    /// Word width in bytes. Default 2.
    #[serde(default = "default_response_width")]
    pub width: u8,
    /// Byte order of this word on the wire. Absent ⇒ `be`, the Sensirion
    /// 16-bit big-endian word every command descriptor written before this had.
    ///
    /// SMBus is little-endian by definition (SMBus 3.1 §6.5.5: "data is sent
    /// low byte first"), so every SMBus read-word part answers LSB then MSB —
    /// the MLX90614's §8.4.3 frame among them. A big-endian-only response word
    /// made those parts inexpressible: byte-swapping the value into the encode
    /// would produce a word whose two halves are a different measurement.
    #[serde(default = "default_response_endian")]
    pub endian: Endian,
    /// Linear encoding for a `source` word.
    #[serde(default)]
    pub encode: Option<Encode>,
}

fn default_response_endian() -> Endian {
    Endian::Be
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
    /// Source: a part attribute of this name. When `default_str` is set the
    /// value is emitted as a quoted string; otherwise it is parsed as f64.
    #[serde(default)]
    pub from_attr: Option<String>,
    /// Fallback for numeric `from_attr` when the attribute is absent or non-numeric.
    #[serde(default)]
    pub default: Option<f64>,
    /// Fallback for string `from_attr` when the attribute is absent or blank.
    /// Presence of this field selects the string emission path.
    #[serde(default)]
    pub default_str: Option<String>,
    /// Whether a missing pin binding suppresses the whole device. Defaults to
    /// true; optional feedback signals such as encoder index set this false.
    #[serde(default = "default_true")]
    pub required: bool,
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
    /// For the `spi_device` primitive: the datasheet-shaped SPI wire framing the
    /// engine's generic SPI device interprets. Absent for non-SPI primitives.
    #[serde(default)]
    pub spi: Option<SpiSpec>,
    /// For the `analog_source` primitive: the datasheet output curve the
    /// engine's generic analog device interprets. Absent for non-analog
    /// primitives.
    #[serde(default)]
    pub analog: Option<AnalogSpec>,
    /// For the `display` primitive: the datasheet-shaped description of a
    /// framebuffer panel — geometry, pixel format, RAM layout, command/data
    /// framing and the command table. Absent for non-display primitives.
    #[serde(default)]
    pub display: Option<DisplaySpec>,

    // ── Tier 2 (`crates/config/src/rules.rs`) ──────────────────────────────
    //
    // Every field below is optional and defaults to empty, so a Tier-1
    // descriptor deserialises byte for byte as it did before they existed.
    /// Declared states. The FIRST is the reset state. Empty ⇒ the part has one
    /// implicit state named `""` and `state == …` is never true.
    #[serde(default)]
    pub states: Vec<String>,
    /// Integer variables and their reset values. The scratch a rule needs that
    /// is not a register the master can see — a bit counter, a latched opcode.
    #[serde(default)]
    pub vars: BTreeMap<String, i64>,
    /// Sample queues (see [`FifoSpec`]).
    #[serde(default)]
    pub fifos: Vec<FifoSpec>,
    /// Pin ROLES this part drives. Each binds to a pad through a `config:` key
    /// exactly as [`pins`](Self::pins) does — the key is the role name unless
    /// [`output_pins`](Self::output_pins) maps it to a different one.
    #[serde(default)]
    pub outputs: Vec<String>,
    /// Optional role → `config:` key map for [`outputs`](Self::outputs), for a
    /// part whose config key is not simply the role name (`INT` → `int_pin`).
    #[serde(default)]
    pub output_pins: BTreeMap<String, String>,
    /// Message framing for a command-shell part (see [`FrameSpec`]).
    #[serde(default)]
    pub frames: Option<FrameSpec>,
    /// The rules themselves (see [`Rule`]). Fire in declaration order.
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// **Free-running device timers** — the part's own clock, not the bus's.
    /// Each fires [`TimingAction`]s into the register file after a delay
    /// (`after_us`) or on a period (`period_us`), advanced by the device's
    /// `advance_time_us` hook. See [`DeviceTimer`]. Empty ⇒ the device has no
    /// clock of its own, which is every descriptor written before this existed.
    #[serde(default)]
    pub timers: Vec<DeviceTimer>,
    /// **Derived measurement channels** — named values computed from the
    /// device's stimulus channels (and from earlier derived names) by a small
    /// arithmetic expression, evaluated fresh on every read. A `source:` on a
    /// register, a field or a response word may name one exactly as it names a
    /// stimulus channel. Empty ⇒ the read path is untouched, which is every
    /// descriptor written before this existed. See [`DerivedChannel`].
    #[serde(default)]
    pub derived: Vec<DerivedChannel>,
}

/// One **derived channel**: a named value computed from other channels.
///
/// This is the datasheet shape for a register that reports a QUANTITY THE PART
/// COMPUTES rather than one it senses. The INA219 is the motivating case: its
/// POWER register (0x03) is `bus_mV × |I_mA| / 1000` in 2 mW units (§8.5.4) —
/// a product of two stimulus channels. `source:` takes one key and `scale_from`
/// scales by a *register* field, so before this the only options were a POWER
/// register left at reset-0 (a power monitor that reports no power to
/// `getPower_mW()`) or a third `power` stimulus channel the host would have to
/// keep consistent with the other two by hand.
///
/// ## The expression language, and why it is this small
///
/// `expr` is infix arithmetic over:
///   * **names** — a `metadata.inputs` channel key, or a derived channel
///     declared EARLIER in this list;
///   * **decimal literals** — `500.0`, `-1`, `1e3`;
///   * **operators** — `+ - * /` with the usual precedence, unary `-`, and
///     parentheses;
///   * **functions** — `abs(x)`, `min(a, b)`, `max(a, b)`. Nothing else.
///
/// There is no `round`, no `floor`, no conditional and no comparison, because
/// each of those is a decision about the part's QUANTISER or its STATE, and
/// both of those belong to primitives that say so: rounding to a register count
/// is [`Encode`]'s job (one rule for every device), and a value that depends on
/// what the part is currently doing is a state machine, not an expression.
///
/// ## Evaluation order and cycles
///
/// Channels are evaluated in declaration order, so a later one may read an
/// earlier one. A name that is neither a declared input nor an EARLIER derived
/// channel is a **load error** naming both the channel and the name — which is
/// also why a cycle cannot be written: `a` cannot see `b` unless `b` is above
/// it, so no chain can close. The alternative (resolve by dependency and detect
/// cycles) would let a descriptor read top-to-bottom in an order it is not
/// evaluated in, and a reader holding the datasheet would have to build the
/// graph in their head to know what `POWER` means.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DerivedChannel {
    /// The name a `source:` addresses this value by. Must not collide with a
    /// declared `metadata.inputs` key: a `source:` naming both would be
    /// ambiguous, so that is a load error rather than a precedence rule.
    pub name: String,
    /// The arithmetic expression, in the grammar above.
    pub expr: String,
}

/// One free-running timer owned by a declarative device.
///
/// This is the datasheet shape for everything a part does on its OWN schedule
/// rather than in answer to a bus transaction: a continuous-conversion sensor
/// that refreshes a data register every N µs, a one-shot whose result appears
/// `after_us` after firmware wrote the start bit, a watchdog that sets a fault
/// flag. [`DataReady`] covers the narrow start-bit → status-bit case; a timer
/// covers the rest, and both are driven by the same simulated µs.
///
/// **Exactly one** of `period_us` (repeating) and `after_us` (one-shot) is
/// declared, and it must be non-zero — a zero-period timer would fire an
/// unbounded number of times in one `advance_time_us` call.
///
/// **Ordering.** When several timers come due inside one time advance they
/// fire in ascending deadline order, ties broken by declaration order, and a
/// periodic timer that is due more than once fires once per elapsed period.
/// The sequence is therefore a pure function of (elapsed µs, declaration
/// order) — identical on native and wasm.
///
/// ⚠️ A timer only advances on a bus that drives the device's `advance_time_us`
/// hook. On a chip with no absolute-µs source the device's clock never moves
/// and no timer ever fires — the same holdout [`DataReady`] documents.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct DeviceTimer {
    /// Diagnostic name. Not addressable from the bus.
    pub name: String,
    /// Repeating period in µs. Mutually exclusive with `after_us`.
    #[serde(default)]
    pub period_us: Option<u64>,
    /// One-shot delay in µs, measured from the moment the timer starts.
    /// Mutually exclusive with `period_us`.
    #[serde(default)]
    pub after_us: Option<u64>,
    /// When the timer starts running. [`TimerStart::OnReset`] (the default) is
    /// a part that free-runs from power-on; [`TimerStart::Manual`] waits for
    /// `start_on_write`.
    #[serde(default)]
    pub start: TimerStart,
    /// A write that (re)starts this timer. Present ⇒ the timer restarts from
    /// the moment of that write, whatever `start` says.
    #[serde(default)]
    pub start_on_write: Option<TimerStartOnWrite>,
    /// What firing does, by register NAME — the same [`TimingAction`] the MCU
    /// register machine's [`TimingDescriptor`] uses, so there is one vocabulary
    /// for "the device changed a register by itself". The device owns these
    /// writes, so `write_mask` (which protects silicon's bits from FIRMWARE)
    /// does not restrict them.
    #[serde(default)]
    pub on_fire: Vec<TimingAction>,
}

/// When a [`DeviceTimer`] begins running.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TimerStart {
    /// Free-running from reset (power-on).
    #[default]
    OnReset,
    /// Idle until something starts it — today, a `start_on_write`.
    Manual,
}

/// The write that starts a [`DeviceTimer`].
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct TimerStartOnWrite {
    /// Register whose write starts the timer.
    pub register: String,
    /// Bits that must be left SET by the write for it to start (level, not
    /// edge — a driver re-issues the same on-demand bit for every reading,
    /// exactly as [`DataReady::start_mask`] documents). Absent ⇒ ANY write to
    /// the register starts it, which is the "write anything to trigger" idiom.
    #[serde(default)]
    pub mask: Option<u32>,
}

/// The `behavior.analog` section of a declarative `analog_source` — a
/// datasheet-shaped description of a part whose whole interface is one
/// analogue voltage (a Sharp IR ranger's Vo, an MQ-x module's AOUT). The
/// generic engine device interprets it as an `AnalogSource`: a drivable
/// input channel (declared under `metadata.inputs`, exactly one for this
/// primitive) mapped through a piecewise-linear curve to millivolts.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AnalogSpec {
    /// The output curve as `(input value, output mV)` points, ascending in
    /// input. Piecewise-linear between neighbours. Read straight off the
    /// datasheet's typical-output graph.
    pub curve: Vec<(f32, f32)>,
    /// Behaviour below the first curve point. `clamp` (the only mode) holds
    /// the first point's voltage: a region the datasheet does not specify is
    /// held constant rather than invented.
    #[serde(default)]
    pub below_first: AnalogBelowFirst,
    /// Behaviour above the last curve point: hold a named floor voltage
    /// (`floor_mv`, e.g. the GP2Y0A21's ~0.4 V far floor — far is NOT zero)
    /// or hold the last point's voltage (`hold_last`).
    #[serde(default)]
    pub above_last: AnalogAboveLast,
}

/// Out-of-band behaviour below the first curve point (see [`AnalogSpec`]).
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AnalogBelowFirst {
    /// Hold the first curve point's voltage.
    #[default]
    Clamp,
}

/// Out-of-band behaviour above the last curve point (see [`AnalogSpec`]).
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Default)]
pub struct AnalogAboveLast {
    /// Hold an explicit floor voltage (mV) — the honest model of a part whose
    /// output settles on a non-zero floor beyond its specified band. Absent ⇒
    /// hold the last curve point's voltage.
    #[serde(default)]
    pub floor_mv: Option<f32>,
}

// ─── the `display` primitive ────────────────────────────────────────────────
//
// A framebuffer panel is not a register file, which is why it needed a
// primitive of its own rather than another `spi_device` with a long register
// list. What a display controller datasheet actually states is: the frame
// memory's extent and pixel format, how a byte on the wire is told apart from
// a command (a D/C pad on SPI, a control byte on I²C), which opcodes move the
// address counters, and how those counters wrap. All five are data. The engine
// (`peripherals/components/declarative_display.rs`) is the only place that
// knows what "wrap into the next page" means.
//
// WHAT IS DELIBERATELY NOT HERE: pixel-value transforms. The model stores what
// firmware wrote, byte for byte. Gamma, colour inversion and contrast are
// recorded as panel FLAGS in the artifact's meta, never applied to the stored
// bytes — a twin that pre-rendered its own idea of the picture could not be
// compared against a photograph of the glass.

/// The `behavior.display` section: one framebuffer panel, entirely as data.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DisplaySpec {
    /// Frame-memory width in pixels. THE CONTROLLER'S frame memory, not the
    /// glass — a 170×320 module is a smaller glass wired to a subset of a
    /// 240-column controller, and firmware picks the strip with the window
    /// commands. Use `glass_crop` to expose the crop as `config:` keys.
    pub width: u16,
    /// Frame-memory height in pixels.
    pub height: u16,
    pub pixel_format: DisplayPixelFormat,
    /// `crate::inspect::artifact_format` name the paint artifact carries, so a
    /// consumer decoding the bytes reads the same string it always did.
    pub artifact_format: String,
    pub ram: DisplayRam,
    /// How a command byte is told apart from a data byte.
    pub dc: DisplayDc,
    #[serde(default)]
    pub addressing: DisplayAddressing,
    /// Power-on window, when it is not simply the whole frame memory.
    #[serde(default)]
    pub window: DisplayWindow,
    /// MADCTL-style orientation bits, for controllers whose frame memory is
    /// addressed in a rotated coordinate system. Absent ⇒ no rotation.
    #[serde(default)]
    pub orientation: Option<DisplayOrientation>,
    /// Named integer cells a command can store into (`set_var`) and the
    /// orientation reads. Values are the power-on / reset contents.
    #[serde(default)]
    pub vars: std::collections::BTreeMap<String, u32>,
    /// I²C slave address, for a panel framed by a control byte. Ignored for a
    /// D/C-pin panel, which is selected by CS.
    #[serde(default)]
    pub default_address: Option<u8>,
    /// This module's supply connection is modelled: the engine exposes a
    /// `powered` config key, refuses the bus when it is explicitly `false`, and
    /// reports `powered` in the artifact. See the ST7789 descriptor for why an
    /// ABSENT key means powered.
    #[serde(default)]
    pub supply_gated: bool,
    /// The glass may show a strip of the frame memory: the engine exposes
    /// `col_offset` / `row_offset` / `cols` / `rows` config keys, all-or-nothing,
    /// and crops the artifact to that fixed physical window.
    #[serde(default)]
    pub glass_crop: bool,
    /// The command table. An opcode with no entry here is consumed and ignored,
    /// which is what a controller does with a command it does not implement.
    #[serde(default)]
    pub commands: Vec<DisplayCommand>,
}

/// How the frame memory encodes a pixel.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DisplayPixelFormat {
    /// 1 bpp, one byte = 8 vertically-stacked pixels of one page (SSD1306,
    /// SH1107, PCD8544). The write unit is one byte.
    MonoPage,
    /// 16 bpp, big-endian on the wire (ST7789, ILI9341). The write unit is two
    /// bytes, high byte first.
    Rgb565,
    /// 24 bpp, one byte per channel.
    Rgb888,
    /// Two 1 bpp planes, black then red (tri-colour e-paper).
    Tricolor,
}

impl DisplayPixelFormat {
    /// Bytes the controller accumulates before it commits one write unit and
    /// advances the address counters.
    pub fn write_unit_bytes(self) -> usize {
        match self {
            Self::MonoPage | Self::Tricolor => 1,
            Self::Rgb565 => 2,
            Self::Rgb888 => 3,
        }
    }
}

/// Frame-memory shape.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DisplayRam {
    /// Page count for a page-major panel (height / 8). Absent for row-major.
    #[serde(default)]
    pub pages: Option<u16>,
    /// Total frame-memory size. Stated so the descriptor says what the
    /// controller holds; the engine CHECKS it against the geometry rather than
    /// trusting it, so the two cannot drift apart.
    pub bytes: u32,
    pub layout: DisplayRamLayout,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DisplayRamLayout {
    /// `byte(page × width + column)` — the paged OLED/LCD GDDRAM.
    PageMajor,
    /// `pixel(row × width + column)` — the linear TFT frame memory.
    RowMajor,
}

/// Where the command/data distinction comes from.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DisplayDc {
    pub source: DisplayDcSource,
    /// `pin` only: the abstract pin role, resolved to a pad through the
    /// `dc_pin` config key. Named for symmetry with `behavior.pins`.
    #[serde(default)]
    pub pin_role: Option<String>,
    /// `pin` only: the D/C level that frames a COMMAND (0 for every MIPI DCS
    /// panel). A data byte is the other level.
    #[serde(default)]
    pub command_level: u8,
    /// `control_byte` only: the control-byte value that opens a COMMAND
    /// stream. Any other value opens the data stream — which is what the
    /// SSD1306 does with Co/D̄C̄ (0x00 vs 0x40).
    #[serde(default)]
    pub command_value: Option<u8>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DisplayDcSource {
    /// A dedicated D/C pad sampled at transfer time (4-wire SPI).
    Pin,
    /// The first byte of each I²C transaction selects the stream for the rest
    /// of it. Command PARAMETERS then arrive on the command stream, and every
    /// data-stream byte is frame memory.
    ControlByte,
}

/// Which addressing modes the controller implements and which one it powers on
/// in.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DisplayAddressing {
    #[serde(default)]
    pub modes: Vec<DisplayAddressingMode>,
    #[serde(default)]
    pub default: DisplayAddressingMode,
}

impl Default for DisplayAddressing {
    fn default() -> Self {
        Self {
            modes: vec![DisplayAddressingMode::Horizontal],
            default: DisplayAddressingMode::Horizontal,
        }
    }
}

/// How the address counters advance after a write unit is committed.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DisplayAddressingMode {
    /// Column first; past the column end, back to the column start and on to
    /// the next page/row; past that end too, back to the start of both.
    #[default]
    Horizontal,
    /// Page first; past the page end, back to the page start and on to the next
    /// column. Page-major panels only.
    Vertical,
    /// Column only, clamped at the last column of the frame memory: the page
    /// never changes and nothing wraps. Page-major panels only.
    Page,
}

/// Power-on window, where it is not the whole frame memory. Each absent bound
/// defaults to the last column / row / page the geometry allows, which is what
/// every panel here powers on with.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct DisplayWindow {
    #[serde(default)]
    pub col_end: Option<u16>,
    #[serde(default)]
    pub row_end: Option<u16>,
    #[serde(default)]
    pub page_end: Option<u16>,
}

/// MADCTL-style orientation: which bits of which var swap and mirror the axes.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DisplayOrientation {
    /// Name of the var (see [`DisplaySpec::vars`]) holding the orientation byte.
    pub var: String,
    /// Exchange page and column address order (MADCTL MV). Changes what a legal
    /// column IS, so it also moves the window clamp.
    pub swap_bit: u8,
    /// Mirror the column address order (MADCTL MX).
    pub mirror_x_bit: u8,
    /// Mirror the page address order (MADCTL MY).
    pub mirror_y_bit: u8,
}

/// One entry of the controller's command table.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DisplayCommand {
    pub opcode: u8,
    /// Inclusive end of an opcode RANGE, for the families that encode an
    /// argument in the opcode's low bits (SSD1306 `0xB0..=0xB7` = set page).
    /// Absent ⇒ this entry is the single `opcode`.
    #[serde(default)]
    pub opcode_end: Option<u8>,
    /// Datasheet mnemonic. Carried for error messages and review; nothing
    /// dispatches on it.
    #[serde(default)]
    pub name: Option<String>,
    /// Parameter bytes this command consumes before its actions run.
    #[serde(default)]
    pub args: u8,
    #[serde(default, rename = "do")]
    pub actions: Vec<DisplayAction>,
}

/// What a command does when its parameters are complete.
///
/// Written in YAML as a ONE-KEY MAP — `{ set_window: { … } }`, `{ invert: true }`,
/// `{ reset_control: true }` — which is the shape the rest of the part schema
/// uses and the shape an LLM writes without being told. It is a struct of
/// optional fields rather than a Rust enum because `serde_yaml` renders an
/// externally-tagged enum as a YAML `!tag`, and a schema whose action syntax is
/// `!set_window` in one place and `{ key: value }` everywhere else is a schema
/// people get wrong. Exactly one field must be set; the engine's descriptor
/// validation refuses zero or two, so a typo'd action name is a load error that
/// names the command rather than a silent no-op.
///
/// These are the DISPLAY-SPECIFIC actions, and they exist because the Phase C
/// rule vocabulary (`set`/`clear` bits, `write REG = expr`, `goto`, `timer`,
/// `push`/`pop`, `pin`) has no word for an address window or a RAM stream.
/// Everything a controller command does to a framebuffer is one of these nine.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct DisplayAction {
    /// Set one axis's window `[start, end]` and move that axis's cursor to the
    /// start (CASET / RASET / SSD1306 0x21 / 0x22).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_window: Option<DisplaySetWindow>,
    /// Move one axis's cursor without touching its window (SSD1306 set-page and
    /// the two column-nibble commands).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_cursor: Option<DisplaySetCursor>,
    /// Select the addressing mode by index into `addressing.modes`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_mode: Option<DisplayValue>,
    /// Store an integer into a named var (MADCTL).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_var: Option<DisplaySetVar>,
    /// Open the RAM write stream; subsequent data bytes are pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ram_write: Option<DisplayRamWrite>,
    /// DISPON / DISPOFF.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_on: Option<bool>,
    /// SLPOUT / SLPIN. A panel that never woke is dark whatever is in memory,
    /// so this is reported beside `display_on` rather than folded into it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awake: Option<bool>,
    /// INVON / INVOFF. Recorded, never applied to the stored bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invert: Option<bool>,
    /// Software reset: control state (flags, vars, window, cursors) returns to
    /// power-on. FRAME MEMORY IS NOT CLEARED — ST7789V §9.1.22 p.202,
    /// "Contents of memory is not cleared" — so a painted frame survives.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reset_control: bool,
}

impl DisplayAction {
    /// How many of the mutually exclusive action fields this entry sets.
    /// Anything but 1 is a descriptor error.
    pub fn arms_set(&self) -> usize {
        self.set_window.is_some() as usize
            + self.set_cursor.is_some() as usize
            + self.set_mode.is_some() as usize
            + self.set_var.is_some() as usize
            + self.ram_write.is_some() as usize
            + self.display_on.is_some() as usize
            + self.awake.is_some() as usize
            + self.invert.is_some() as usize
            + self.reset_control as usize
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DisplaySetWindow {
    pub axis: DisplayAxis,
    pub start: DisplayValue,
    pub end: DisplayValue,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DisplaySetCursor {
    pub axis: DisplayAxis,
    #[serde(default)]
    pub part: DisplayCursorPart,
    pub value: DisplayValue,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DisplaySetVar {
    pub name: String,
    pub value: DisplayValue,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DisplayRamWrite {
    /// Reset the cursors to the window start (RAMWR, 0x2C). `false` continues
    /// from where the last write stopped (WRMEMC, 0x3C — §9.1.33 p.225).
    #[serde(default = "default_true")]
    pub reset_cursor: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DisplayAxis {
    Col,
    Row,
    Page,
}

/// Which part of a cursor a `set_cursor` replaces. The SSD1306 sets a column
/// in two halves (0x00..0x0F low nibble, 0x10..0x1F high nibble), so a
/// read-modify-write on the cursor is part of the command set, not a quirk.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DisplayCursorPart {
    #[default]
    All,
    LowNibble,
    HighNibble,
}

/// Where a command action's integer comes from. EXACTLY ONE source field must
/// be set; the mask and the clamp are applied after it, in that order.
///
/// Masking and clamping are both here and both explicit because the two panels
/// ported first disagree about which they do, and the difference is visible:
/// the SSD1306 MASKS a column bound (`0x21` keeps the low 7 bits, so 200
/// becomes 72) while the ST7789 CLAMPS it (`.min(239)`, so 200 stays 200 and
/// 300 becomes 239). A schema that offered only one of the two would have
/// silently moved one panel's pixels.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct DisplayValue {
    /// Source: a constant.
    #[serde(default)]
    pub value: Option<u32>,
    /// Source: `(opcode & opcode_mask) >> opcode_shift`, for an opcode range.
    #[serde(default)]
    pub opcode_mask: Option<u8>,
    #[serde(default)]
    pub opcode_shift: u8,
    /// Source: these parameter-byte indices, most-significant first.
    #[serde(default)]
    pub args: Option<Vec<u8>>,
    /// Applied to the source value.
    #[serde(default)]
    pub mask: Option<u32>,
    /// Clamp to the last legal index on the action's axis, in the CURRENT
    /// orientation (the swap bit changes what a legal column is).
    #[serde(default)]
    pub clamp_axis_max: bool,
}

impl DeviceDescriptor {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        serde_yaml::from_str(yaml).context("Failed to parse Device Descriptor")
    }

    /// Look up and parse the embedded descriptor for a device `type:` string
    /// (accepts either spelling for the encoder). Returns `Ok(None)` for a type
    /// with no declarative descriptor. This is the SINGLE embed point — the
    /// runtime attach path (`core`'s `bus/declarative_device.rs`) resolves
    /// descriptors through here, so there is one source of truth for the
    /// `configs/devices/*.yaml` set.
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
        "adxl345_spi" => Some(include_str!("../../../configs/devices/adxl345_spi.yaml")),
        "max31855" => Some(include_str!("../../../configs/devices/max31855.yaml")),
        "bh1750" => Some(include_str!("../../../configs/devices/bh1750.yaml")),
        "veml7700" => Some(include_str!("../../../configs/devices/veml7700.yaml")),
        "tmp102" => Some(include_str!("../../../configs/devices/tmp102.yaml")),
        "mcp9808" => Some(include_str!("../../../configs/devices/mcp9808.yaml")),
        "pca9685" => Some(include_str!("../../../configs/devices/pca9685.yaml")),
        "vcnl4010" => Some(include_str!("../../../configs/devices/vcnl4010.yaml")),
        "pcf8574" => Some(include_str!("../../../configs/devices/pcf8574.yaml")),
        "vl53l0x" => Some(include_str!("../../../configs/devices/vl53l0x.yaml")),
        "as5600" => Some(include_str!("../../../configs/devices/as5600.yaml")),
        "sht30" => Some(include_str!("../../../configs/devices/sht30.yaml")),
        "at24c256" => Some(include_str!("../../../configs/devices/at24c256.yaml")),
        "tmp117" => Some(include_str!("../../../configs/devices/tmp117.yaml")),
        "ina219" => Some(include_str!("../../../configs/devices/ina219.yaml")),
        "ads1115" => Some(include_str!("../../../configs/devices/ads1115.yaml")),
        "mma8451q" => Some(include_str!("../../../configs/devices/mma8451q.yaml")),
        "fxos8700" => Some(include_str!("../../../configs/devices/fxos8700.yaml")),
        "mlx90614" => Some(include_str!("../../../configs/devices/mlx90614.yaml")),
        "oled-ssd1306" => Some(include_str!("../../../configs/devices/ssd1306.yaml")),
        "oled-ssd1306-128x32" => Some(include_str!("../../../configs/devices/ssd1306_128x32.yaml")),
        "st7789-170x320" => Some(include_str!("../../../configs/devices/st7789.yaml")),
        "gp2y0a21" => Some(include_str!("../../../configs/devices/gp2y0a21.yaml")),
        "dc-motor" | "dc_motor" => Some(include_str!("../../../configs/devices/dc_motor.yaml")),
        "bldc-motor" | "bldc_motor" => {
            Some(include_str!("../../../configs/devices/bldc_motor.yaml"))
        }
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
                // r.size is already a byte count; it used to be formatted to
                // "<n>B" purely so this field could hold a String, then parsed
                // straight back at every read. That round-trip is gone.
                size: r.size,
            })
            .unwrap_or(MemoryRange { base: 0, size: 0 });

        let ram = ir
            .memory_regions
            .get("RAM")
            .map(|r| MemoryRange {
                base: r.base,
                // r.size is already a byte count; it used to be formatted to
                // "<n>B" purely so this field could hold a String, then parsed
                // straight back at every read. That round-trip is gone.
                size: r.size,
            })
            .unwrap_or(MemoryRange { base: 0, size: 0 });

        Self {
            schema_version: default_schema_version(),
            name: ir.name,
            arch,
            core,
            // An SVD/IR document describes a register map, not a clock tree —
            // it has no core frequency to carry over. `0` is the honest answer
            // ("undeclared"); attach sites keep their historical default for it
            // rather than inventing a number here.
            cpu_hz: 0,
            flash,
            ram,
            reset_vector_offset: 0,
            atomic_register_aliases: AtomicAliasFlavour::None,
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
                        irq_controller: None,
                        clock: None,
                        config: std::collections::HashMap::from([(
                            "internal_ir_peripheral".to_string(),
                            serde_yaml::to_value(p).unwrap(),
                        )]),
                    }
                })
                .collect(),
            pins: std::collections::BTreeMap::new(),
            analog_pins: Default::default(),
            io_voltage_v: None,
            gpio_input_thresholds: None,
            include: None,
        }
    }
}

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

fn default_first_occurrence() -> u32 {
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

/// Assert what a display device actually PAINTED, over a bounded region of its
/// own pixel grid.
///
/// One primitive for every panel — ILI9341, SSD1306, SH1107, tri-color e-paper,
/// the parallel ILI9341 — because it is keyed by the `external_devices:` id and
/// reads the framebuffer artifact the model already publishes, never a
/// per-model accessor.
///
/// **Why a region plus an ink RANGE, and not a lit-pixel count.** Two real
/// failures had to be distinguishable, and a count only separates one of them:
///
/// * A panel that paints NOTHING (a declared-but-undriven D/C line latches low,
///   every byte frames as a command, not one pixel lands) has zero ink.
/// * A panel that paints the wrong thing — a desynchronised command stream
///   writing command bytes into frame memory as pixels — has plenty of ink, in
///   roughly the right place, and sails past any "did it paint?" threshold.
///
/// Bounding the region and bounding the ink from BOTH sides is what tells those
/// apart: a header band the firmware fills solid must come back essentially
/// fully inked, and noise in that band does not. `max_ink` is the half that
/// makes the second case fail, so it is not optional decoration — a region with
/// `min_ink: 0.0` and no `max_ink` asserts nothing at all and
/// [`TestScript::validate`] rejects it.
///
/// A digest of the whole framebuffer would also catch both, exactly, and was
/// rejected: it fails on any legitimate change, says nothing about WHERE the
/// picture went wrong, and cannot be written by hand from a datasheet or a
/// photograph of the real panel.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct DisplayRegionDetails {
    /// `external_devices:` id of the display (e.g. `"tft"`).
    pub id: String,
    /// Region origin, in the panel's own pixel coordinates. Defaults to (0, 0).
    #[serde(default)]
    pub x: usize,
    #[serde(default)]
    pub y: usize,
    /// Region size. Defaults to the rest of the panel from (`x`, `y`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<usize>,
    /// Lower bound on the fraction (0.0..=1.0) of the region's pixels that must
    /// carry ink — non-black on an emissive panel, non-white on e-paper.
    pub min_ink: f64,
    /// Upper bound on the same fraction. Absent means 1.0 (no upper bound),
    /// which is only allowed when `min_ink` is itself above zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_ink: Option<f64>,
    /// Require the panel to be EMITTING, not merely painted.
    ///
    /// Ink measures frame memory, and frame memory fills whether or not the
    /// panel can show it. On an emissive display those are different
    /// questions: an AMOLED has no backlight and its brightness lives in the
    /// controller (DCS `WRDISBV`, reset 0x00), so firmware ported from a
    /// backlit TFT driver paints a perfect frame and displays black.
    ///
    /// This is not hypothetical and it is why the field exists: deleting the
    /// one `WRDISBV` write from the nRF54LM20A snake firmware left its lab
    /// passing 7/7, because every assertion measured pixels that had genuinely
    /// been written to a panel nobody could see.
    ///
    /// Only meaningful for a panel that publishes `meta.lit`; asking it of one
    /// that does not is an error rather than a pass, on the same principle as
    /// every other way of not-measuring here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lit: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct DisplayRegionAssertion {
    pub display_region: DisplayRegionDetails,
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
struct StimulusSpecYaml {
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

fn default_stack_paint() -> bool {
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

/// Structural guard for a `display_region` assertion.
///
/// The interesting clause is the last one. `min_ink: 0.0` with no `max_ink`
/// accepts every possible framebuffer, including one that was never written —
/// a gate that cannot fail, which is worse than no gate because it reads as
/// coverage. Both other clauses are ordinary range checks.
fn validate_display_region(index: usize, d: &DisplayRegionDetails) -> Result<()> {
    if d.id.trim().is_empty() {
        anyhow::bail!("assertions[{index}]: display_region.id cannot be empty");
    }
    for (name, v) in [("min_ink", Some(d.min_ink)), ("max_ink", d.max_ink)] {
        if let Some(v) = v {
            if !v.is_finite() || !(0.0..=1.0).contains(&v) {
                anyhow::bail!(
                    "assertions[{index}]: display_region.{name} must be a fraction in 0.0..=1.0 (got {v})"
                );
            }
        }
    }
    if let Some(max) = d.max_ink {
        if max < d.min_ink {
            anyhow::bail!(
                "assertions[{index}]: display_region.max_ink ({max}) is below min_ink ({})",
                d.min_ink
            );
        }
    }
    if d.min_ink == 0.0 && d.max_ink.is_none() {
        anyhow::bail!(
            "assertions[{index}]: display_region with min_ink 0.0 and no max_ink accepts every \
             possible framebuffer, including one the firmware never wrote. Give it a floor \
             (min_ink) to prove the region was painted, or a ceiling (max_ink) to prove it was \
             left clear."
        );
    }
    Ok(())
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
    uart_injections: FieldPresence<Vec<UartInjectionSpec>>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    uart_injections: Option<Option<&'a [UartInjectionSpec]>>,
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

/// Per-kind structural validation of a fault spec: that the target shape and the
/// kind-specific parameters required to lower the fault are present. This is the
/// config-side half of the fault compiler; silicon-resolution guardrails (does
/// the peripheral exist, is the bit within the register) run against the built
/// bus at run time.
fn validate_fault(f: &FaultSpec) -> Result<()> {
    // Every implemented fault is lowered onto the bus before the firmware runs
    // (see `labwired_cli::faults`), and nothing evaluates a fault's trigger
    // after that. A later trigger would therefore fire at start while the
    // script said otherwise, so refuse it the way stimuli refuse the triggers
    // they do not wire.
    if f.trigger != FaultTrigger::AtStart {
        anyhow::bail!(
            "Fault '{}' ({:?}): trigger {:?} is not yet supported for faults; every fault is \
             applied when the bus is built (use at_start, or omit trigger)",
            f.id,
            f.kind,
            f.trigger
        );
    }
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
#[path = "lib_parse_size_tests.rs"]
mod parse_size_tests;

#[cfg(test)]
#[path = "lib_stimuli_tests.rs"]
mod stimuli_tests;

#[cfg(test)]
#[path = "lib_uart_injection_tests.rs"]
mod uart_injection_tests;

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "lib_memory_size_tests.rs"]
mod memory_size_tests;

#[cfg(test)]
#[path = "lib_pin_map_tests.rs"]
mod pin_map_tests;

#[cfg(test)]
#[path = "lib_can_player_path_inline_tests.rs"]
mod can_player_path_inline_tests;

#[cfg(test)]
#[path = "lib_builtin_chip_tests.rs"]
mod builtin_chip_tests;
