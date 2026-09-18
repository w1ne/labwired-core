// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

#![allow(dead_code)]
use crate::*;

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
pub(crate) struct TimingActionFields {
    register: String,
    #[serde(default)]
    bits: Option<u32>,
    #[serde(default)]
    value: Option<u32>,
}

/// The derived shape, used only to accept the `!set_bits` tag form.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TimingActionTagged {
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
    /// Name of a `config:` key whose value OVERRIDES
    /// [`noise_sigma`](Self::noise_sigma) for this channel when the placement
    /// sets it.
    ///
    /// The descriptor's own `noise_sigma` is a property of the part; this is
    /// the knob a board hands the user. A part whose datasheet quotes one noise
    /// figure for a whole channel SET — an IMU's six axes, a magnetometer's
    /// three — names the same key on each of them, which is how `noise_sigma:
    /// 0.02` on an `external_devices` entry reaches all six axes at once. It is
    /// spelled once per channel rather than as a group, so a part whose axes
    /// have genuinely different figures can still say so, and so that reading
    /// one channel's entry tells you everything that moves it.
    ///
    /// The key must also be declared in `metadata.config_keys` to be advertised
    /// in the peripheral manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noise_sigma_key: Option<String>,
    /// Constant offset applied to the channel value, in `unit`.
    #[serde(default)]
    pub bias: Option<f64>,
    /// Factor `input(KEY)` multiplies this channel by before it becomes the
    /// INTEGER a rule expression sees. Absent ⇒ 1.0.
    ///
    /// The rule language is integer-only, and `input()` is defined as "the
    /// value as the part reports it" — for a register device that is the
    /// register's own `encode:`, which is why a rule comparing `input(x)`
    /// against `reg(DATA)` compares like with like. A pins-only part has no
    /// register to borrow an encoding from, so it states the same thing here:
    /// the counts its protocol shifts out per engineering unit.
    ///
    /// The HX711 is the motivating case. Its channel is grams and its frame is
    /// 24 bits at 100 counts per gram; without this, `input(weight)` truncates
    /// to whole grams and a load cell loses exactly the digits it exists to
    /// measure — silently, because 10 g and 10.5 g both read 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expr_scale: Option<f64>,
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

pub(crate) fn default_pointer_bytes() -> u8 {
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

pub(crate) fn default_pointer_mask() -> u16 {
    0xFF
}

pub(crate) fn default_pointer_width() -> u8 {
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
pub(crate) struct WhenFieldSetFields {
    addr: u8,
    mask: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AutoIncrementMap {
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
    /// **Flat RAM behind the declared map.** Present ⇒ every command-byte
    /// address that no [`registers`](Self::registers) entry covers is one byte
    /// of this array: a read serves it, a write stores it. Absent ⇒ an
    /// undeclared address reads `0xFF` (open bus) and swallows writes, which is
    /// what every descriptor written before this key existed meant.
    ///
    /// This is the shape of a **register shell** — a part whose datasheet map
    /// is a few meaningful registers in a large space of storage the driver
    /// configures and reads back. Three shipped models were exactly that and
    /// nothing else: the SX1278's 128 bytes, the MFRC522's 64 and the
    /// nRF24L01+'s 24, each a `[u8; N]` behind an address/data phase machine.
    /// Declaring them one `RegisterSpec` at a time would mean inventing a name
    /// per address, and an address left undeclared is not a blank — it reads
    /// `0xFF` and drops the driver's write, which is a different part.
    ///
    /// Declared registers still WIN at their own addresses, so a part may mix
    /// the two: the nRF24L01+'s STATUS is a `write_one_to_clear` register and
    /// the other twenty-three addresses are storage.
    #[serde(default)]
    pub register_file: Option<SpiRegisterFile>,
}

/// Flat byte storage backing an SPI part's undeclared addresses
/// (see [`SpiSpec::register_file`]).
///
/// Deliberately NOT [`RegisterFileSpec`], which is the I²C key: that one owns a
/// write-POINTER (`pointer_mask`, `first_write_after_start_sets_pointer`,
/// `auto_increment`) because an I²C register-file part selects its address with
/// a bus write. An SPI part's address comes out of the command byte and its
/// walk is [`SpiFraming::auto_increment`], so those three keys would be dead
/// fields a descriptor could set and have ignored.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SpiRegisterFile {
    /// Number of one-byte cells. An address at or above it is not backed, so it
    /// reads `0xFF` and drops writes exactly as an undeclared address does
    /// without this key.
    pub size: usize,
    /// Value every cell powers up holding, before `reset` is stamped over it.
    /// Absent ⇒ 0.
    #[serde(default)]
    pub fill: Option<u8>,
    /// Sparse power-on values, keyed by address.
    #[serde(default)]
    pub reset: BTreeMap<u16, u8>,
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
    /// **The command byte is an OPCODE plus an address**, not one direction
    /// bit: `mosi & op_mask` selects the operation and is compared against
    /// [`op_read`](Self::op_read) and [`op_write`](Self::op_write). Absent ⇒
    /// [`rw_bit`](Self::rw_bit) decides, which is every descriptor written
    /// before this key existed.
    ///
    /// It WINS over `rw_bit`. `rw_bit` carries a non-`None` default, so there
    /// is no way to tell a defaulted one from a declared one and "declaring
    /// both is an error" would reject every descriptor that sets `op_mask`. The
    /// op field is the more specific statement of the same datasheet sentence,
    /// so it is the one that decides.
    ///
    /// A command byte matching NEITHER value selects no register at all. Its
    /// data phase serves [`command_response`](Self::command_response) (or
    /// `0xFF` without one) and drops writes — which is what a part does with a
    /// command that is not a register access. The nRF24L01+ (§8.3.1, Table 19)
    /// is the motivating case: `R_REGISTER` is `000A AAAA` and `W_REGISTER` is
    /// `001A AAAA`, but `W_TX_PAYLOAD` is `1010 0000` and `FLUSH_RX` is
    /// `1110 0010`. Decoded by bit 5 alone, a 32-byte `W_TX_PAYLOAD` burst
    /// writes its payload over CONFIG, EN_AA, EN_RXADDR and the rest — the
    /// register file silently destroyed by the command that sends a packet.
    #[serde(default)]
    pub op_mask: Option<u8>,
    /// Value of the `op_mask` field that means "read the addressed register".
    #[serde(default)]
    pub op_read: Option<u8>,
    /// Value of the `op_mask` field that means "write the addressed register".
    #[serde(default)]
    pub op_write: Option<u8>,
    /// **Register whose word is clocked OUT while the command byte is clocked
    /// IN.** Absent ⇒ `0x00`, the byte every descriptor written before this key
    /// returned during the command phase.
    ///
    /// nRF24L01+ §8.3.1: "the STATUS register is serially shifted out on the
    /// MISO pin simultaneously with the command word on MOSI". Every RF24-style
    /// driver reads its interrupt flags that way — `write_register` returns the
    /// byte the command phase produced — so a part that answers `0x00` there
    /// reports no interrupt has ever fired.
    #[serde(default)]
    pub command_response: Option<String>,
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
            op_mask: None,
            op_read: None,
            op_write: None,
            command_response: None,
        }
    }
}

pub(crate) fn default_command_bytes() -> u8 {
    1
}
pub(crate) fn default_rw_bit() -> Option<u8> {
    Some(7)
}
pub(crate) fn default_addr_mask() -> u8 {
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
    /// **FIFO drain port**: while the named FIFO is NON-EMPTY, a read of this
    /// register serves one packed component of its oldest entry instead of the
    /// live [`source`](Self::source). See [`RegisterFifo`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fifo: Option<RegisterFifo>,
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
    /// **Streaming port**: the byte-wise auto-increment pointer does NOT
    /// advance past this register. The register IS the port, and what moves is
    /// an internal address counter the master cannot address.
    ///
    /// Absent ⇒ the pointer steps, which is every register written before this
    /// key existed.
    ///
    /// The Bosch BMI270's config upload is the motivating case: §"Initialization
    /// sequence" has the host stream the ~8 KB feature-engine image into
    /// `INIT_DATA` (0x5E) in one burst, with `INIT_ADDR` advancing inside the
    /// part. A pointer that stepped per byte would walk the whole map thirty-two
    /// times in that one transaction — over `ACC_CONF`, over `PWR_CTRL`, and
    /// over `CMD` (0x7E), where one byte in every 256 of a firmware image is
    /// `0xB6` and issues a SOFT RESET. The upload would reset the part it is
    /// trying to initialise, repeatedly, and the handshake it exists to satisfy
    /// could never complete.
    ///
    /// It holds the pointer in BOTH directions, because that is what a port is:
    /// a part's FIFO data register (the BMI270's own `FIFO_DATA`, 0x24) is read
    /// the same way it is written.
    #[serde(default)]
    pub stream: bool,
    /// **Civil-calendar decomposition** of the register's `source:` channel,
    /// which must carry Unix seconds (UTC). Present ⇒ a read reports THIS field
    /// of that instant rather than the instant itself, and a write to the
    /// register RECOMPOSES — it replaces this field of the sourced channel and
    /// leaves the other six alone. See [`CalendarField`].
    ///
    /// The DS3231's `0x00..=0x06` are exactly this: one settable clock read out
    /// as seven registers. Modelling them as seven independent storage bytes is
    /// what makes a model where writing `SECONDS` and reading `MINUTES` can
    /// disagree about which minute it is, and where advancing time moves
    /// nothing. Modelling them as seven *read-only* views of one channel is the
    /// opposite failure: `RTClib::adjust()` is the first call every sketch
    /// makes, and it would do nothing at all.
    ///
    /// The conversion is Howard Hinnant's civil-from-days / days-from-civil
    /// pair, in UTC, with no leap seconds — the same arithmetic the hand-written
    /// DS3231 model used, so the port reproduces its transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar: Option<CalendarField>,
}

/// One field of a civil calendar instant (see [`RegisterSpec::calendar`]).
/// `Year` is the two-digit year an RTC holds (`0..=99`), `Weekday` is 1..=7
/// with Sunday = 1, which is the DS3231/DS1307 convention.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CalendarField {
    Second,
    Minute,
    Hour,
    Weekday,
    Day,
    Month,
    Year,
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
pub(crate) fn de_scale_from_list<'de, D>(deserializer: D) -> Result<Vec<ScaleFrom>, D::Error>
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
    /// **Binary-coded decimal**, applied SYMMETRICALLY at the wire boundary:
    /// a read encodes the integer count as packed BCD (two decimal digits per
    /// byte, tens in the high nibble), and a write decodes the BCD the master
    /// put on the wire back to an integer before anything stores it.
    ///
    /// This is the real-time-clock register shape (DS3231, DS1307, PCF8563):
    /// `12` seconds reads as `0x12`, and a driver that writes `0x59` means 59.
    /// It is the LAST step of the read encode — after `scale`, `offset`, the
    /// clamp window and `wrap` — and the FIRST step of the write decode, so the
    /// stored word and every expression that reads it (`reg()`, `field()`,
    /// `scale_from`) are in decimal, never in nibbles. A model that stored the
    /// nibbles instead would make `reg(SECONDS) < 60` a guard that is false for
    /// a third of every minute.
    ///
    /// A nibble above 9 is not a decimal digit. On the write side such a byte
    /// is decoded the way the silicon's counter chain does — `(hi & 0xF) * 10 +
    /// (lo & 0xF)`, so `0x1A` is 20 — and on the read side a count wider than
    /// the digits the register holds saturates at all-nines rather than
    /// wrapping into a neighbouring field.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bcd: bool,
    /// **Which bits carry the NUMBER.** Bits outside this mask are plain flags:
    /// stored as written and served back verbatim, untouched by the numeric
    /// encoding. Absent ⇒ the whole word is the number, which is what every
    /// descriptor written before this key means.
    ///
    /// ## Why a register needs this
    ///
    /// The DS3231's alarm registers are the case. `A1M1` is bit 7 of the SAME
    /// byte whose low seven bits are the BCD seconds, and the four mask bits
    /// are what decide the alarm RATE — once a second, when the seconds match,
    /// when the minutes and seconds match, and so on. A `bcd:` that claims the
    /// whole word runs the flag through the nibble decode, so `0x89` ("mask
    /// set, 9 seconds") stores as 89 and reads back `0x89` only by accident;
    /// masking the flag away instead — which is what those registers did before
    /// this key — makes alarm matching unexpressible, because the bit that
    /// decides the rate is gone.
    ///
    /// With `value_mask`, the stored word is `number | flags` and both halves
    /// survive a round trip:
    ///
    /// ```yaml
    /// - { name: ALARM1_SECONDS, addr: 0x07, width: 1, access: rw,
    ///     encode: { bcd: true, value_mask: 0x7F },
    ///     bits: [{ name: A1M1, shift: 7 }] }
    /// ```
    ///
    /// A rule then reads the number as `reg(ALARM1_SECONDS) & 0x7F` and the
    /// flag as `field(ALARM1_SECONDS.A1M1)` — two independent things in one
    /// byte, which is what the silicon has.
    ///
    /// ⚠️ The number must fit inside the mask in BOTH domains: decimal 59 is
    /// `0x3B` and its BCD form is `0x59`, and both sit inside `0x7F`. A mask
    /// too narrow for the BCD form would truncate the tens digit on the wire.
    ///
    /// Only meaningful with [`bcd`](Self::bcd) today, and only on a STORAGE
    /// register — a register with a `source:` computes its whole word at read
    /// time and has no stored flags to preserve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_mask: Option<u32>,
    /// Rounding applied to the encoded value before it becomes an integer
    /// count. Absent ⇒ [`Rounding::Nearest`], which is what every descriptor
    /// written before this field existed means (`f64::round`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round: Option<Rounding>,
    /// Field-driven clamp: the saturation window is read from another
    /// register's bit-field instead of being the constant `clamp_min`/
    /// `clamp_max` pair. See [`ClampFrom`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clamp_from: Vec<ClampFrom>,
}

/// One register's view into a FIFO — the drain seam.
///
/// ## Why "non-empty", rather than a mode flag
///
/// A part with a FIFO has a BYPASS mode in which its data registers serve the
/// live conversion, and a FIFO mode in which the same registers walk the queue.
/// Both behaviours are already implied by the queue itself: in bypass the
/// [`FifoFill`](crate::FifoFill) guard is false, nothing is ever pushed, the
/// FIFO is always empty, and the register falls through to `source:`.
///
/// So there is no second mode switch to keep in step with the first. A
/// descriptor that gets its fill guard right gets its read path right for
/// free, and a Tier-1 register with no `fifo:` is untouched.
///
/// ## Popping
///
/// `pop: true` on the LAST slot a driver reads is what advances the queue. The
/// ADXL345's burst is `DATAX0 .. DATAZ1`, so `DATAZ0` carries the pop; a driver
/// that stops after X gets the same sample again, which is exactly what the
/// silicon does with a FIFO whose read was abandoned.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct RegisterFifo {
    /// The FIFO this register drains.
    pub name: String,
    /// Which packed component of the entry, indexing
    /// [`FifoFill::pack`](crate::FifoFill::pack).
    pub slot: u8,
    /// Whether completing a read of this register POPS the entry.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pop: bool,
}

/// How an encoded value becomes an integer count.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Rounding {
    /// `f64::round` — half away from zero. The default, and what every
    /// descriptor written before `round:` existed means.
    #[default]
    Nearest,
    /// `f64::floor` — toward negative infinity. This is the division a
    /// counter-field decomposition needs: `hour = floor(t / 3600) mod 24` is
    /// the hour, while rounding to nearest makes the second half of every hour
    /// report the next one.
    Floor,
    /// `f64::ceil` — toward positive infinity.
    Ceil,
    /// `f64::trunc` — toward zero.
    Trunc,
}

/// A register-bit-field-keyed clamp window, the mirror of [`ScaleFrom`]. The
/// engine extracts `(value(register) >> shift) & mask` and looks the field
/// value up in `map`; the entry is the `[min, max]` window, in RAW COUNTS,
/// applied where the constant `clamp_min`/`clamp_max` pair is applied.
///
/// ## Why a part needs this
///
/// The ADXL345 is the motivating case. Its output is 10 bits in the default
/// mode and up to 13 in FULL_RES, and the range the part saturates at is
/// `DATA_FORMAT[1:0]` — ±2/4/8/16 g. Both halves are firmware-owned and both
/// move the saturation point, so a constant `clamp_max` would be right for
/// exactly one of the four settings and would silently stop the part
/// saturating at the other three. The clamp is a *consequence* of a register
/// the driver wrote, the same way [`ScaleFrom`] makes the LSB size one.
///
/// A field value absent from `map` leaves the constant window (or no window)
/// in force, which is the same "unmapped ⇒ neutral" rule `scale_from` has.
/// Several `clamp_from` entries INTERSECT: each narrows the window, so a part
/// whose resolution bit and range bits both bound the count states each once.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ClampFrom {
    /// Name of the register whose bit-field selects the window.
    pub register: String,
    /// Mask applied after `shift`.
    pub mask: u32,
    #[serde(default)]
    pub shift: u8,
    /// Extracted field value → `[min, max]` window in raw counts.
    pub map: std::collections::BTreeMap<u32, ClampWindow>,
}

/// One `[min, max]` saturation window in raw counts (see [`ClampFrom`]).
#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub struct ClampWindow {
    pub min: f64,
    pub max: f64,
}

pub(crate) fn one_f64() -> f64 {
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

pub(crate) fn default_response_endian() -> Endian {
    Endian::Be
}

pub(crate) fn default_response_width() -> u8 {
    2
}

pub(crate) fn default_code_width() -> u8 {
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
    /// For the `led_strip` primitive: an addressable LED strip's wire protocol
    /// and artifact contract. Absent for every other primitive.
    #[serde(default)]
    pub led_strip: Option<LedStripSpec>,
    /// For the `uart_device` primitive: the part's frame shape, its command
    /// table and what it says unprompted. See [`UartSpec`]. Absent for every
    /// other primitive.
    #[serde(default)]
    pub uart: Option<UartSpec>,
    /// For the `logic_gate` primitive: a 74-series part's truth table, its
    /// enables, its direction/select control and its propagation delay. See
    /// [`LogicSpec`]. Absent for every other primitive.
    ///
    /// A gate has no bus at all — no address, no register, nothing to read
    /// back — so none of the `i2c`/`spi`/`uart` blocks above can describe one.
    #[serde(default)]
    pub logic: Option<LogicSpec>,

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
    ///
    /// When [`period_from`](Self::period_from) is also declared, this is the
    /// period the source register's RESET value gives — the rate the part ticks
    /// at before firmware writes anything — and the field takes over from the
    /// first write onward.
    #[serde(default)]
    pub period_us: Option<u64>,
    /// **Field-driven period**: the repeating period is looked up from another
    /// register's bit-field instead of being the constant
    /// [`period_us`](Self::period_us). See [`TimerPeriodFrom`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub period_from: Option<TimerPeriodFrom>,
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

/// A register-bit-field-keyed **timer period**, the timing twin of
/// [`ScaleFrom`] and [`ClampFrom`].
///
/// ## Why a part needs this
///
/// A sample rate is a register on nearly every part that has one, and a
/// constant `period_us` is right for exactly one setting of it. Four shipped
/// descriptors said so in their own headers before this key existed:
///
/// * **DS3231** `CONTROL.RS2:RS1` — the INT/SQW square wave is 1 Hz, 1.024 kHz,
///   4.096 kHz or 8.192 kHz. The power-on value is 8.192 kHz, so a model with a
///   constant 1 Hz was not merely inflexible, it was wrong at reset.
/// * **ADXL345** `BW_RATE[3:0]` — sixteen output data rates, 3200 Hz halving
///   down to 0.098 Hz. A driver that asks for 800 Hz and gets 100 Hz sees one
///   sample in eight.
/// * **MPU6050** `SMPLRT_DIV` + `DLPF_CFG`, and **HX711**'s gain pulses.
///
/// ## The shape, and why it is a TABLE
///
/// The engine extracts `(reg(register) >> shift) & mask` — or the named
/// `field:`, which is the same thing spelled the way the datasheet spells it —
/// and looks the value up in `table`, whose values are periods in µs.
///
/// A table rather than an arithmetic rule because that is the shape of the
/// datasheet: these are enumerations with footnotes, not formulas. Even the
/// ADXL345's, which IS a clean halving, has a non-halving low end in the
/// datasheet's own table. A part whose rate genuinely is a formula over a wide
/// field (the MPU6050's 8-bit `SMPLRT_DIV`) does not fit here and is named as
/// still-blocked rather than approximated by a 256-row table.
///
/// An **unmapped** field value leaves [`DeviceTimer::period_us`] in force —
/// the same "unmapped ⇒ neutral" rule `scale_from` and `clamp_from` have — so
/// a reserved encoding does not silently stop the part's clock.
///
/// ## When the period changes under a RUNNING timer
///
/// The deadline is re-anchored to `now + the new period`. It is not
/// recomputed from the old deadline: firmware that rewrites the rate register
/// has restarted the divider, and keeping the old anchor would make the first
/// interval after the change a length that neither setting has.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct TimerPeriodFrom {
    /// Name of the register whose bit-field selects the period.
    pub register: String,
    /// A named `bits:` field of that register. Exactly one of this and
    /// [`mask`](Self::mask) is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// An explicit mask, applied after [`shift`](Self::shift), for a part that
    /// has no name for the bits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<u32>,
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub shift: u8,
    /// Extracted field value → repeating period in µs. A zero period is a load
    /// error: it would fire without bound.
    pub table: std::collections::BTreeMap<u32, u64>,
}

fn is_zero_u8(v: &u8) -> bool {
    *v == 0
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
    /// Panel flags as they stand at power-on, before firmware sends anything.
    /// Every MIPI DCS panel powers on dark and asleep (all `false`, the
    /// default); the PCD8544 powers on with its display-control D bit set and
    /// its power-down bit clear, so a bench module lights up before any
    /// `display_on` command. A model that assumed dark would report a blank
    /// panel for firmware that legitimately never sends one.
    #[serde(default)]
    pub power_on: DisplayPowerOn,
    /// What a CS assert does to a half-open stream.
    #[serde(default)]
    pub cs_select: DisplayCsSelect,
    /// Which panel flags and counters the paint artifact's `meta` carries, and
    /// under what key.
    ///
    /// NOT a house style with per-format defaults: every consumer that decodes
    /// a panel — the browser overlay, the CLI's `painted bytes=` line, the
    /// evidence tests — reads these names, so which keys a panel publishes is
    /// part of its contract and belongs in its descriptor. `w`, `h`, `format`
    /// and `generation` are always present because they describe the payload
    /// itself; everything else is listed here.
    pub artifact_meta: Vec<DisplayMetaField>,
    /// Extra conditions, beyond DISPON and awake, that must hold for the panel
    /// to emit light.
    ///
    /// Forced by the RM67162, and general to every emissive panel. A backlit
    /// TFT's brightness is a separate pin the controller knows nothing about,
    /// so `display_on AND awake` is the whole truth there. An AMOLED has no
    /// backlight: brightness lives INSIDE the controller (`WRDISBV`, DCS 0x51)
    /// and its reset value is 0x00, i.e. black. Firmware ported from a TFT
    /// sends a perfect init and a full frame, never writes 0x51, and shows
    /// nothing on the bench. Without this a model would report that firmware
    /// `lit` and flatter a driver that cannot work.
    #[serde(default)]
    pub lit_requires: Vec<DisplayLitRequirement>,
    /// A BUSY line the host polls, and the level it rests at when idle.
    ///
    /// Only e-paper has one, and THE TWO E-PAPERS HERE DISAGREE ABOUT THE
    /// POLARITY: the SSD1680 asserts BUSY high, so idle is low; the UC8151D
    /// pulls it low while busy and releases it high. GxEPD2 blocks in
    /// `_waitWhileBusy` until it reads not-busy, with a 30 s escape timeout
    /// that at simulated speed is ~10^7 steps per refresh and reads to a user
    /// as hung firmware. Driving the line to the WRONG idle level is therefore
    /// indistinguishable from not driving it at all, and a house default would
    /// pick one panel's polarity and hang the other.
    ///
    /// These models refresh instantaneously, so "always idle" is the faithful
    /// reading — the line is driven once, at attach, and never moves.
    #[serde(default)]
    pub busy: Option<DisplayBusy>,
}

/// See [`DisplaySpec::busy`].
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct DisplayBusy {
    /// Config key naming the pad, so a board that does not wire BUSY simply
    /// omits it.
    pub config_key: String,
    /// The level BUSY rests at when the controller is not refreshing.
    pub idle_level: bool,
}

/// One clause of [`DisplaySpec::lit_requires`]: a declared var that must be at
/// least `min` for the panel to be lit.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct DisplayLitRequirement {
    /// Name of the var (see [`DisplaySpec::vars`]).
    pub var: String,
    /// The smallest value that still emits light. `1` for a brightness whose
    /// reset value is 0.
    pub min: u32,
}

/// What a CS assert does to a stream that is already open.
///
/// Both readings are real and the two panels here disagree. The ST7789 model
/// treats CS as the transaction boundary: a half-sent command does not survive
/// a deselect. The ILI9341 model deliberately lets a RAMWR pixel stream survive
/// one, because a driver that chunks a large blit releases CS between bursts
/// and expects the pointer to be where it left it — closing the stream there
/// paints the first chunk and drops the rest.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DisplayCsSelect {
    /// CS assert closes the open stream and discards a partial command.
    #[default]
    ClosesStream,
    /// CS assert changes nothing; the stream and the address counters survive.
    KeepsStream,
}

/// Panel flags at power-on. See [`DisplaySpec::power_on`].
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
pub struct DisplayPowerOn {
    #[serde(default)]
    pub display_on: bool,
    #[serde(default)]
    pub awake: bool,
    #[serde(default)]
    pub inverted: bool,
}

/// One entry of [`DisplaySpec::artifact_meta`]: a flag, optionally published
/// under a different key than its engine name.
///
/// Written as either `- lit_pixels` or `- { flag: lit, as: display_on }`. The
/// rename exists because the same panel fact has a different published name on
/// different panels — the PCD8544 has always reported its inverse-video bit as
/// `inverse`, and renaming it to `inverted` in a port would break the browser
/// overlay that reads it.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum DisplayMetaField {
    Flag(DisplayMetaFlag),
    Renamed {
        flag: DisplayMetaFlag,
        #[serde(rename = "as")]
        published_as: String,
    },
    /// The CURRENT VALUE OF A DECLARED VAR, raw or hex-formatted.
    ///
    /// Written `- { var: brightness }` or `- { var: colmod, format: hex8 }`.
    /// Forced by the RM67162, whose artifact has always carried `brightness`
    /// as a number and `colmod` / `madctl` as `"0x55"`-style strings. The
    /// formatting is part of the published contract — a consumer that parsed
    /// `"0x55"` reads `85` if the key silently becomes a number — so it is
    /// stated per entry rather than guessed from the value.
    Var {
        var: String,
        /// Publish under this key instead of the var's own name.
        #[serde(default, rename = "as")]
        published_as: Option<String>,
        #[serde(default)]
        format: DisplayMetaFormat,
    },
    /// INKED BYTES OF ONE NAMED PLANE — bytes that are not [`DisplayRam::blank`].
    ///
    /// Written `- { plane: black }` (key `black_ink_bytes`) or
    /// `- { plane: black, of: screen }` (key `screen_black_ink_bytes`).
    /// A separate entry from [`DisplayMetaFlag::InkBytes`] because that one
    /// counts the WHOLE frame memory, and on a two-plane e-paper the two
    /// planes are independent pictures: one number for both cannot say which
    /// colour is on the glass, and both deleted models published the two
    /// counts separately.
    Plane {
        plane: String,
        /// Which copy of the plane to count. See [`DisplayPlaneOf`].
        #[serde(default)]
        of: DisplayPlaneOf,
        /// Publish under this key instead of the derived one.
        #[serde(default, rename = "as")]
        published_as: Option<String>,
    },
}

/// Which copy of a plane a [`DisplayMetaField::Plane`] entry counts.
///
/// THE E-PAPER DISTINCTION, and the reason `refresh` exists: frame memory is
/// not the screen. Firmware that writes a new frame and never activates has
/// changed `ram` and changed nothing a camera can see.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DisplayPlaneOf {
    /// The controller's frame memory, as written by the wire.
    #[default]
    Ram,
    /// What the last `refresh` action put on the glass.
    Screen,
}

/// How a [`DisplayMetaField::Var`] renders into the artifact's `meta`.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMetaFormat {
    /// A JSON number.
    #[default]
    Raw,
    /// `"0xNN"` — two hex digits, upper case.
    Hex8,
    /// `"0xNNNN"` — four hex digits, upper case.
    Hex16,
}

impl DisplayMetaField {
    /// The flag this entry publishes, or `None` for a var / plane entry.
    pub fn flag(&self) -> Option<DisplayMetaFlag> {
        match self {
            Self::Flag(f) => Some(*f),
            Self::Renamed { flag, .. } => Some(*flag),
            Self::Var { .. } | Self::Plane { .. } => None,
        }
    }

    /// The var this entry reads, or `None` for a flag / plane entry.
    pub fn var(&self) -> Option<&str> {
        match self {
            Self::Var { var, .. } => Some(var),
            _ => None,
        }
    }

    /// The plane this entry counts and which copy of it, or `None`.
    pub fn plane(&self) -> Option<(&str, DisplayPlaneOf)> {
        match self {
            Self::Plane { plane, of, .. } => Some((plane, *of)),
            _ => None,
        }
    }

    pub fn format(&self) -> DisplayMetaFormat {
        match self {
            Self::Var { format, .. } => *format,
            _ => DisplayMetaFormat::Raw,
        }
    }

    /// The `meta` key this entry publishes under.
    ///
    /// Borrowed for every entry that names its own key, OWNED for the one that
    /// derives it (`{ plane: black }` → `black_ink_bytes`). A `Cow` rather than
    /// a leak: `key()` is called once per artifact per field, and a leak there
    /// grows without bound over a long run.
    pub fn key(&self) -> std::borrow::Cow<'_, str> {
        use std::borrow::Cow;
        match self {
            Self::Flag(f) => Cow::Borrowed(f.default_key()),
            Self::Renamed { published_as, .. } => Cow::Borrowed(published_as.as_str()),
            Self::Var {
                var,
                published_as: None,
                ..
            } => Cow::Borrowed(var.as_str()),
            Self::Var {
                published_as: Some(k),
                ..
            }
            | Self::Plane {
                published_as: Some(k),
                ..
            } => Cow::Borrowed(k.as_str()),
            // `black` → `black_ink_bytes`; `of: screen` → `screen_black_ink_bytes`.
            Self::Plane {
                plane,
                of,
                published_as: None,
            } => match of {
                DisplayPlaneOf::Ram => Cow::Owned(format!("{plane}_ink_bytes")),
                DisplayPlaneOf::Screen => Cow::Owned(format!("screen_{plane}_ink_bytes")),
            },
        }
    }
}

/// A fact about a painted panel that the artifact's `meta` can carry.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMetaFlag {
    /// Frame-memory bytes carrying at least one lit pixel (1 bpp panels).
    InkBytes,
    /// Lit pixels across a 1 bpp frame memory.
    LitPixels,
    /// Artifact bytes that are not 0x00. THE definition the CLI's
    /// `painted bytes=` line prints.
    PaintedBytes,
    /// Artifact payload length.
    TotalBytes,
    /// The most common non-black pixel, as `0xRRRR` (RGB565 panels).
    TopColour,
    /// How many pixels carry [`Self::TopColour`].
    TopColourPixels,
    /// DISPON, and a supply to hold it.
    DisplayOn,
    /// SLPOUT seen, and a supply.
    Awake,
    /// DISPON **and** awake — what a camera would see.
    Lit,
    /// The module's supply pins are connected in the design.
    Powered,
    /// Inversion flag. Recorded, never applied to the stored bytes.
    Inverted,
    /// The COMPLEMENT of `awake`, ungated by the supply — the name the RM67162
    /// artifact has always published. Not a rename of `awake`: `awake` is
    /// supply-gated (`powered && awake`) and this is the raw sleep flag, so an
    /// unpowered panel reports `asleep: true` rather than `awake: false`, and
    /// the two would disagree for a panel that had been woken and then lost its
    /// rail.
    Asleep,
    /// Which of the two real D/C wirings this placement uses: `"gpio"` when
    /// firmware toggles a pin, `"controller_dcx"` when the SPI controller
    /// drives the line itself. A string, because it is a choice and not a flag.
    DcSource,
    /// How many times a `refresh` action has put frame memory on the glass.
    /// The only thing that tells a written frame from a shown one, and what
    /// `labwired_verify`'s `min_refresh_generation` clause resolves against.
    RefreshGeneration,
    /// Bytes per plane — the SPLIT of a multi-plane artifact payload. Without
    /// it a consumer cannot find where the red plane starts.
    PlaneBytes,
}

impl DisplayMetaFlag {
    pub fn default_key(self) -> &'static str {
        match self {
            Self::InkBytes => "ink_bytes",
            Self::LitPixels => "lit_pixels",
            Self::PaintedBytes => "painted_bytes",
            Self::TotalBytes => "total_bytes",
            Self::TopColour => "top_colour",
            Self::TopColourPixels => "top_colour_pixels",
            Self::DisplayOn => "display_on",
            Self::Awake => "awake",
            Self::Lit => "lit",
            Self::Powered => "powered",
            Self::Inverted => "inverted",
            Self::Asleep => "asleep",
            Self::DcSource => "dc_source",
            Self::RefreshGeneration => "refresh_generation",
            Self::PlaneBytes => "plane_bytes",
        }
    }
}

// ─── the `led_strip` primitive ──────────────────────────────────────────────
//
// An addressable LED strip is a PER-LED COLOUR ARRAY clocked by a wire
// protocol. It is not a framebuffer panel: there is no address counter, no
// command table, no window, and no frame memory that a later command re-reads.
// Writing it as a `display` would have meant inventing all four.
//
// What is data: the strip's colour order on the wire, how many LEDs, which
// protocol clocks them in, and — for the single-wire parts — the bit-timing
// table the datasheet states in nanoseconds. What is engine: the two decoders,
// and the artifact.
//
// WHAT IS DELIBERATELY NOT HERE, on both wires: power draw and daisy-chain
// propagation delay. Neither is observable in the colour array, which is the
// only thing this primitive claims to reproduce.

/// The `behavior.led_strip` section: one addressable strip, entirely as data.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LedStripSpec {
    /// How bytes reach the strip.
    pub wire: LedStripWire,
    /// `crate::inspect::artifact_format` name the artifact carries, so a
    /// consumer decoding the bytes reads the same string it always did. It is
    /// also what states the payload's BYTE ORDER (`APA102_RGB` vs
    /// `ws2812_grb`) — there is no second key repeating that fact, because two
    /// keys for one fact is how a descriptor starts lying.
    pub artifact_format: String,
    /// Default strip length when the placement states no `num_pixels`.
    pub default_pixels: u32,
    /// Whether this strip's supply connection is modelled: the engine exposes a
    /// `powered` config key, refuses the wire when it is explicitly `false`,
    /// and reports `powered` in the artifact.
    ///
    /// TRUE for the APA102, which is the clearest case in the tree: the LEDs
    /// draw every milliamp from the rail and none from the data lines, so a
    /// diagram wiring only CLK/DATA/CS is completely dark on a bench.
    #[serde(default)]
    pub supply_gated: bool,
    /// Which facts the artifact's `meta` carries. `w`, `h`, `format` and
    /// `generation` are always present because they describe the payload;
    /// everything else is listed here, for the same reason a display's is.
    pub artifact_meta: Vec<LedStripMetaField>,
    /// `nrz_gpio` only: the bit-timing table, in nanoseconds.
    #[serde(default)]
    pub timing: Option<LedStripTiming>,
    /// `spi_frames` only: the framing of one SPI transaction.
    #[serde(default)]
    pub spi_frames: Option<LedStripSpiFrames>,
}

/// Which wire protocol clocks a strip's colours in.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LedStripWire {
    /// Clocked SPI: a start frame, one fixed-size frame per LED, an end frame.
    /// The strip latches when CS is released (APA102 / DotStar).
    SpiFrames,
    /// ONE self-clocked data wire carrying an NRZ stream, decoded from GPIO
    /// EDGE TIMING: every bit is a HIGH pulse whose DURATION is the bit value.
    /// A long LOW gap latches the frame (WS2812 / WS2812B / SK6812).
    NrzGpio,
}

/// The single-wire bit timing, IN NANOSECONDS, as the datasheet states it.
///
/// Stated rather than hard-coded because it is the part's own number: a SK6812
/// and a WS2812B share this decoder and differ here. The engine scales these to
/// simulated cycles with the firmware's clock, so the decode tracks the same
/// time base the edges were scheduled on.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct LedStripTiming {
    /// HIGH duration, in ns, separating a `0` (short high) from a `1` (long
    /// high). The mid-point between T0H and T1H.
    pub high_threshold_ns: u64,
    /// LOW-gap duration, in ns, that ends a frame and displays it. The
    /// datasheet minimum is the reset time; a detector below it must still be
    /// far above any inter-bit low.
    pub reset_threshold_ns: u64,
    /// Bits per LED. 24 for an RGB part.
    pub bits_per_pixel: u32,
}

/// The framing of one clocked-SPI transaction.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct LedStripSpiFrames {
    /// Bytes that must open the transaction, matched exactly. A transaction
    /// that does not start with them latches NOTHING — a glitchy transfer must
    /// not blank a strip.
    pub start_frame: Vec<u8>,
    /// Size of one LED frame, in bytes.
    pub frame_bytes: u32,
    /// Mask applied to an LED frame's first byte, and the value that mask must
    /// equal for the frame to be an LED frame. Anything else is the end frame
    /// or garbage and STOPS the decode.
    pub header_mask: u8,
    pub header_value: u8,
    /// Mask selecting the global-brightness field out of that same first byte.
    /// Absent ⇒ this strip has no per-LED brightness field.
    #[serde(default)]
    pub brightness_mask: Option<u8>,
    /// Index, within one LED frame, of each colour byte in `colour_order`.
    pub colour_bytes: Vec<u8>,
}

/// A fact about a clocked strip that the artifact's `meta` can carry.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LedStripMetaFlag {
    /// Per-LED global brightness, as an array. `spi_frames` only.
    Brightness,
    /// How many LEDs the decoder actually reconstructed.
    PixelsDecoded,
    /// How many of them carry a non-zero colour.
    LitPixels,
    /// The strip's supply pins are connected in the design.
    Powered,
    /// The chip-select pad label. `spi_frames` only.
    CsPin,
    /// The data pad number. `nrz_gpio` only.
    DataPin,
}

impl LedStripMetaFlag {
    pub fn default_key(self) -> &'static str {
        match self {
            Self::Brightness => "brightness",
            Self::PixelsDecoded => "pixels_decoded",
            Self::LitPixels => "lit_pixels",
            Self::Powered => "powered",
            Self::CsPin => "cs_pin",
            Self::DataPin => "data_pin",
        }
    }
}

/// One entry of [`LedStripSpec::artifact_meta`], optionally renamed. Same shape
/// and the same reason as [`DisplayMetaField`].
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(untagged)]
pub enum LedStripMetaField {
    Flag(LedStripMetaFlag),
    Renamed {
        flag: LedStripMetaFlag,
        #[serde(rename = "as")]
        published_as: String,
    },
}

impl LedStripMetaField {
    pub fn flag(&self) -> LedStripMetaFlag {
        match self {
            Self::Flag(f) => *f,
            Self::Renamed { flag, .. } => *flag,
        }
    }

    pub fn key(&self) -> &str {
        match self {
            Self::Flag(f) => f.default_key(),
            Self::Renamed { published_as, .. } => published_as,
        }
    }
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
    /// The value an ERASED frame-memory byte holds, and therefore the value an
    /// ink count treats as "no ink".
    ///
    /// STATED, because the two families disagree and the disagreement is the
    /// whole picture. An OLED's GDDRAM powers on at `0x00` and a set bit is a
    /// lit pixel. A tri-colour e-paper powers on at `0xFF` and a set bit is NO
    /// ink — the planes are erased white — so `black_ink_bytes` counts bytes
    /// that are not `0xFF`. A house default of 0 would report a blank e-paper
    /// as fully inked and a cleared one as blank, which is backwards on both
    /// counts. It is also what `clear_ram` fills with.
    #[serde(default)]
    pub blank: u8,
    /// Named 1-bpp PLANES the frame memory is divided into, in payload order.
    ///
    /// Empty — the default — is one undivided frame memory, which is every
    /// panel but the tri-colour e-papers. Those hold TWO independent 1-bpp
    /// RAMs selected by the command that opens the stream (SSD1680 0x24 black /
    /// 0x26 red; UC8151D DTM1 0x10 / DTM2 0x13), not one deeper pixel format:
    /// a write to one plane leaves the other alone, and the artifact payload is
    /// the planes concatenated in this order with `plane_bytes` giving the
    /// split.
    #[serde(default)]
    pub planes: Vec<String>,
    /// What one step of each address counter MEANS.
    ///
    /// EXPLICIT AND PER AXIS, because the SSD1680 mixes them in one window:
    /// 0x44 (RAM-X window) takes the X bounds in BYTES — the datasheet's
    /// "start/8" — while 0x45 (RAM-Y window) takes Y in pixels. A model that
    /// read both in pixels put every row of a partial window in the wrong place
    /// and still streamed a plausible byte count. Never inferred from the pixel
    /// format: a 1-bpp panel may address either way, and the SSD1306 addresses
    /// columns in pixels.
    #[serde(default)]
    pub units: DisplayRamUnits,
    /// Whether a data byte is frame memory unconditionally, or only after a
    /// `ram_write` command has opened the stream.
    ///
    /// STATED RATHER THAN DERIVED FROM THE FRAMING. It is tempting to say
    /// "I²C control byte ⇒ always, D/C pad ⇒ command", because that is what the
    /// SSD1306 and the ST7789 do. The PCD8544 is a D/C-pad panel with NO RAMWR
    /// opcode at all — every D/C-high byte is DDRAM — so deriving the rule from
    /// the framing would have dropped every pixel that panel was ever sent, on
    /// a code path with no error to read.
    pub stream: DisplayRamStream,
}

/// See [`DisplayRam::stream`].
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DisplayRamStream {
    /// Every data byte is frame memory. There is no RAMWR opcode.
    Always,
    /// A `ram_write` action opens the stream; data bytes outside it are
    /// command parameters or strays. The stream stays open until the next
    /// command byte closes it.
    Command,
    /// A `ram_write` action opens the stream and the WINDOW BOUNDS IT: the
    /// controller accepts exactly `(col_end - col_start + 1) * (row_end -
    /// row_start + 1)` write units and then the stream is closed, whatever
    /// arrives next.
    ///
    /// The SSD1680's own behaviour, and not a tidier spelling of `command`.
    /// GxEPD2 configures the RAM window (0x44/0x45) and the counters
    /// (0x4E/0x4F) before every 0x24/0x26, so the byte count is a fact the
    /// controller knows; a stream that ran past it would wrap the counters back
    /// to the window start and overwrite the rows it had just written. It is
    /// also what lets a panel with no D/C line wired tell the byte AFTER a full
    /// plane from a pixel — see `DisplayDc::unwired`.
    WindowCounted,
}

/// See [`DisplayRam::units`]. One entry per addressed axis.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
pub struct DisplayRamUnits {
    #[serde(default)]
    pub col: DisplayUnit,
    #[serde(default)]
    pub row: DisplayUnit,
}

/// What one step of an address counter covers.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DisplayUnit {
    /// One pixel. Every panel but the e-papers, on both axes.
    #[default]
    Pixels,
    /// One BYTE — eight pixels of a 1-bpp row. The SSD1680's X axis.
    Bytes,
}

impl DisplayUnit {
    /// Pixels per counter step.
    pub fn pixels_per_step(self) -> u16 {
        match self {
            Self::Pixels => 1,
            Self::Bytes => 8,
        }
    }
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
    /// What framing this panel falls back to when NO D/C line is resolved at
    /// attach. A declared CHEAT, per panel, never a house rule.
    ///
    /// A board that wires no D/C pad is a real board — the ESP32 e-paper lab is
    /// one — and the two e-paper models ported here answered it DIFFERENTLY,
    /// which is why it is data. The SSD1680 model inferred: a byte arriving
    /// with no stream open is a command, anything else is a parameter or a
    /// pixel, and `window_counted` is what makes that inference terminate. The
    /// UC8151D model could not infer (its plane streams end at the next command
    /// byte, so an inferring decoder can never leave one) and treated every
    /// byte as DATA. Both are cheats; both are what the deleted models did; a
    /// default would have silently changed one panel's picture.
    #[serde(default)]
    pub unwired: DisplayDcUnwired,
}

/// See [`DisplayDc::unwired`].
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DisplayDcUnwired {
    /// Read the latched level anyway — what every panel did before this key
    /// existed, and what a panel whose D/C pad is REQUIRED keeps doing.
    #[default]
    Level,
    /// No stream open ⇒ command, otherwise parameter/pixel. A declared cheat;
    /// the marker and its `real:` clause sit on the engine's decode, which is
    /// the one place it actually happens.
    Infer,
    /// Every byte is data. Same, and the same marker.
    Data,
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
    /// A D/C line that the SPI **controller** may drive itself — the nRF54L
    /// SPIM's `PSEL.DCX` + `DCXCNT`, which holds D/C low for the first DCXCNT
    /// bytes of a transfer and high for the rest, with no firmware pin write
    /// anywhere.
    ///
    /// The byte-level framing is identical to [`Self::Pin`]: the device latches
    /// a level and reads it before each transfer. What differs is ATTACH. A
    /// `pin` panel demands `dc_pin` and resolves it to a GPIO output register;
    /// an `hw_dcx` panel accepts EITHER `dc_pin` (an nRF52-era or STM32 board,
    /// where firmware toggles the line) OR `hw_dcx: true` (the controller
    /// drives it), and requires exactly one of them. Neither is inference: both
    /// are wires, and which one is connected is a fact about the board.
    HwDcx,
}

/// Which addressing modes the controller implements and which one it powers on
/// in.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DisplayAddressing {
    #[serde(default)]
    pub modes: Vec<DisplayAddressingMode>,
    #[serde(default)]
    pub default: DisplayAddressingMode,
    /// What the column counter does at the last column in `page` addressing.
    /// The two paged OLEDs here disagree and the difference is a whole row of
    /// pixels: the SSD1306 model holds the counter at the last column, the
    /// SH1107 wraps it back to zero.
    #[serde(default)]
    pub page_wrap: DisplayPageWrap,
}

/// See [`DisplayAddressing::page_wrap`].
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DisplayPageWrap {
    /// Hold at the last column of the frame memory.
    #[default]
    Clamp,
    /// Return to column 0.
    Wrap,
}

impl Default for DisplayAddressing {
    fn default() -> Self {
        Self {
            modes: vec![DisplayAddressingMode::Horizontal],
            default: DisplayAddressingMode::Horizontal,
            page_wrap: DisplayPageWrap::Clamp,
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
    /// Guard: this entry decodes the opcode only while a var holds a
    /// particular value.
    ///
    /// An INSTRUCTION-SET BANK, which is a real thing on the older LCD
    /// controllers: the PCD8544's function-set H bit decides whether `0x80|n`
    /// means "set X address" or "set Vop", and nothing about the byte says
    /// which. Without a guard the two readings share one entry in a flat
    /// 256-way table and one of them silently wins.
    #[serde(default)]
    pub when: Option<DisplayWhen>,
}

/// See [`DisplayCommand::when`].
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct DisplayWhen {
    /// Name of the var (see [`DisplaySpec::vars`]) the guard reads.
    pub var: String,
    /// Applied to the var before the comparison. Absent ⇒ the whole value.
    #[serde(default)]
    pub mask: Option<u32>,
    /// The masked value this entry requires.
    pub equals: u32,
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
    /// Blank frame memory. A SEPARATE action from `reset_control` because the
    /// two are separate facts: a MIPI SWRESET resets control state and keeps
    /// the picture (ST7789V §9.1.22 p.202, ILI9341 §8.2.2), while a hardware
    /// RST line clears both. A controller that does clear declares both
    /// actions; nothing is implied.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub clear_ram: bool,
    /// Put the frame memory ON THE GLASS and bump the refresh generation.
    ///
    /// E-paper is the only family here where writing frame memory does not
    /// change what a camera sees. SSD1680 0x20 (master activation) and UC8151D
    /// 0x12 (DRF) are what move the ink; until one arrives the panel still
    /// shows the previous image, and `labwired_verify`'s
    /// `min_refresh_generation` clause is the only thing that can tell "RAM was
    /// written" from "the picture changed".
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub refresh: bool,
    /// Guard: run this entry only when a PARAMETER BYTE of the command holds a
    /// particular value. Not an action — an action still has to be set.
    ///
    /// Forced by the SSD1680's 0x22, which is a SEQUENCE SELECTOR: the same
    /// opcode with parameter 0xF8 powers the booster on and with 0x83 powers it
    /// off, and GxEPD2 sends both. One entry per arm, each guarded, is the
    /// datasheet's own shape. Without it a command whose meaning is in its
    /// parameter needs a Rust arm, which is the thing this primitive exists to
    /// delete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<DisplayArgWhen>,
}

/// See [`DisplayAction::when`].
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct DisplayArgWhen {
    /// Index of the parameter byte this guard reads.
    pub arg: u8,
    /// Applied to the byte before the comparison. Absent ⇒ the whole byte.
    #[serde(default)]
    pub mask: Option<u8>,
    /// The masked value this entry requires.
    pub equals: u8,
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
            + self.clear_ram as usize
            + self.refresh as usize
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
    /// Which of [`DisplayRam::planes`] this stream writes. Required when the
    /// frame memory HAS planes, refused when it does not — the plane is the
    /// only thing that tells the SSD1680's 0x24 from its 0x26, and a stream
    /// that defaulted to the first plane would paint the red image in black.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plane: Option<String>,
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
