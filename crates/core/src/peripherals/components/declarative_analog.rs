// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Declarative `analog_source` primitive: a part whose whole interface is one
//! analogue voltage, described in `configs/devices/*.yaml` as a datasheet
//! output curve. The Sharp GP2Y0A21 IR ranger (`gp2y0a21.yaml`) is the proof
//! part; an MQ-x-style module is the same shape with a rising curve.
//!
//! Why a primitive and not N hand-written kits: the analog modules in this
//! tree (`mq6.rs`, `ldr.rs`, `soil_moisture.rs`) are the same ~150 lines of
//! Rust differing only in a table. The table is datasheet language, so the
//! table is what the descriptor carries — the engine owns everything else
//! (SimInput plumbing, mV→ADC-count, kit metadata, attach), exactly the
//! separation the `i2c_device` primitive established.

use std::any::Any;

use anyhow::{bail, Context, Result};
use labwired_config::{AnalogAboveLast, AnalogSpec, DeviceDescriptor};

use crate::peripherals::kit::{
    AttachCtx, Category, ConfigKey, ConfigType, KitMetadata, PeripheralKit, Transport,
};
use crate::sim_input::{InputChannel, SimInput};

/// The generic analog device. Constructed from a [`DeviceDescriptor`] whose
/// `behavior.analog` holds the curve; driven through [`SimInput`] on the
/// descriptor's single input channel; read by the ADC model through
/// [`crate::bus::sim_inputs::AnalogSource`].
#[derive(Debug, serde::Serialize)]
pub struct DeclarativeAnalogDevice {
    /// ADC channel this part's output pin is wired to.
    channel: u8,
    /// The datasheet curve, ascending in input.
    curve: Vec<(f32, f32)>,
    below_clamp_mv: f32,
    above: AnalogAboveLast,
    /// Current value of the input channel (engineering units).
    input_value: f64,
    /// The one input channel this primitive drives (from `metadata.inputs`,
    /// leaked to `'static` by the kit so `input_channels()` can hand it out.
    input: &'static InputChannel,
    v_ref_mv: f32,
    /// system.yaml `external_devices` id, stamped at attach (see
    /// [`crate::sim_input::SimInput::component_id`]).
    component_id: Option<String>,
}

impl DeclarativeAnalogDevice {
    fn from_descriptor(
        descriptor: &DeviceDescriptor,
        channel: u8,
        input: &'static InputChannel,
    ) -> Result<Self> {
        let spec = descriptor
            .behavior
            .analog
            .as_ref()
            .context("analog_source kit is missing behavior.analog")?;
        validate_spec(spec)?;
        let below_clamp_mv = spec.curve[0].1;
        let input_value = descriptor
            .metadata
            .as_ref()
            .and_then(|metadata| {
                metadata
                    .inputs
                    .iter()
                    .find(|candidate| candidate.key == input.key)
            })
            .and_then(|input| input.default)
            .unwrap_or(0.0);
        Ok(Self {
            channel,
            curve: spec.curve.clone(),
            below_clamp_mv,
            above: spec.above_last,
            input_value,
            input,
            v_ref_mv: 3300.0,
            component_id: None,
        })
    }

    pub fn channel(&self) -> u8 {
        self.channel
    }

    /// Output voltage in mV for the current input: piecewise-linear over the
    /// descriptor curve, with the descriptor's out-of-band rules.
    pub fn output_mv(&self) -> u16 {
        let d = self.input_value as f32;
        let (first_d, first_v) = self.curve[0];
        if d <= first_d {
            return first_v as u16;
        }
        let (last_d, last_v) = self.curve[self.curve.len() - 1];
        if d >= last_d {
            return match self.above.floor_mv {
                Some(floor) if d > last_d => floor as u16,
                _ => last_v as u16,
            };
        }
        for w in self.curve.windows(2) {
            let (d0, v0) = w[0];
            let (d1, v1) = w[1];
            if d <= d1 {
                let t = (d - d0) / (d1 - d0);
                return (v0 + t * (v1 - v0)) as u16;
            }
        }
        // Unreachable given the bounds above, but never invent a voltage.
        self.below_clamp_mv as u16
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

/// Datasheet sanity rules for `behavior.analog`: at least two curve points,
/// strictly ascending inputs, voltages inside the 3.3 V rail.
fn validate_spec(spec: &AnalogSpec) -> Result<()> {
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
    Ok(())
}

impl SimInput for DeclarativeAnalogDevice {
    fn input_channels(&self) -> &'static [InputChannel] {
        std::slice::from_ref(self.input)
    }

    fn set_input(&mut self, key: &str, value: f64) -> Result<(), crate::sim_input::SimInputError> {
        self.require_channel(key, value)?;
        self.input_value = value;
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
/// one instance per YAML device. `metadata()` must hand back a `&'static
/// KitMetadata`, so `from_yaml` builds it once and leaks it (the kit is itself
/// a long-lived registry entry, so the leak is bounded by the device count).
pub struct DeclarativeAnalogKit {
    descriptor: DeviceDescriptor,
    channels: &'static [InputChannel],
    metadata: &'static KitMetadata,
}

impl DeclarativeAnalogKit {
    pub fn from_yaml(yaml: &str) -> Result<Self> {
        let descriptor = DeviceDescriptor::from_yaml(yaml)?;
        validate_descriptor(&descriptor)?;

        let channels = leak_channels(&descriptor);
        let metadata = leak_metadata(&descriptor, channels);
        Ok(Self {
            descriptor,
            channels,
            metadata,
        })
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
    validate_spec(spec)?;
    let input_count = descriptor
        .metadata
        .as_ref()
        .map(|metadata| metadata.inputs.len())
        .unwrap_or(0);
    if input_count != 1 {
        bail!("analog_source drives exactly one input channel, got {input_count}");
    }
    Ok(())
}

/// Leak the descriptor's `metadata.inputs` into a static channel table — the
/// same derivation the I²C primitive's `leak_channels` does, kept local so
/// this module owns its whole contract.
fn leak_channels(descriptor: &DeviceDescriptor) -> &'static [InputChannel] {
    let inputs = descriptor
        .metadata
        .as_ref()
        .map(|m| m.inputs.as_slice())
        .unwrap_or(&[]);
    let channels: Vec<InputChannel> = inputs
        .iter()
        .map(|i| InputChannel {
            key: Box::leak(i.key.clone().into_boxed_str()),
            label: Box::leak(i.label.clone().into_boxed_str()),
            unit: Box::leak(i.unit.clone().into_boxed_str()),
            min: i.min,
            max: i.max,
        })
        .collect();
    Box::leak(channels.into_boxed_slice())
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

/// Derive a `&'static KitMetadata` from the descriptor's display metadata.
fn leak_metadata(
    descriptor: &DeviceDescriptor,
    channels: &'static [InputChannel],
) -> &'static KitMetadata {
    let meta = descriptor.metadata.as_ref();
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
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
    let config_keys: &'static [ConfigKey] = if declared_keys.is_empty() {
        Box::leak(
            vec![ConfigKey {
                name: "channel",
                ty: ConfigType::Int,
                doc: "ADC channel index (0..N). Defaults to 0.",
            }]
            .into_boxed_slice(),
        )
    } else {
        Box::leak(
            declared_keys
                .iter()
                .map(|k| ConfigKey {
                    name: leak(k.name.clone()),
                    ty: config_type_from_str(&k.ty),
                    doc: leak(k.doc.clone()),
                })
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        )
    };

    Box::leak(Box::new(KitMetadata {
        device_type: leak(descriptor.r#type.clone()),
        label: leak(label),
        summary: leak(summary),
        detail: leak(detail),
        transport: Transport::Analog,
        category: Category::Analog,
        config_keys,
        labs: &[],
        inputs: channels,
    }))
}

impl PeripheralKit for DeclarativeAnalogKit {
    fn metadata(&self) -> &'static KitMetadata {
        self.metadata
    }

    fn attach(&self, ctx: &mut AttachCtx<'_>) -> Result<()> {
        let channel = ctx.config_i64("channel").unwrap_or(0).clamp(0, 255) as u8;
        let mut device =
            DeclarativeAnalogDevice::from_descriptor(&self.descriptor, channel, &self.channels[0])?;
        // Honour `config:` overrides that name the input channel (e.g. a
        // `distance` seed), matching how a hand-written kit seeded its default.
        for input in self.channels {
            if let Some(v) = ctx.config_f64(input.key) {
                let _ = device.set_input(input.key, v);
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
    fn metadata(&self) -> &'static KitMetadata {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim_input::SimInput;

    static TEST_CHANNEL: InputChannel = InputChannel {
        key: "distance",
        label: "Distance",
        unit: "mm",
        min: 0.0,
        max: 800.0,
    };

    fn device() -> DeclarativeAnalogDevice {
        let descriptor =
            DeviceDescriptor::from_yaml(labwired_config::embedded_device_yaml("gp2y0a21").unwrap())
                .unwrap();
        DeclarativeAnalogDevice::from_descriptor(&descriptor, 0, &TEST_CHANNEL).unwrap()
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
        s2.input_value = 900.0;
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
        let bad = DeviceDescriptor::from_yaml(
            "type: bad\nbehavior:\n  primitive: analog_source\n  analog:\n    curve:\n      - [200, 1000]\n      - [100, 2000]\nmetadata:\n  inputs:\n    - { key: x, label: X, unit: u, min: 0, max: 1 }\n",
        )
        .unwrap();
        assert!(DeclarativeAnalogKit::from_yaml(
            "type: bad\nbehavior:\n  primitive: analog_source\n  analog:\n    curve:\n      - [200, 1000]\n      - [100, 2000]\nmetadata:\n  inputs:\n    - { key: x, label: X, unit: u, min: 0, max: 1 }\n"
        )
        .is_err());
        let _ = bad;
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
}
