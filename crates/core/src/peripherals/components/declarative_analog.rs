// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Declarative `analog_source` primitive: a part whose whole interface is one
//! analogue voltage, described in `configs/devices/*.yaml` as a datasheet
//! output CURVE or as a datasheet EQUATION. The Sharp GP2Y0A21 IR ranger
//! (`gp2y0a21.yaml`) is the proof part for the curve; the NTC thermistor
//! (`ntc_thermistor.yaml`, a beta equation behind a divider) is the proof part
//! for the formula.
//!
//! Why a primitive and not N hand-written kits: the analog modules in this
//! tree were the same ~150 lines of Rust differing only in a table or a line
//! of algebra. Both are datasheet language, so both are what the descriptor
//! carries — the engine owns everything else (SimInput plumbing, mV→ADC-count,
//! kit metadata, attach), exactly the separation the `i2c_device` primitive
//! established.
//!
//! **Curve or formula, never both.** A datasheet that publishes a graph gets a
//! `curve:`; one that publishes an equation gets a `formula:`. Sampling an
//! equation into a table is lossy exactly where the equation is steep — the
//! CdS power law needs sub-0.001 lx spacing near darkness to stay inside one
//! ADC LSB — and a table that coarse would be a worse copy of a line anyone
//! can check against the datasheet.
//!
//! **More than one stimulus channel.** A part may declare several: the LiPo
//! charger's pin voltage is a function of state-of-charge AND whether the
//! charger is plugged in. With more than one channel the descriptor must say
//! which value reaches the pin (`formula:`, or `source:` for a curve) — there
//! is no "the" input to infer, and inferring one would silently ignore the
//! rest.

use std::any::Any;
use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use labwired_config::{AnalogAboveLast, AnalogEncode, AnalogSpec, DeviceDescriptor};

use super::declarative_expr::{compile_derived, compile_formula, eval_derived, CompiledExpr};

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, LabRef, PeripheralKit, Transport,
};
use crate::sim_input::{InputChannel, SimInput};

/// The generic analog device. Constructed from a [`DeviceDescriptor`] whose
/// `behavior.analog` holds the curve or the formula; driven through
/// [`SimInput`] on the descriptor's input channels; read by the ADC model
/// through [`crate::bus::sim_inputs::AnalogSource`].
#[derive(Debug, serde::Serialize)]
pub struct DeclarativeAnalogDevice {
    /// ADC channel this part's output pin is wired to.
    channel: u8,
    /// The datasheet curve, ascending in input. Empty when the part states a
    /// formula instead.
    curve: Vec<(f32, f32)>,
    below_clamp_mv: f32,
    above: AnalogAboveLast,
    encode: AnalogEncode,
    /// Which name the curve is indexed by. `None` for a formula part.
    curve_source: Option<String>,
    /// Current value of every input channel (engineering units), keyed by
    /// channel key.
    input_values: HashMap<String, f64>,
    /// The input channels this primitive drives (from `metadata.inputs`),
    /// owned by this device so it remains independent of the originating kit.
    inputs: std::borrow::Cow<'static, [InputChannel]>,
    /// `behavior.derived`, compiled once at load.
    #[serde(skip)]
    derived: Vec<CompiledExpr>,
    /// `behavior.analog.formula`, compiled once at load. `None` for a curve.
    #[serde(skip)]
    formula: Option<CompiledExpr>,
    v_ref_mv: f32,
    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl DeclarativeAnalogDevice {
    fn from_descriptor(
        descriptor: &DeviceDescriptor,
        channel: u8,
        inputs: std::borrow::Cow<'static, [InputChannel]>,
    ) -> Result<Self> {
        let spec = descriptor
            .behavior
            .analog
            .as_ref()
            .context("analog_source kit is missing behavior.analog")?;
        validate_spec(spec, inputs.len())?;

        let declared = descriptor.metadata.as_ref().map(|m| m.inputs.as_slice());
        let mut input_values = HashMap::new();
        for input in inputs.iter() {
            let default = declared
                .and_then(|list| list.iter().find(|candidate| candidate.key == input.key))
                .and_then(|candidate| candidate.default)
                .unwrap_or(0.0);
            input_values.insert(input.key.to_string(), default);
        }

        let input_keys: Vec<String> = inputs.iter().map(|i| i.key.to_string()).collect();
        let derived = compile_derived(&descriptor.behavior.derived, &input_keys)?;
        let mut known = input_keys;
        known.extend(derived.iter().map(|d| d.name.clone()));

        let formula = match &spec.formula {
            Some(src) => Some(compile_formula("analog.formula", src, &known)?),
            None => None,
        };
        let curve_source = match (&spec.formula, &spec.source) {
            (Some(_), _) => None,
            (None, Some(name)) => {
                if !known.iter().any(|k| k == name) {
                    bail!(
                        "analog.source names '{name}', which is neither a declared input channel \
                         nor a derived channel. Known names: {known:?}"
                    );
                }
                Some(name.clone())
            }
            // One channel and no `source:` — the channel IS the source.
            (None, None) => Some(inputs[0].key.to_string()),
        };

        let below_clamp_mv = spec.curve.first().map(|p| p.1).unwrap_or(0.0);
        Ok(Self {
            channel,
            curve: spec.curve.clone(),
            below_clamp_mv,
            above: spec.above_last,
            encode: spec.encode,
            curve_source,
            input_values,
            inputs,
            derived,
            formula,
            v_ref_mv: 3300.0,
            component_id: None,
        })
    }

    pub fn channel(&self) -> u8 {
        self.channel
    }

    /// Every channel value the part can read this instant: the driven stimulus
    /// slots plus the `derived:` names computed from them, in declaration
    /// order.
    fn slots(&self) -> HashMap<String, f64> {
        let mut slots = self.input_values.clone();
        eval_derived(&self.derived, &mut slots);
        slots
    }

    /// Output voltage in mV for the current inputs: the descriptor's formula,
    /// or piecewise-linear over its curve with the stated out-of-band rules.
    pub fn output_mv(&self) -> u16 {
        let slots = self.slots();
        let mv = match &self.formula {
            Some(f) => f.eval_with(&slots),
            None => self.curve_mv(&slots),
        };
        self.quantise(mv)
    }

    /// The real millivolt value the curve gives for the current source value.
    ///
    /// In `f64`, and multiplying before dividing, because the descriptor's job
    /// is to be the LINE the datasheet drew. The hand-written models this
    /// primitive replaces evaluated the same lines in `f32` and in whatever
    /// order read well (`Vref * (1 - m/100)` rather than `Vref - Vref*m/100`),
    /// which cost the last bit at points where the exact answer is a whole
    /// millivolt: the soil probe's 660.0 mV at 80 % arrived as 659.99996 and
    /// truncated to 659. Reproducing that would mean copying an artefact of
    /// float ordering into data anyone can check by hand.
    fn curve_mv(&self, slots: &HashMap<String, f64>) -> f64 {
        // `curve_source` is Some for every curve part (checked at load).
        let d = self
            .curve_source
            .as_ref()
            .and_then(|key| slots.get(key))
            .copied()
            .unwrap_or(0.0);
        let (first_d, first_v) = (f64::from(self.curve[0].0), f64::from(self.curve[0].1));
        if d <= first_d {
            return first_v;
        }
        let last = self.curve[self.curve.len() - 1];
        let (last_d, last_v) = (f64::from(last.0), f64::from(last.1));
        if d >= last_d {
            return match self.above.floor_mv {
                Some(floor) if d > last_d => f64::from(floor),
                _ => last_v,
            };
        }
        for w in self.curve.windows(2) {
            let (d0, v0) = (f64::from(w[0].0), f64::from(w[0].1));
            let (d1, v1) = (f64::from(w[1].0), f64::from(w[1].1));
            if d <= d1 {
                return v0 + (d - d0) * (v1 - v0) / (d1 - d0);
            }
        }
        // Unreachable given the bounds above, but never invent a voltage.
        f64::from(self.below_clamp_mv)
    }

    /// Make the real millivolt value the integer the pin reports, the way the
    /// descriptor says. Clamped to the rail either way: a pin cannot be below
    /// ground or above Vref whatever the algebra produced.
    fn quantise(&self, mv: f64) -> u16 {
        let clamped = if mv.is_nan() {
            0.0
        } else {
            mv.clamp(0.0, f64::from(self.v_ref_mv))
        };
        match self.encode {
            AnalogEncode::Trunc => clamped as u16,
            AnalogEncode::Round => clamped.round() as u16,
        }
    }

    /// Convert `output_mv` to a 12-bit ADC count (0..4095) for 3.3 V Vref.
    pub fn adc_count(&self) -> u16 {
        let mv = self.output_mv() as u32;
        ((mv * 4095) / 3300).min(4095) as u16
    }

    pub fn as_any(&self) -> &dyn Any {
        self
    }
    pub fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Datasheet sanity rules for `behavior.analog`: exactly one of `curve:` /
/// `formula:`; a curve of at least two points, strictly ascending in input,
/// with voltages inside the 3.3 V rail; and a `source:` whenever the part has
/// more than one stimulus channel to choose between.
fn validate_spec(spec: &AnalogSpec, input_count: usize) -> Result<()> {
    match (spec.curve.is_empty(), spec.formula.is_some()) {
        (true, false) => bail!(
            "behavior.analog states neither `curve:` nor `formula:` — an analog_source with no \
             output rule reports 0 mV forever, which is a part that is not modelled rather than \
             a part with a flat output"
        ),
        (false, true) => bail!(
            "behavior.analog states BOTH `curve:` and `formula:`. They are two answers for the \
             same pin; delete the one that is not the datasheet's."
        ),
        _ => {}
    }
    if spec.formula.is_some() {
        if spec.source.is_some() {
            bail!(
                "behavior.analog states `source:` alongside `formula:` — a formula names the \
                 channels it reads itself, so `source:` would select nothing"
            );
        }
        if spec.above_last.floor_mv.is_some() {
            bail!(
                "behavior.analog states `above_last` alongside `formula:` — out-of-band rules \
                 are CURVE rules; write the bound into the expression (min/max) where a reader \
                 can see it"
            );
        }
        return Ok(());
    }
    if spec.curve.len() < 2 {
        bail!(
            "analog curve needs at least two points, got {}",
            spec.curve.len()
        );
    }
    for w in spec.curve.windows(2) {
        if w[1].0 <= w[0].0 {
            bail!(
                "analog curve inputs must be strictly ascending: {:?} then {:?}",
                w[0],
                w[1]
            );
        }
    }
    for &(d, v) in &spec.curve {
        if !(0.0..=3300.0).contains(&v) {
            bail!("analog curve point ({d}, {v}) has a voltage outside 0..3300 mV");
        }
    }
    if let Some(floor) = spec.above_last.floor_mv {
        if !(0.0..=3300.0).contains(&floor) {
            bail!("analog above_last floor {floor} is outside 0..3300 mV");
        }
    }
    if input_count > 1 && spec.source.is_none() {
        bail!(
            "analog_source declares {input_count} input channels and a `curve:` with no \
             `source:` — with more than one channel there is no 'the' input, and inferring one \
             would silently ignore the rest"
        );
    }
    Ok(())
}

impl SimInput for DeclarativeAnalogDevice {
    fn input_channels(&self) -> &[InputChannel] {
        &self.inputs
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        // `require_channel` already rejected any key the descriptor does not
        // declare, so the slot exists.
        self.input_values.insert(key.to_string(), value);
        Ok(())
    }

    fn component_id(&self) -> Option<&str> {
        self.component_id.as_deref()
    }

    fn set_component_id(&mut self, id: String) {
        self.component_id = Some(id);
    }
}

impl crate::bus::sim_inputs::AnalogSource for DeclarativeAnalogDevice {
    fn output_mv(&self) -> u16 {
        self.output_mv()
    }
}

// ─── PeripheralKit registration ────────────────────────────────────────────

/// A [`PeripheralKit`] backed by a declarative `analog_source` descriptor —
/// one instance per YAML device. The kit owns its metadata and each attached
/// model owns its channels, so temporary runtime kits can be dropped.
pub struct DeclarativeAnalogKit {
    descriptor: DeviceDescriptor,
    channels: std::borrow::Cow<'static, [InputChannel]>,
    metadata: KitMetadata,
}

impl DeclarativeAnalogKit {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let descriptor = DeviceDescriptor::from_yaml(yaml)?;
        validate_descriptor(&descriptor)?;

        let channels = owned_channels(&descriptor);
        let metadata = owned_metadata(&descriptor, channels.clone());
        Ok(Self {
            descriptor,
            channels,
            metadata,
        })
    }

    /// Build the device this kit attaches, without a bus.
    ///
    /// The migration-parity tests drive the descriptor directly — they compare
    /// `output_mv()` against the arithmetic a deleted Rust model performed, and
    /// standing up an `AttachCtx` would put a bus between the sweep and the
    /// thing under test. Channel seeding still comes from the descriptor's own
    /// `metadata.inputs[].default`, which is the only thing `attach` adds.
    pub fn build(&self, channel: u8) -> Result<DeclarativeAnalogDevice> {
        DeclarativeAnalogDevice::from_descriptor(&self.descriptor, channel, self.channels.clone())
    }
}

/// Validate the static descriptor contract for the `analog_source` primitive
/// without constructing a long-lived kit. Part-pack preflight calls this for
/// every carried pack, including leaves that no current canvas references.
pub(crate) fn validate_descriptor(descriptor: &DeviceDescriptor) -> Result<()> {
    if descriptor.behavior.primitive != "analog_source" {
        bail!(
            "declarative analog kit requires behavior.primitive: analog_source, got '{}'",
            descriptor.behavior.primitive
        );
    }
    let spec = descriptor
        .behavior
        .analog
        .as_ref()
        .context("analog_source kit is missing behavior.analog")?;
    let input_count = descriptor
        .metadata
        .as_ref()
        .map(|metadata| metadata.inputs.len())
        .unwrap_or(0);
    if input_count == 0 {
        bail!(
            "analog_source drives at least one input channel, got none — a part nothing can \
             drive reports its default forever"
        );
    }
    validate_spec(spec, input_count)?;
    // Compile the expressions here too, so a descriptor whose formula names a
    // channel it does not declare fails at LOAD with the name in the message
    // rather than reading 0 mV on the bus. Part-pack preflight calls this for
    // every carried pack, including leaves no current canvas references.
    let input_keys: Vec<String> = descriptor
        .metadata
        .as_ref()
        .map(|m| m.inputs.iter().map(|i| i.key.clone()).collect())
        .unwrap_or_default();
    let derived = compile_derived(&descriptor.behavior.derived, &input_keys)?;
    let mut known = input_keys;
    known.extend(derived.iter().map(|d| d.name.clone()));
    if let Some(src) = &spec.formula {
        compile_formula("analog.formula", src, &known)?;
    }
    if let Some(name) = &spec.source {
        if !known.iter().any(|k| k == name) {
            bail!(
                "analog.source names '{name}', which is neither a declared input channel nor a \
                 derived channel. Known names: {known:?}"
            );
        }
    }
    Ok(())
}

/// Copy the descriptor's `metadata.inputs` into an owned channel table — the
/// same derivation the I²C primitive's `owned_channels` does, kept local so
/// this module owns its whole contract.
fn owned_channels(descriptor: &DeviceDescriptor) -> std::borrow::Cow<'static, [InputChannel]> {
    let inputs = descriptor
        .metadata
        .as_ref()
        .map(|m| m.inputs.as_slice())
        .unwrap_or(&[]);
    let channels: Vec<InputChannel> = inputs
        .iter()
        .map(|i| InputChannel {
            key: std::borrow::Cow::Owned(i.key.clone()),
            label: std::borrow::Cow::Owned(i.label.clone()),
            unit: std::borrow::Cow::Owned(i.unit.clone()),
            min: i.min,
            max: i.max,
        })
        .collect();
    std::borrow::Cow::Owned(channels)
}

/// Map a descriptor's `config_keys[].ty` string onto a [`ConfigType`].
fn config_type_from_str(ty: &str) -> ConfigType {
    match ty {
        "int" => ConfigType::Int,
        "float" => ConfigType::Float,
        "bool" => ConfigType::Bool,
        _ => ConfigType::Str,
    }
}

/// Derive owned kit metadata from the descriptor's display metadata.
fn owned_metadata(
    descriptor: &DeviceDescriptor,
    channels: std::borrow::Cow<'static, [InputChannel]>,
) -> KitMetadata {
    let meta = descriptor.metadata.as_ref();
    let owned_text = |s: String| -> std::borrow::Cow<'static, str> { std::borrow::Cow::Owned(s) };
    let label = meta
        .and_then(|m| m.label.clone())
        .unwrap_or_else(|| descriptor.r#type.clone());
    let summary = meta
        .and_then(|m| m.summary.clone())
        .unwrap_or_else(|| "Declarative analog source.".to_string());
    let detail = meta
        .and_then(|m| m.detail.clone())
        .unwrap_or_else(|| summary.clone());

    // Config keys: an explicit `metadata.config_keys` is the COMPLETE set;
    // otherwise synthesise the lone `channel` key, matching the hand-written
    // analog kits this primitive replaces.
    let declared_keys = meta.map(|m| m.config_keys.as_slice()).unwrap_or(&[]);
    let config_keys: std::borrow::Cow<'static, [ConfigKey]> = if declared_keys.is_empty() {
        std::borrow::Cow::Owned(vec![ConfigKey {
            name: std::borrow::Cow::Borrowed("channel"),
            ty: ConfigType::Int,
            doc: std::borrow::Cow::Borrowed("ADC channel index (0..N). Defaults to 0."),
        }])
    } else {
        std::borrow::Cow::Owned(
            declared_keys
                .iter()
                .map(|k| ConfigKey {
                    name: owned_text(k.name.clone()),
                    ty: config_type_from_str(&k.ty),
                    doc: owned_text(k.doc.clone()),
                })
                .collect::<Vec<_>>(),
        )
    };

    // Labs: mirror any declared starter labs verbatim. A descriptor that
    // dropped them would take the part's one-click demo off the shelf while
    // every test still passed.
    let labs: std::borrow::Cow<'static, [LabRef]> = std::borrow::Cow::Owned(
        meta.map(|m| m.labs.as_slice())
            .unwrap_or(&[])
            .iter()
            .map(|l| LabRef {
                board_id: owned_text(l.board_id.clone()),
                chip: owned_text(l.chip.clone()),
                example_dir: owned_text(l.example_dir.clone()),
                demo_elf: owned_text(l.demo_elf.clone()),
            })
            .collect::<Vec<_>>(),
    );

    KitMetadata {
        device_type: owned_text(descriptor.r#type.clone()),
        label: owned_text(label),
        summary: owned_text(summary),
        detail: owned_text(detail),
        transport: Transport::Analog,
        category: Category::Analog,
        config_keys,
        labs,
        inputs: channels,
    }
}

impl PeripheralKit for DeclarativeAnalogKit {
    fn metadata(&self) -> &KitMetadata {
        &self.metadata
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        let channel = ctx.config_i64("channel").unwrap_or(0).clamp(0, 255) as u8;
        let mut device = DeclarativeAnalogDevice::from_descriptor(
            &self.descriptor,
            channel,
            self.channels.clone(),
        )?;
        // Honour `config:` overrides that name the input channel (e.g. a
        // `distance` seed), matching how a hand-written kit seeded its default.
        for input in self.channels.iter() {
            if let Some(v) = ctx.config_f64(input.key.as_ref()) {
                let _ = device.set_input(input.key.as_ref(), v);
            }
        }
        ctx.attach_analog_source(channel, Box::new(device))?;
        Ok(())
    }
}

// ─── Registry statics ──────────────────────────────────────────────────────
//
// A `DeclarativeAnalogKit` is parsed from YAML at runtime, but the registry
// (`registry::KITS`) is a const slice of `&'static dyn PeripheralKit`. A
// `static LazyLock<DeclarativeAnalogKit>` is the const-initialisable cell that
// bridges the two: the descriptor is parsed once on first access, and the
// `PeripheralKit` impl below forwards through it. Real parts get one static
// each here and one line in `registry::KITS`; the descriptor lives entirely in
// `configs/devices/*.yaml`.

use std::sync::LazyLock;

impl PeripheralKit for LazyLock<DeclarativeAnalogKit> {
    fn metadata(&self) -> &KitMetadata {
        LazyLock::force(self).metadata()
    }
    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        LazyLock::force(self).attach(ctx)
    }
}

/// Sharp GP2Y0A21YK0F IR distance sensor (declarative `gp2y0a21.yaml`) — the
/// proof part for the `analog_source` primitive.
pub static GP2Y0A21_KIT: LazyLock<DeclarativeAnalogKit> = LazyLock::new(|| {
    DeclarativeAnalogKit::from_yaml(
        labwired_config::embedded_device_yaml("gp2y0a21").expect("gp2y0a21 descriptor is embedded"),
    )
    .expect("gp2y0a21.yaml is a valid analog_source descriptor")
});

/// One `LazyLock` per shipped analog descriptor. Each replaces a hand-written
/// Rust model of the same name, deleted in the same change — see
/// `crates/core/tests/analog_plant_migration_parity.rs`, which sweeps every
/// one of them against the arithmetic the deleted model performed.
macro_rules! analog_kit {
    ($ident:ident, $stem:literal) => {
        pub static $ident: LazyLock<DeclarativeAnalogKit> = LazyLock::new(|| {
            DeclarativeAnalogKit::from_yaml(
                labwired_config::embedded_device_yaml($stem)
                    .expect(concat!($stem, " descriptor is embedded")),
            )
            .expect(concat!($stem, ".yaml is a valid analog_source descriptor"))
        });
    };
}

analog_kit!(LDR_KIT, "ldr");
analog_kit!(POTENTIOMETER_KIT, "potentiometer");
analog_kit!(NTC_THERMISTOR_KIT, "ntc-thermistor");
analog_kit!(MQ6_KIT, "mq-6");
analog_kit!(SOIL_MOISTURE_KIT, "soil-moisture");
analog_kit!(LIPO_CHARGER_KIT, "lipo_charger");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim_input::SimInput;

    static TEST_CHANNELS: &[InputChannel] = &[InputChannel {
        key: std::borrow::Cow::Borrowed("distance"),
        label: std::borrow::Cow::Borrowed("Distance"),
        unit: std::borrow::Cow::Borrowed("mm"),
        min: 0.0,
        max: 800.0,
    }];

    fn device() -> DeclarativeAnalogDevice {
        let descriptor =
            DeviceDescriptor::from_yaml(labwired_config::embedded_device_yaml("gp2y0a21").unwrap())
                .unwrap();
        DeclarativeAnalogDevice::from_descriptor(&descriptor, 0, TEST_CHANNELS.into()).unwrap()
    }

    fn kit_device(stem: &str) -> DeclarativeAnalogDevice {
        let kit = DeclarativeAnalogKit::from_yaml(
            labwired_config::embedded_device_yaml(stem).expect("embedded"),
        )
        .expect("valid descriptor");
        DeclarativeAnalogDevice::from_descriptor(&kit.descriptor, 0, kit.channels)
            .expect("device builds")
    }

    #[test]
    fn datasheet_table_points_are_exact() {
        for (d, v) in [
            (100.0, 3100),
            (150.0, 2300),
            (200.0, 1900),
            (300.0, 1300),
            (400.0, 1000),
            (500.0, 800),
            (600.0, 650),
            (700.0, 550),
            (800.0, 500),
        ] {
            let mut s = device();
            s.set_input("distance", d).unwrap();
            assert_eq!(s.output_mv(), v, "distance {d} mm");
        }
    }

    #[test]
    fn output_falls_monotonically_over_the_specified_band() {
        let mut prev = u16::MAX;
        for d in (100..=800).step_by(10) {
            let mut s = device();
            s.set_input("distance", d as f64).unwrap();
            let v = s.output_mv();
            assert!(v <= prev, "not monotonic at {d} mm: {v} > {prev}");
            prev = v;
        }
    }

    #[test]
    fn interpolation_lies_between_table_points() {
        let mut s = device();
        s.set_input("distance", 250.0).unwrap();
        assert_eq!(s.output_mv(), 1600);
    }

    #[test]
    fn beyond_80cm_holds_the_far_floor_not_zero() {
        let mut s = device();
        s.set_input("distance", 5000.0).unwrap_err();
        s.set_input("distance", 800.0).unwrap();
        assert_eq!(s.output_mv(), 500);
        // The channel max is 800, so the floor path is exercised via the
        // descriptor's own out-of-band rule check below.
        let mut s2 = device();
        s2.input_values.insert("distance".to_string(), 900.0);
        assert_eq!(s2.output_mv(), 400);
        assert_ne!(s2.output_mv(), 0, "far must not read as ground");
    }

    #[test]
    fn below_10cm_clamps_to_the_near_value() {
        let mut s = device();
        s.set_input("distance", 60.0).unwrap();
        assert_eq!(s.output_mv(), 3100);
        s.set_input("distance", 0.0).unwrap();
        assert_eq!(s.output_mv(), 3100);
    }

    #[test]
    fn adc_count_scales_with_voltage() {
        let mut s = device();
        s.set_input("distance", 100.0).unwrap();
        assert_eq!(s.adc_count(), (3100u32 * 4095 / 3300) as u16);
    }

    #[test]
    fn invalid_curves_are_rejected() {
        assert!(DeclarativeAnalogKit::from_yaml(
            "type: bad\nbehavior:\n  primitive: analog_source\n  analog:\n    curve:\n      - [200, 1000]\n      - [100, 2000]\nmetadata:\n  inputs:\n    - { key: x, label: X, unit: u, min: 0, max: 1 }\n"
        )
        .is_err());
    }

    /// Two answers for one pin is a descriptor bug, not a precedence rule.
    #[test]
    fn a_curve_and_a_formula_together_are_refused() {
        let err = DeclarativeAnalogKit::from_yaml(
            "type: bad\nbehavior:\n  primitive: analog_source\n  analog:\n    formula: \"x\"\n    curve:\n      - [0, 0]\n      - [1, 100]\nmetadata:\n  inputs:\n    - { key: x, label: X, unit: u, min: 0, max: 1 }\n",
        )
        .err()
        .expect("the descriptor must be refused")
        .to_string();
        assert!(err.contains("BOTH `curve:` and `formula:`"), "{err}");
    }

    #[test]
    fn neither_a_curve_nor_a_formula_is_refused() {
        let err = DeclarativeAnalogKit::from_yaml(
            "type: bad\nbehavior:\n  primitive: analog_source\n  analog: {}\nmetadata:\n  inputs:\n    - { key: x, label: X, unit: u, min: 0, max: 1 }\n",
        )
        .err()
        .expect("the descriptor must be refused")
        .to_string();
        assert!(err.contains("neither `curve:` nor `formula:`"), "{err}");
    }

    /// Two channels and a curve: which one is on the table's x-axis is a fact
    /// the descriptor must state.
    #[test]
    fn two_channels_and_a_curve_need_a_source() {
        let yaml = "type: bad\nbehavior:\n  primitive: analog_source\n  analog:\n    curve:\n      - [0, 0]\n      - [1, 100]\nmetadata:\n  inputs:\n    - { key: x, label: X, unit: u, min: 0, max: 1 }\n    - { key: y, label: Y, unit: u, min: 0, max: 1 }\n";
        let err = DeclarativeAnalogKit::from_yaml(yaml)
            .err()
            .expect("the descriptor must be refused")
            .to_string();
        assert!(err.contains("no `source:`"), "{err}");
        // …and naming one fixes it.
        let ok = yaml.replace("    curve:", "    source: y\n    curve:");
        assert!(DeclarativeAnalogKit::from_yaml(&ok).is_ok());
    }

    #[test]
    fn a_formula_naming_an_undeclared_channel_is_a_load_error() {
        let err = DeclarativeAnalogKit::from_yaml(
            "type: bad\nbehavior:\n  primitive: analog_source\n  analog:\n    formula: \"y * 2\"\nmetadata:\n  inputs:\n    - { key: x, label: X, unit: u, min: 0, max: 1 }\n",
        )
        .err()
        .expect("the descriptor must be refused")
        .to_string();
        assert!(err.contains("reads 'y'"), "{err}");
    }

    #[test]
    fn kit_metadata_comes_from_the_descriptor() {
        let kit = DeclarativeAnalogKit::from_yaml(
            labwired_config::embedded_device_yaml("gp2y0a21").unwrap(),
        )
        .unwrap();
        let m = kit.metadata();
        assert_eq!(m.device_type, "gp2y0a21");
        assert_eq!(m.label, "Sharp GP2Y0A21 IR Distance Sensor");
        assert!(matches!(m.transport, Transport::Analog));
        assert_eq!(m.inputs.len(), 1);
        assert_eq!(m.inputs[0].key, "distance");
        assert_eq!(m.config_keys[0].name, "channel");
    }

    /// Every shipped analog descriptor builds, and starts where its deleted
    /// Rust model started. A default that silently became 0 would put a pot at
    /// the ground rail and a soil probe at Vref.
    #[test]
    fn every_analog_descriptor_seeds_its_documented_default() {
        for (stem, want_mv) in [
            ("gp2y0a21", 3100u16),    // no default ⇒ 0 mm ⇒ clamped near value
            ("ldr", 2751),            // 100 lx
            ("potentiometer", 1650),  // 50 %
            ("ntc-thermistor", 1650), // 25 °C ⇒ exactly Vref/2
            ("mq-6", 0),              // 0 ppm, clean air
            ("soil-moisture", 1980),  // 40 % moisture
            ("lipo_charger", 1875),   // 50 % SoC, unplugged
        ] {
            assert_eq!(
                kit_device(stem).output_mv(),
                want_mv,
                "{stem} at its default"
            );
        }
    }

    /// The LiPo charger is the two-channel proof part: a `when:` guard on a
    /// boolean channel, a derived terminal voltage, and one `encode: trunc`
    /// standing in for the deleted model's two integer truncations.
    #[test]
    fn the_lipo_charger_reads_both_of_its_channels() {
        let mut bat = kit_device("lipo_charger");
        assert_eq!(bat.input_channels().len(), 2);
        bat.set_input("soc_pct", 100.0).unwrap();
        assert_eq!(bat.output_mv(), 2100);
        bat.set_input("usb_present", 1.0).unwrap();
        assert_eq!(bat.output_mv(), 2100, "the charge bump clamps at 4200 mV");
        bat.set_input("soc_pct", 50.0).unwrap();
        assert_eq!(bat.output_mv(), 1950, "3750 + 150 = 3900 mV, halved");
        bat.set_input("usb_present", 0.0).unwrap();
        assert_eq!(bat.output_mv(), 1875);
    }
}
