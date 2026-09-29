// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Data-driven attach path for the GPIO / pin-timing external-device family.
//!
//! The register-mapped I²C/SPI devices already dispatch through the
//! [`PeripheralKit`](crate::peripherals::kit) registry (no hand-written
//! `from_config` arm each). The GPIO family — rotary encoder, matrix keypad,
//! DHT22, HC-SR04, NeoPixel — historically kept a bespoke `match` arm in
//! [`from_config`](super::from_config) PLUS a hand-mirrored emitter in both the
//! Rust and the TypeScript compiler. That double surface is what this module
//! begins to collapse.
//!
//! GPIO descriptors use generic rules and finite edge schedules; logic devices
//! use truth tables. Attachment resolves physical pads and seeds inputs from
//! descriptor metadata. No protocol-specific Rust model is constructed here.

use super::SystemBus;
use anyhow::{anyhow, Result};
use labwired_config::{DeviceDescriptor, ExternalDevice, PinBinding};

/// Parse the declarative descriptor for `device_type`, if one is embedded.
/// Returns `Ok(None)` when the type is not declarative (the caller then falls
/// through to the legacy hand-written arms). Descriptors are embedded ONCE in
/// the config crate ([`DeviceDescriptor::embedded`]) so the runtime attach path
/// and the canvas emitter share one source.
pub(crate) fn lookup(device_type: &str) -> Result<Option<DeviceDescriptor>> {
    DeviceDescriptor::embedded(device_type)
}

/// Validate the static portion of a GPIO / pin-timing descriptor.
///
/// Pin labels themselves belong to a placed `external_devices` entry and need
/// a concrete MCU to resolve, but every primitive's role-to-config-key mapping
/// is part of the pack. Checking it here lets manifest preflight reject an
/// incomplete private GPIO leaf even when no current canvas references it.
pub(crate) fn validate_descriptor(desc: &DeviceDescriptor) -> Result<()> {
    // The `gpio_device` primitive has no fixed role list — its roles ARE its
    // `pins:` / `outputs:` keys — so it validates itself, rule expressions and
    // all, rather than being checked against a table it does not have.
    if desc.behavior.primitive == "gpio_device" {
        return crate::peripherals::components::declarative_gpio::validate_descriptor(desc);
    }
    // Same argument for `logic_gate`: its roles ARE its `logic:` block, so it
    // validates itself — truth-table expressions, enable targets and
    // transceiver pairing included — rather than being checked against a fixed
    // role table it does not have.
    if desc.behavior.primitive == "logic_gate" {
        return crate::peripherals::components::declarative_logic::validate_descriptor(desc);
    }
    // `segment_display`: its pads are two placement lists, not roles.
    if desc.behavior.primitive == "segment_display" {
        return crate::peripherals::components::declarative_segment_display::validate_descriptor(
            desc,
        );
    }
    if desc.behavior.primitive == "analog_mux" {
        return crate::peripherals::components::declarative_analog_mux::validate_descriptor(desc);
    }
    Err(anyhow!(
        "declarative device '{}' names unknown primitive '{}'",
        desc.r#type,
        desc.behavior.primitive
    ))
}

impl SystemBus {
    /// Attach a declarative GPIO device described by `desc` for the placed
    /// `ext`. Each primitive resolves the descriptor's pin bindings and
    /// constructs its reusable rule or truth-table engine.
    pub(crate) fn attach_declarative_device(
        &mut self,
        ext: &ExternalDevice,
        desc: &DeviceDescriptor,
    ) -> Result<()> {
        validate_descriptor(desc)?;
        match desc.behavior.primitive.as_str() {
            "gpio_device" => self.attach_gpio_device(ext, desc),
            "logic_gate" => self.attach_logic_gate(ext, desc),
            "segment_display" => self.attach_segment_display(ext, desc),
            "analog_mux" => self.attach_analog_mux_device(ext, desc),
            other => Err(anyhow!(
                "declarative device '{}' names unknown primitive '{}'",
                ext.id,
                other
            )),
        }
    }

    /// `gpio_device` primitive → [`DeclarativeGpioDevice`]. The pins-only
    /// Tier-2 part: everything about it is data.
    ///
    /// Role binding follows the same rule the other primitives use — a role
    /// name resolves through `behavior.pins[role]` (or
    /// `behavior.output_pins[role]`) to a `config:` key that carries the pad
    /// label — with one split that matters:
    ///
    /// * `pins:` are pads the MCU DRIVES and the part observes, so they resolve
    ///   to the output register (ODR). Open-drain sampling additionally
    ///   recognizes disabled ESP output drivers through the narrow pad port.
    /// * `outputs:` are pads the PART drives and the MCU samples, so they
    ///   resolve to the input register (IDR).
    ///
    /// Optional default pads belong to `pin_config_defaults` in the descriptor.
    fn attach_gpio_device(&mut self, ext: &ExternalDevice, desc: &DeviceDescriptor) -> Result<()> {
        use crate::peripherals::components::declarative_gpio::{BoundPin, DeclarativeGpioDevice};

        let cpu_hz = param_cpu_hz(desc, ext, self.cpu_hz);
        let pad = |role: &str, key: &str| -> Result<String> {
            ext.config
                .get(key)
                .and_then(|v| {
                    v.as_str()
                        .map(|s| s.to_string())
                        .or_else(|| v.as_i64().map(|n| n.to_string()))
                        .or_else(|| v.as_u64().map(|n| n.to_string()))
                })
                .or_else(|| desc.behavior.pin_config_defaults.get(key).cloned())
                .ok_or_else(|| {
                    anyhow!(
                        "gpio_device '{}' pin role '{}' needs config key '{}', which this \
                         placement does not set",
                        ext.id,
                        role,
                        key
                    )
                })
        };

        let mut bindings = std::collections::BTreeMap::new();
        let mut list_roles = std::collections::BTreeSet::new();
        for (role, binding) in &desc.behavior.pins {
            let labels = match binding {
                PinBinding::Scalar(key) => vec![pad(role, key)?],
                PinBinding::List(keys) => keys
                    .iter()
                    .map(|key| pad(role, key))
                    .collect::<Result<Vec<_>>>()?,
                PinBinding::ConfigList { config, count } => {
                    self.pin_list_config(ext, config, *count)?
                }
            };
            for (name, label) in binding.names(role).into_iter().zip(labels) {
                if !matches!(binding, PinBinding::Scalar(_)) {
                    list_roles.insert(name.clone());
                }
                if bindings.insert(name.clone(), label).is_some() {
                    return Err(anyhow!(
                        "gpio_device '{}' duplicates pin role '{}'",
                        ext.id,
                        name
                    ));
                }
            }
        }
        let mut observed = Vec::new();
        for (role, label) in &bindings {
            if list_roles.contains(role) && desc.behavior.outputs.contains(role) {
                continue;
            }
            let (addr, bit) = Self::resolve_pin_odr(self, label).ok_or_else(|| {
                anyhow!(
                    "gpio_device '{}' pin '{}' ({}) could not be resolved to a GPIO output",
                    ext.id,
                    role,
                    label
                )
            })?;
            observed.push(BoundPin {
                role: role.clone(),
                addr,
                bit,
            });
        }
        let mut driven = Vec::new();
        for role in &desc.behavior.outputs {
            let label = match desc.behavior.output_pins.get(role) {
                Some(key) => pad(role, key)?,
                None => match bindings.get(role) {
                    Some(label) if list_roles.contains(role) => label.clone(),
                    _ => pad(role, role)?,
                },
            };
            let (addr, bit) = Self::resolve_pin_idr(self, &label).ok_or_else(|| {
                anyhow!(
                    "gpio_device '{}' output '{}' ({}) could not be resolved to a GPIO input",
                    ext.id,
                    role,
                    label
                )
            })?;
            driven.push(BoundPin {
                role: role.clone(),
                addr,
                bit,
            });
        }

        let channels = crate::peripherals::components::declarative_i2c::owned_channels(desc);
        // Supply state. `Some(false)` is the only value that changes anything;
        // an absent key means powered. See `components::supply`.
        let mut device = DeclarativeGpioDevice::new(
            ext.id.clone(),
            desc,
            observed,
            driven,
            cpu_hz,
            channels.clone(),
        )?
        .with_powered(crate::peripherals::components::supply::powered_from_placement(ext));
        let specs = desc
            .metadata
            .as_ref()
            .map(|m| m.inputs.as_slice())
            .unwrap_or(&[]);
        for (key, value) in labwired_config::seeded_channel_values(specs, |key: &str| {
            ext.config.get(key).and_then(|value| value.as_f64())
        }) {
            device.seed_input(&key, value);
        }
        self.gpio_devices.push(Box::new(device));
        Ok(())
    }

    /// `logic_gate` primitive → [`DeclarativeLogicDevice`]. A 74-series part
    /// whose whole model is a truth table.
    ///
    /// Pad binding follows the same split every pin-driven primitive uses, with
    /// one addition the others do not need:
    ///
    /// * an INPUT or a CONTROL role (an enable, a direction, a select) is a pad
    ///   the MCU drives, so it resolves to the output register (ODR);
    /// * an OUTPUT role is a pad this part drives, so it resolves to the input
    ///   register (IDR);
    /// * a TRANSCEIVER role is BOTH, and is bound at both ends here, because
    ///   which end is live is decided by the DIR pad at run time and must not
    ///   cost a pad re-resolution per pass.
    ///
    /// The `config:` key for a role defaults to `<role lowercased>_pin` — see
    /// [`config_key_for`](crate::peripherals::components::declarative_logic::config_key_for).
    /// An eight-bit transceiver binds twenty pads, and a `pins:` block spelling
    /// each of them out would be twenty lines of `A1: a1_pin`.
    fn attach_logic_gate(&mut self, ext: &ExternalDevice, desc: &DeviceDescriptor) -> Result<()> {
        use crate::peripherals::components::declarative_logic::{
            config_key_for, pad_roles, DeclarativeLogicDevice, LogicPad,
        };

        let spec = desc
            .behavior
            .logic
            .as_ref()
            .ok_or_else(|| anyhow!("logic_gate '{}' has no `logic:` block", ext.id))?;
        let cpu_hz = param_cpu_hz(desc, ext, self.cpu_hz);
        let (observed_roles, driven_roles) = pad_roles(spec);

        let label = |role: &str| -> Result<String> {
            let key = config_key_for(desc, role);
            ext.config
                .get(&key)
                .and_then(|v| {
                    v.as_str()
                        .map(|s| s.to_string())
                        .or_else(|| v.as_i64().map(|n| n.to_string()))
                        .or_else(|| v.as_u64().map(|n| n.to_string()))
                })
                .ok_or_else(|| {
                    anyhow!(
                        "logic_gate '{}' pin role '{}' needs config key '{}', which this \
                         placement does not set",
                        ext.id,
                        role,
                        key
                    )
                })
        };

        let mut observed = Vec::with_capacity(observed_roles.len());
        for role in &observed_roles {
            let pin = label(role)?;
            let odr = Self::resolve_pin_odr(self, &pin).ok_or_else(|| {
                anyhow!(
                    "logic_gate '{}' input '{}' ({}) could not be resolved to a GPIO output",
                    ext.id,
                    role,
                    pin
                )
            })?;
            observed.push(LogicPad {
                role: role.clone(),
                odr: Some(odr),
                idr: None,
            });
        }

        let mut driven = Vec::with_capacity(driven_roles.len());
        for role in &driven_roles {
            let pin = label(role)?;
            let idr = Self::resolve_pin_idr(self, &pin).ok_or_else(|| {
                anyhow!(
                    "logic_gate '{}' output '{}' ({}) could not be resolved to a GPIO input",
                    ext.id,
                    role,
                    pin
                )
            })?;
            driven.push(LogicPad {
                role: role.clone(),
                odr: None,
                idr: Some(idr),
            });
        }

        self.gpio_devices.push(Box::new(DeclarativeLogicDevice::new(
            ext.id.clone(),
            desc,
            observed,
            driven,
            cpu_hz,
        )?));
        Ok(())
    }

    /// `segment_display` primitive → [`DeclarativeSegmentDisplay`]. A
    /// multiplexed segment LED display: the placement lists its segment pads
    /// and its digit-select pads (any count), their active levels and its
    /// font; the descriptor gives the persistence defaults.
    ///
    /// [`DeclarativeSegmentDisplay`]: crate::peripherals::components::declarative_segment_display::DeclarativeSegmentDisplay
    fn attach_segment_display(
        &mut self,
        ext: &ExternalDevice,
        desc: &DeviceDescriptor,
    ) -> Result<()> {
        use crate::peripherals::components::declarative_segment_display::{
            DeclarativeSegmentDisplay, Pad, SegmentDisplaySpec,
        };

        let cpu_hz = param_cpu_hz(desc, ext, self.cpu_hz);
        let spec = SegmentDisplaySpec::from_config(
            &ext.config,
            param_u64(desc, ext, "persistence_us")?,
            param_u64(desc, ext, "threshold_pct")?,
            param_u64(desc, ext, "min_duty_pct")?,
        )
        .map_err(|e| anyhow!("segment_display '{}': {e}", ext.id))?;
        let resolve = |bus: &SystemBus, what: &str, label: &str| -> Result<Pad> {
            let (addr, bit) = Self::resolve_pin_odr(bus, label).ok_or_else(|| {
                anyhow!(
                    "segment_display '{}' {what} pad '{label}' could not be resolved to a GPIO output",
                    ext.id
                )
            })?;
            Ok(Pad { addr, bit })
        };
        let segments = spec
            .segment_pins
            .iter()
            .map(|l| resolve(self, "segment", l))
            .collect::<Result<Vec<_>>>()?;
        let digits = spec
            .digit_pins
            .iter()
            .map(|l| resolve(self, "digit", l))
            .collect::<Result<Vec<_>>>()?;
        let device =
            DeclarativeSegmentDisplay::new(ext.id.clone(), spec, segments, digits, cpu_hz)?;
        self.gpio_devices.push(Box::new(device));
        Ok(())
    }

    /// `analog_mux` primitive → [`AnalogMux`](crate::peripherals::components::declarative_analog_mux::AnalogMux).
    ///
    /// The placement's `connection:` is the ADC (or another mux) that Z
    /// drives and `config.channel` its channel. Every select role is a pad
    /// the MCU drives; the enable role is optional in the placement (absent =
    /// tied to its active level). The `config:` key of a role is
    /// `<role lowercased>_pin`, as for `logic_gate`.
    fn attach_analog_mux_device(
        &mut self,
        ext: &ExternalDevice,
        desc: &DeviceDescriptor,
    ) -> Result<()> {
        use crate::peripherals::components::declarative_analog_mux::{AnalogMux, MuxPad};
        use crate::peripherals::components::declarative_logic::config_key_for;

        let spec = desc
            .behavior
            .analog_mux
            .as_ref()
            .ok_or_else(|| anyhow!("analog_mux '{}' has no `analog_mux:` block", ext.id))?;
        let channel = match ext.config.get("channel").and_then(|v| v.as_u64()) {
            Some(c) if c <= 255 => c as u8,
            Some(c) => {
                return Err(anyhow!(
                    "analog_mux '{}': channel {c} is outside 0..=255",
                    ext.id
                ))
            }
            None => {
                return Err(anyhow!(
                    "analog_mux '{}': config `channel` (the ADC channel the common pin drives) \
                     is required",
                    ext.id
                ))
            }
        };
        let label = |role: &str| -> Option<(String, String)> {
            let key = config_key_for(desc, role);
            let v = ext.config.get(&key)?;
            let label = v
                .as_str()
                .map(str::to_owned)
                .or_else(|| v.as_u64().map(|n| n.to_string()))?;
            Some((key, label))
        };
        let pad = |bus: &SystemBus, role: &str, key: &str, label: &str| -> Result<MuxPad> {
            let (addr, bit) = Self::resolve_pin_odr(bus, label).ok_or_else(|| {
                anyhow!(
                    "analog_mux '{}' pin {role} ({key} = '{label}') is not a GPIO pad of this chip",
                    ext.id
                )
            })?;
            let peripheral = bus.find_peripheral_index(addr).ok_or_else(|| {
                anyhow!(
                    "analog_mux '{}' pin {role} ('{label}') has no GPIO peripheral",
                    ext.id
                )
            })?;
            if bus.peripherals[peripheral].dev.read_gpio_pad(bit).is_none() {
                return Err(anyhow!(
                    "analog_mux '{}' pin {role} ('{label}'): the GPIO model cannot report this \
                     pad's level, so the mux could not follow it",
                    ext.id
                ));
            }
            Ok(MuxPad {
                label: label.to_string(),
                peripheral,
                bit,
            })
        };

        let mut select = Vec::with_capacity(spec.select.len());
        for role in &spec.select {
            let (key, l) = label(role).ok_or_else(|| {
                anyhow!(
                    "analog_mux '{}' select pin {role} needs config key '{}', which this \
                     placement does not set",
                    ext.id,
                    config_key_for(desc, role)
                )
            })?;
            select.push(pad(self, role, &key, &l)?);
        }
        let enable = match &spec.enable {
            Some(e) => match label(&e.pin) {
                Some((key, l)) => Some((pad(self, &e.pin, &key, &l)?, e.active)),
                None => None,
            },
            None => None,
        };
        let mux = AnalogMux::new(
            ext.id.clone(),
            ext.connection.clone(),
            channel,
            select,
            enable,
        );
        self.attach_analog_mux(mux)
    }

    /// Resolve a declared fixed-size config list, with no device-specific count.
    fn pin_list_config(
        &self,
        ext: &ExternalDevice,
        key: &str,
        count: usize,
    ) -> Result<Vec<String>> {
        let arr = ext
            .config
            .get(key)
            .and_then(|v| v.as_sequence())
            .ok_or_else(|| {
                anyhow!(
                    "declarative device '{}' config is missing a '{}' list",
                    ext.id,
                    key
                )
            })?;
        if arr.len() != count {
            return Err(anyhow!(
                "declarative device '{}' expects exactly {} '{}' entries, got {}",
                ext.id,
                count,
                key,
                arr.len()
            ));
        }
        arr.iter().enumerate().map(|(i, v)| {
            v.as_str().map(str::to_owned)
                .or_else(|| v.as_i64().map(|n| n.to_string()))
                .or_else(|| v.as_u64().map(|n| n.to_string()))
                .ok_or_else(|| anyhow!("declarative device '{}' config '{}[{}]' must be a pin label or integer", ext.id, key, i))
        }).collect()
    }
}

/// Resolve a self-timing device's simulated CPU clock, in Hz.
///
/// Order: the placed device's own `config.cpu_hz` (what the diagram emitter
/// writes) → the system's clock, i.e. the manifest's `cpu_hz:` if it declares
/// one and otherwise the chip descriptor's → the device descriptor's
/// `default:`.
///
/// The middle step is the whole point. A DHT22 converts its datasheet
/// microseconds to simulated cycles with this number, so feeding it a clock
/// the firmware was not built against stretches every bit cell by the ratio
/// between the two and the firmware decodes noise. Until the chip descriptor
/// carried the clock, that middle step did not exist: the descriptor's flat
/// 80 MHz default answered for a 16 MHz ATmega and a 160 MHz C3 alike.
fn param_cpu_hz(desc: &DeviceDescriptor, ext: &ExternalDevice, system_cpu_hz: u64) -> u64 {
    let entry = desc.behavior.params.get("cpu_hz");
    let config_key = entry
        .and_then(|v| v.get("key"))
        .and_then(|k| k.as_str())
        .unwrap_or("cpu_hz");
    if let Some(explicit) = ext.config.get(config_key).and_then(|v| v.as_u64()) {
        return explicit;
    }
    if system_cpu_hz > 0 {
        return system_cpu_hz;
    }
    entry
        .and_then(|v| v.get("default"))
        .and_then(|d| d.as_u64())
        .unwrap_or(DEFAULT_DEVICE_CPU_HZ)
}

/// An integer `behavior.params` entry: the placement `config:` value under
/// the param's `key` (default: the param name), else the descriptor default.
fn param_u64(desc: &DeviceDescriptor, ext: &ExternalDevice, name: &str) -> Result<u64> {
    let entry = desc.behavior.params.get(name);
    let config_key = entry
        .and_then(|v| v.get("key"))
        .and_then(|k| k.as_str())
        .unwrap_or(name);
    if let Some(value) = ext.config.get(config_key) {
        return value.as_u64().ok_or_else(|| {
            anyhow!(
                "'{}' config '{config_key}' must be a non-negative integer",
                ext.id
            )
        });
    }
    entry
        .and_then(|v| v.get("default"))
        .and_then(|d| d.as_u64())
        .ok_or_else(|| anyhow!("'{}' has no value for param '{name}'", ext.id))
}

/// Last-resort clock for a self-timed device: no `config.cpu_hz`, no system
/// clock, no descriptor default. Kept at the value every declarative device
/// descriptor already carries so a chip YAML written before
/// `ChipDescriptor::cpu_hz` behaves exactly as it did.
const DEFAULT_DEVICE_CPU_HZ: u64 = 80_000_000;
