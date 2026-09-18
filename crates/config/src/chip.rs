// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

#![allow(dead_code)]
use crate::*;

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

pub(crate) fn default_clock_controller() -> String {
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
pub(crate) struct IrqTarget {
    line: Option<u32>,
    controller: Option<String>,
}

pub(crate) fn irq_line_from_number(n: &serde_yaml::Number) -> Result<u32, String> {
    if let Some(u) = n.as_u64() {
        u32::try_from(u).map_err(|_| format!("irq: line {u} is out of range"))
    } else if let Some(i) = n.as_i64() {
        u32::try_from(i).map_err(|_| format!("irq: {i} is not a valid line number"))
    } else {
        Err(format!("irq: expected an integer line number, got {n}"))
    }
}

pub(crate) fn parse_irq_string(s: &str) -> Result<IrqTarget, String> {
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

pub(crate) fn parse_irq_value(value: serde_yaml::Value) -> Result<IrqTarget, String> {
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

pub(crate) fn deserialize_irq_target<'de, D>(deserializer: D) -> Result<IrqTarget, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_yaml::Value::deserialize(deserializer)?;
    parse_irq_value(value).map_err(serde::de::Error::custom)
}

#[derive(Deserialize)]
pub(crate) struct PeripheralConfigWire {
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
pub(crate) struct GpioInputThresholdsYaml {
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
pub(crate) fn deserialize_io_voltage<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
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
pub(crate) fn deserialize_atomic_alias_flavour<'de, D>(d: D) -> Result<AtomicAliasFlavour, D::Error>
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
    /// Non-secure peripheral alias window. When set, an MMIO access that maps
    /// to no peripheral window but whose address plus this offset DOES map to
    /// one is served by that peripheral — the TrustZone NS alias of a secure
    /// peripheral map.
    ///
    /// The nRF54L family exposes every peripheral twice: the secure alias at
    /// `0x5000_0000+` (the devicetree default this chip YAML maps) and the
    /// non-secure alias exactly `0x1000_0000` below it
    /// (`USE_NON_SECURE_ADDRESS_MAP`). An NS firmware image addresses
    /// `0x400D_8200` where the mapped descriptor says `0x500D_8200`; without
    /// this key those accesses map to nothing. The offset is applied ONLY as a
    /// fallback for addresses that are otherwise unmapped, so it can never
    /// shadow a peripheral the chip declares, and a chip that omits the key is
    /// byte-identical to before. Accepts an integer or the same
    /// `"0x…"`/`_`-separated string forms as `base_address`.
    #[serde(
        default,
        deserialize_with = "deserialize_opt_u64_lax",
        skip_serializing_if = "Option::is_none"
    )]
    pub ns_alias_offset: Option<u64>,
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

/// Reject `config_overrides` on the YAML wire before Serde collapses an absent
/// field, `{}`, and `null` into the same empty `HashMap`. Programmatic callers
/// cannot express that distinction, but every user-facing environment manifest
/// passes through [`EnvironmentManifest::from_file`].
pub(crate) fn reject_explicit_node_config_overrides(wire: &serde_yaml::Value) -> Result<()> {
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

pub(crate) fn validate_environment_interconnect_config(
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

pub(crate) fn reject_unknown_interconnect_config_keys(
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

pub(crate) fn optional_nonempty_interconnect_string<'a>(
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

pub(crate) fn yaml_str_key(key: &str) -> serde_yaml::Value {
    serde_yaml::Value::String(key.to_string())
}

/// `from_str` accepts `size: 1024` as a string field; `from_value` does not.
/// Walk mappings and stringify numeric `size` so include-merge can deserialize
/// without a YAML text round-trip.
pub(crate) fn coerce_yaml_size_numbers(value: &mut serde_yaml::Value) {
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

pub(crate) fn mapping_field<'a>(value: &'a serde_yaml::Value, key: &str) -> Option<&'a str> {
    value.as_mapping()?.get(yaml_str_key(key))?.as_str()
}

pub(crate) fn take_includes(doc: &mut serde_yaml::Value) -> Result<Vec<String>> {
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

pub(crate) fn merge_seq_by(
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

pub(crate) fn merge_yaml_maps(
    base: serde_yaml::Value,
    overlay: serde_yaml::Value,
) -> serde_yaml::Value {
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

pub(crate) fn merge_chip_yaml(
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

pub(crate) fn expand_chip_includes(
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
            ns_alias_offset: None,
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

#[cfg(test)]
#[path = "lib_pin_map_tests.rs"]
mod pin_map_tests;

#[cfg(test)]
#[path = "lib_builtin_chip_tests.rs"]
mod builtin_chip_tests;
