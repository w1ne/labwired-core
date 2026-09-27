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
//! Each such device gets ONE [`labwired_config::DeviceDescriptor`] YAML under
//! `configs/devices/`. The descriptor names an irreducible **primitive** (the
//! genuinely un-data-fiable timing algorithm — the Gray-code walk, the matrix
//! reflect, the one-wire frame) and binds its abstract pins to `config:` keys.
//! [`attach`] resolves those bindings and instantiates the primitive; the
//! primitive's Rust model is unchanged, so behavior is byte-identical to the old
//! hand-written arm. Adding a device that reuses an existing primitive is then
//! one YAML file — no new Rust in the attach path.
//!
//! Migrated: rotary encoder (`quadrature`), 4×4 keypad (`matrix`), DHT22/AM2302
//! (`one_wire`), and HC-SR04 (`pulse_echo`). NeoPixel stays a GPIO observer for
//! now (ESP32-S3-specific, not a `BusResidentDevice`). The emitter unification
//! (both engines reading the descriptor's `emit:` block) is the separate next
//! step.

use super::SystemBus;
use anyhow::{anyhow, Result};
use labwired_config::{BrushedMotorConfig, DeviceDescriptor, ExternalDevice};

/// Parse the declarative descriptor for `device_type`, if one is embedded.
/// Returns `Ok(None)` when the type is not declarative (the caller then falls
/// through to the legacy hand-written arms). Descriptors are embedded ONCE in
/// the config crate ([`DeviceDescriptor::embedded`]) so the runtime attach path
/// and the canvas emitter share one source.
pub(crate) fn lookup(device_type: &str) -> Result<Option<DeviceDescriptor>> {
    DeviceDescriptor::embedded(device_type)
}

impl SystemBus {
    /// Attach a declarative GPIO device described by `desc` for the placed
    /// `ext`. Dispatches on the descriptor's primitive; each primitive arm
    /// resolves the descriptor's pin bindings and constructs the (unchanged)
    /// Rust model, so behavior matches the former hand-written arm exactly.
    pub(crate) fn attach_declarative_device(
        &mut self,
        ext: &ExternalDevice,
        desc: &DeviceDescriptor,
    ) -> Result<()> {
        match desc.behavior.primitive.as_str() {
            "quadrature" => self.attach_quadrature(ext, desc),
            "matrix" => self.attach_matrix(ext, desc),
            "one_wire" => self.attach_one_wire(ext, desc),
            "pulse_echo" => self.attach_pulse_echo(ext, desc),
            "dc_motor" => self.attach_dc_motor(ext, desc),
            other => Err(anyhow!(
                "declarative device '{}' names unknown primitive '{}'",
                ext.id,
                other
            )),
        }
    }

    /// `pulse_echo` primitive → [`HcSr04`]. Reproduces the former `"hc-sr04"`/
    /// `"hcsr04"` arm: `trig` resolves to a GPIO **output** (ODR, the sensor
    /// observes the MCU's trigger pulse) and `echo` to a GPIO **input** (IDR,
    /// the sensor drives a distance-proportional pulse back). Unlike the
    /// bus-resident devices this pushes onto the dedicated `hcsr04` list (it
    /// carries the event-scheduler edge path); `distance_cm` is the
    /// host-controlled hand position.
    fn attach_pulse_echo(&mut self, ext: &ExternalDevice, desc: &DeviceDescriptor) -> Result<()> {
        let trig = self.pin_config(ext, desc, "trig", "PA8")?;
        let echo = self.pin_config(ext, desc, "echo", "PA9")?;
        let cpu_hz = param_u64(desc, ext, "cpu_hz", 80_000_000);
        let distance_cm = param_f64(desc, ext, "distance_cm", 50.0) as f32;

        let (trig_addr, trig_bit) = Self::resolve_pin_odr(self, &trig).ok_or_else(|| {
            anyhow!(
                "HC-SR04 '{}' trig_pin '{}' could not be resolved to a GPIO",
                ext.id,
                trig
            )
        })?;
        let (echo_addr, echo_bit) = Self::resolve_pin_idr(self, &echo).ok_or_else(|| {
            anyhow!(
                "HC-SR04 '{}' echo_pin '{}' could not be resolved to a GPIO",
                ext.id,
                echo
            )
        })?;

        self.hcsr04.push(crate::peripherals::hc_sr04::HcSr04::new(
            ext.id.clone(),
            trig_addr,
            trig_bit,
            echo_addr,
            echo_bit,
            cpu_hz,
            distance_cm,
        ));
        Ok(())
    }

    /// `one_wire` primitive → [`Dht22`]. Reproduces the former `"dht22"`/
    /// `"am2302"` arm: the single `data` role resolves to BOTH the pin's output
    /// (ODR, host drive) and input (IDR, sensor reply) register — one
    /// bidirectional wire — and temperature/humidity are host-controlled through
    /// the standard stimulus API.
    fn attach_one_wire(&mut self, ext: &ExternalDevice, desc: &DeviceDescriptor) -> Result<()> {
        let data = self.pin_config(ext, desc, "data", "PA8")?;
        let cpu_hz = param_u64(desc, ext, "cpu_hz", 80_000_000);
        let temperature_c = param_f64(desc, ext, "temperature_c", 22.0) as f32;
        let humidity_pct = param_f64(desc, ext, "humidity_pct", 50.0) as f32;

        let (odr_addr, odr_bit) = Self::resolve_pin_odr(self, &data).ok_or_else(|| {
            anyhow!(
                "DHT22 '{}' data_pin '{}' could not be resolved to a GPIO output",
                ext.id,
                data
            )
        })?;
        let (idr_addr, idr_bit) = Self::resolve_pin_idr(self, &data).ok_or_else(|| {
            anyhow!(
                "DHT22 '{}' data_pin '{}' could not be resolved to a GPIO input",
                ext.id,
                data
            )
        })?;
        debug_assert_eq!(
            odr_bit, idr_bit,
            "ODR and IDR of one pin must share a bit index"
        );

        // Canvas/part type `dht11` shares this descriptor; pack DHT11 integer
        // frames so Adafruit-style `#define DHTTYPE DHT11` firmware gets valid reads.
        let dht11_frame = ext.r#type.eq_ignore_ascii_case("dht11");
        self.gpio_devices.push(Box::new(
            crate::peripherals::components::dht22::Dht22::new_with_frame(
                ext.id.clone(),
                odr_addr,
                idr_addr,
                odr_bit,
                cpu_hz,
                temperature_c,
                humidity_pct,
                dht11_frame,
            ),
        ));
        Ok(())
    }

    /// `matrix` primitive → [`Keypad`]. Reproduces the former `"keypad"` arm:
    /// the `rows` role binds to a 4-entry list of GPIO **output** pads (ODR,
    /// which the keypad observes) and `cols` to a 4-entry list of GPIO **input**
    /// pads (IDR, which it drives); the pressed key is host-controlled through
    /// the `key` stimulus channel.
    fn attach_matrix(&mut self, ext: &ExternalDevice, desc: &DeviceDescriptor) -> Result<()> {
        use crate::peripherals::components::keypad::{Keypad, COLS, ROWS};

        let row_pins = self.pin_list_config(ext, desc, "rows")?;
        let col_pins = self.pin_list_config(ext, desc, "cols")?;

        // Rows are MCU outputs the keypad observes → resolve to ODR.
        let mut row_odr = [(0u64, 0u8); ROWS];
        for (i, pin) in row_pins.iter().enumerate() {
            row_odr[i] = Self::resolve_pin_odr(self, pin).ok_or_else(|| {
                anyhow!(
                    "keypad '{}' row_pin '{}' could not be resolved to a GPIO output",
                    ext.id,
                    pin
                )
            })?;
        }
        // Columns are MCU inputs the keypad drives → resolve to IDR.
        let mut col_idr = [(0u64, 0u8); COLS];
        for (i, pin) in col_pins.iter().enumerate() {
            col_idr[i] = Self::resolve_pin_idr(self, pin).ok_or_else(|| {
                anyhow!(
                    "keypad '{}' col_pin '{}' could not be resolved to a GPIO input",
                    ext.id,
                    pin
                )
            })?;
        }

        self.gpio_devices
            .push(Box::new(Keypad::new(ext.id.clone(), row_odr, col_idr)));
        Ok(())
    }

    /// `quadrature` primitive → [`RotaryEncoder`]. Reproduces the former
    /// `"rotary-encoder"` arm: both channels resolve to a GPIO **input** (IDR)
    /// register, the model walks the Gray sequence onto them, and rotation is
    /// host-controlled through the `position` stimulus channel.
    fn attach_quadrature(&mut self, ext: &ExternalDevice, desc: &DeviceDescriptor) -> Result<()> {
        let clk = self.pin_config(ext, desc, "a", "PA0")?;
        let dt = self.pin_config(ext, desc, "b", "PA1")?;
        let cpu_hz = param_u64(desc, ext, "cpu_hz", 80_000_000);

        let (clk_idr_addr, clk_bit) = Self::resolve_pin_idr(self, &clk).ok_or_else(|| {
            anyhow!(
                "rotary-encoder '{}' clk_pin '{}' could not be resolved to a GPIO input",
                ext.id,
                clk
            )
        })?;
        let (dt_idr_addr, dt_bit) = Self::resolve_pin_idr(self, &dt).ok_or_else(|| {
            anyhow!(
                "rotary-encoder '{}' dt_pin '{}' could not be resolved to a GPIO input",
                ext.id,
                dt
            )
        })?;

        self.gpio_devices.push(Box::new(
            crate::peripherals::components::rotary_encoder::RotaryEncoder::new(
                ext.id.clone(),
                clk_idr_addr,
                clk_bit,
                dt_idr_addr,
                dt_bit,
                cpu_hz,
            ),
        ));
        Ok(())
    }

    /// `dc_motor` primitive → the brushed DC plant (the same runtime the
    /// `motor_models:` manifest section builds). The descriptor binds the
    /// plant's simulation pins; brake/enable/encoder pins are OPTIONAL — a
    /// motor behind an H-bridge has none of them wired, and the emitter omits
    /// those bindings (`required: false`).
    fn attach_dc_motor(&mut self, ext: &ExternalDevice, desc: &DeviceDescriptor) -> Result<()> {
        let pwm_pin = self.pin_config(ext, desc, "pwm", "PA0")?;
        let direction_pin = self.pin_config(ext, desc, "direction", "PA1")?;
        let config = BrushedMotorConfig {
            id: ext.id.clone(),
            resistance_ohm: param_f64(desc, ext, "resistance_ohm", 1.2),
            inductance_h: param_f64(desc, ext, "inductance_h", 0.002),
            torque_constant_nm_per_a: param_f64(desc, ext, "torque_constant_nm_per_a", 0.08),
            back_emf_constant_v_per_rad_s: param_f64(
                desc,
                ext,
                "back_emf_constant_v_per_rad_s",
                0.08,
            ),
            rotor_inertia_kg_m2: param_f64(desc, ext, "rotor_inertia_kg_m2", 0.00004),
            viscous_friction_nm_per_rad_s: param_f64(
                desc,
                ext,
                "viscous_friction_nm_per_rad_s",
                0.00001,
            ),
            supply_voltage_v: param_f64(desc, ext, "supply_voltage_v", 12.0),
            load_torque_nm: param_f64(desc, ext, "load_torque_nm", 0.0),
            encoder_cpr: param_u64(desc, ext, "encoder_cpr", 1024) as u32,
            simulation_clock_hz: param_u64(desc, ext, "simulation_clock_hz", 80_000_000),
            pwm_pin,
            direction_pin,
            brake_pin: self.optional_pin_config(ext, desc, "brake"),
            enable_pin: self.optional_pin_config(ext, desc, "enable"),
            encoder_a_pin: self.optional_pin_config(ext, desc, "encoder_a"),
            encoder_b_pin: self.optional_pin_config(ext, desc, "encoder_b"),
            encoder_index_pin: self.optional_pin_config(ext, desc, "encoder_index"),
            fault_pin: self.optional_pin_config(ext, desc, "fault"),
        };
        let runtime = self.build_dc_motor(config)?;
        self.motors.push(runtime);
        Ok(())
    }

    /// Optional pin role: present in the descriptor but ABSENT from the emitted
    /// config (the emitter omits unwired optional bindings) → `None`. The
    /// `pin_config` default is deliberately empty so a missing key cannot be
    /// mistaken for a real pin label.
    fn optional_pin_config(
        &self,
        ext: &ExternalDevice,
        desc: &DeviceDescriptor,
        role: &str,
    ) -> Option<String> {
        self.pin_config(ext, desc, role, "")
            .ok()
            .filter(|pin| !pin.is_empty())
    }

    /// Resolve the pad label for the abstract pin `role`: read the `config:` key
    /// the descriptor binds that role to, falling back to `default` (preserving
    /// the old arm's fallback for a config that omits the pin).
    fn pin_config(
        &self,
        ext: &ExternalDevice,
        desc: &DeviceDescriptor,
        role: &str,
        default: &str,
    ) -> Result<String> {
        let key = desc.behavior.pins.get(role).ok_or_else(|| {
            anyhow!(
                "declarative device '{}' descriptor is missing pin role '{}'",
                ext.id,
                role
            )
        })?;
        // Accept string labels ("GPIO5", "PA8") or bare integers (5) from emitters.
        // Integer-only configs used to fall through to STM32 defaults (PA9) and
        // fail ESP32-C3 HC-SR04 attach.
        Ok(ext
            .config
            .get(key)
            .and_then(|v| {
                v.as_str()
                    .map(|s| s.to_string())
                    .or_else(|| v.as_i64().map(|n| n.to_string()))
                    .or_else(|| v.as_u64().map(|n| n.to_string()))
            })
            .unwrap_or_else(|| default.to_string()))
    }

    /// Resolve a list-valued pin role (e.g. the keypad's `rows`/`cols`): read
    /// the `config:` key the descriptor binds it to as a 4-entry list of pad
    /// labels. Errors — keyed on the config field name — mirror the former
    /// hand-written `keypad` arm exactly.
    fn pin_list_config(
        &self,
        ext: &ExternalDevice,
        desc: &DeviceDescriptor,
        role: &str,
    ) -> Result<Vec<String>> {
        const EXPECTED: usize = 4;
        let key = desc.behavior.pins.get(role).ok_or_else(|| {
            anyhow!(
                "declarative device '{}' descriptor is missing pin role '{}'",
                ext.id,
                role
            )
        })?;
        let arr = ext
            .config
            .get(key)
            .and_then(|v| v.as_sequence())
            .ok_or_else(|| anyhow!("keypad '{}' config is missing a '{}' list", ext.id, key))?;
        let pins: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        if pins.len() != EXPECTED {
            return Err(anyhow!(
                "keypad '{}' expects exactly {} '{}' entries, got {}",
                ext.id,
                EXPECTED,
                key,
                pins.len()
            ));
        }
        Ok(pins)
    }
}

/// Read a `u64` primitive param: the descriptor's `params.<name>` entry gives
/// the `config:` key and default; `config[key]` overrides it when present.
/// Falls back to `fallback` when the descriptor omits the param entirely.
fn param_u64(desc: &DeviceDescriptor, ext: &ExternalDevice, name: &str, fallback: u64) -> u64 {
    let (config_key, default) = match desc.behavior.params.get(name) {
        Some(v) => (
            v.get("key").and_then(|k| k.as_str()).unwrap_or(name),
            v.get("default")
                .and_then(|d| d.as_u64())
                .unwrap_or(fallback),
        ),
        None => (name, fallback),
    };
    ext.config
        .get(config_key)
        .and_then(|v| v.as_u64())
        .unwrap_or(default)
}

/// Read an `f64` primitive param (temperature, humidity): same `{key, default}`
/// descriptor shape as [`param_u64`], overridden by `config[key]` when present.
fn param_f64(desc: &DeviceDescriptor, ext: &ExternalDevice, name: &str, fallback: f64) -> f64 {
    let (config_key, default) = match desc.behavior.params.get(name) {
        Some(v) => (
            v.get("key").and_then(|k| k.as_str()).unwrap_or(name),
            v.get("default")
                .and_then(|d| d.as_f64())
                .unwrap_or(fallback),
        ),
        None => (name, fallback),
    };
    ext.config
        .get(config_key)
        .and_then(|v| v.as_f64())
        .unwrap_or(default)
}
