// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! ONE walk over every device an author placed on this machine, whatever
//! transport binds it, and the ONE join that gives each one its manifest
//! identity.
//!
//! # Why this is not per-transport
//!
//! A placed part becomes attached to the machine in one of two structurally
//! different ways, and for a while only the first was visible to `inspect`:
//!
//! 1. **Owned by a controller.** An I²C slave or an SPI panel lives inside the
//!    controller peripheral it answers to, so it is reachable only by asking
//!    that controller — [`crate::Peripheral::for_each_attached_device`]. A walk
//!    over `SystemBus::peripherals` alone cannot see it. That is the bug that
//!    made a customer rig report 52 chip-internal peripherals and not one of
//!    the six parts on the canvas.
//! 2. **Resident on the bus.** An HC-SR04 on TRIG/ECHO, a servo on a PWM pad, a
//!    WS2812 strip, a thermistor driving an ADC channel, a CAN tester node —
//!    none of these answers to an address, so none can live inside a
//!    controller. Each is held in a typed collection on [`SystemBus`] instead.
//!
//! The second kind was invisible to `inspect` for exactly the same reason the
//! first kind had been: the walk did not go where the models are. Fixing it per
//! family would have meant an eighth, ninth, tenth special case — each one a
//! fresh chance to add a device that simulates correctly and reports nothing.
//!
//! So there is one walk ([`SystemBus::for_each_attached_device`]) that covers
//! both kinds, one borrowed record shape
//! ([`crate::inspect::AttachedDeviceRef`]) that both kinds emit, and one join
//! ([`SystemBus::inspect_devices`]) that turns either into the same
//! [`crate::inspect::DeviceInspect`]. A transport is a field of the record, not
//! a branch in the pipeline.
//!
//! # What the two kinds know about themselves
//!
//! They differ in exactly one respect, and the record shape carries it
//! explicitly rather than hiding it:
//!
//! * A controller-hosted device can state its stamped manifest id; this keeps
//!   SPI cascade modules distinct when they share a chip-select. Without a
//!   stamp, its address (or chip-select) and controller identify its placement.
//! * A bus-resident device has no address to be identified by, so it carries
//!   the `external_devices:` id it was stamped with when it was attached, and
//!   the join matches on that. This is not a fallback or a guess: the id is the
//!   author's own text, copied onto the model at attach.
//!
//! # The order is load-bearing
//!
//! Controller-hosted devices are walked first, bus-resident ones after. A
//! declaration can only be claimed once, so this guarantees the addressed join
//! — which has to do real disambiguation work — is never disturbed by a
//! name-matched device, and keeps the `devices` array a stable prefix of what
//! it was before bus-resident devices joined it.

use super::SystemBus;
use crate::inspect::{AttachedDeviceRef, DeviceEvidence};
// `component_id` is the SimInput identity stamp — the same one the stimulus
// walk resolves `set_input`'s `component:` against, so a device answers to the
// same name in `inspect` as it does in the stimulus API.

impl SystemBus {
    /// Walk every attached (off-chip) device on this machine, calling
    /// `f(bus, device)` for each. `bus` is the peripheral the device hangs off
    /// — `Some("i2c0")`, `Some("gpioa")`, `Some("fdcan1")` — or `None` in the
    /// one case where the engine genuinely does not know (see
    /// [`crate::inspect::DeviceAttachment::bus`]).
    ///
    /// This is the ONE walk behind `inspect`'s `devices` array. A device
    /// reachable here is reachable from every inspect surface at once; a
    /// device NOT reachable here is invisible to all of them, which is why
    /// [`Self::for_each_bus_resident_device`] must list every collection of
    /// device models the bus holds. `attached_device_walk_covers_every_bus_
    /// collection` in `crates/core/tests/inspect_device_binding_universal.rs`
    /// is the gate that fails if a new one is added and not walked.
    ///
    /// Borrow discipline: everything happens inside the visitor.
    /// `AttachedDeviceRef` borrows the model, and a controller may be holding a
    /// `RefCell` borrow open for the duration of the call, so the reference
    /// deliberately cannot escape.
    pub fn for_each_attached_device(&self, f: &mut dyn FnMut(Option<&str>, AttachedDeviceRef<'_>)) {
        for entry in &self.peripherals {
            let name = entry.name.as_str();
            entry
                .dev
                .for_each_attached_device(&mut |d| f(Some(d.bus.unwrap_or(name)), d));
        }
        self.for_each_bus_resident_device(f);
    }

    /// Modeled pulse width (µs) for an I2C controller at `address`. `None`
    /// includes silent, absent, unsupported, and unknown outputs. This query
    /// does not prove output-enable, servo power, or physical signal delivery.
    /// Addresses can be ambiguous across buses; this query does not resolve them.
    pub fn pwm_channel_pulse_us(&self, address: u8, channel: u8) -> Option<f64> {
        let mut found = None;
        self.for_each_attached_device(&mut |_, d| {
            if found.is_none() && d.transport == "i2c" && d.address == Some(address) {
                found = d.evidence.and_then(|e| e.pwm_channel_pulse_us(channel));
            }
        });
        found
    }

    /// Bind PWM-driven servos to unambiguous direct sources on supported hosts.
    /// Unsupported controller and mux topologies remain unbound.
    pub fn bind_direct_pwm_servos(&self) {
        // Every rebinding replaces earlier topology-qualified subscriptions.
        for peripheral in &self.peripherals {
            if let Some(host) = peripheral
                .dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::i2c::I2c>())
            {
                for cell in host.attached_devices_internal() {
                    cell.borrow().clear_servo_pwm_observers();
                }
            } else if let Some(host) = peripheral
                .dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::esp32s3::i2c::Esp32s3I2c>())
            {
                host.clear_pwm_subscribers();
            } else if let Some(host) = peripheral
                .dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::esp32::i2c::Esp32I2c>())
            {
                host.clear_pwm_subscribers();
            } else if let Some(host) = peripheral
                .dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::esp32c3::i2c::Esp32c3I2c>())
            {
                host.clear_pwm_subscribers();
            } else if let Some(host) = peripheral
                .dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::nrf52::twim::Nrf52Twim>())
            {
                host.clear_pwm_subscribers();
            } else if let Some(host) = peripheral.dev.as_any().and_then(|a| {
                a.downcast_ref::<crate::peripherals::nrf52::serial_instance::Nrf52SerialInstance>()
            }) {
                host.twim_pwm_host().clear_pwm_subscribers();
            }
        }
        for servo in self.observed_arcs_of::<crate::peripherals::components::servo::Servo>() {
            let (Some((address, channel)), Some(bus_name)) =
                (servo.source(), self.declared_connection(servo.id()))
            else {
                continue;
            };
            // Replay only after every declared direct I2C peer is accounted
            // for by attachment-stamped identity and declared address.
            let expected: Vec<_> = self
                .external_device_decls
                .iter()
                .filter(|d| {
                    d.connection == bus_name
                        && !matches!(
                            d.device_type.as_str(),
                            "servo" | "sg90" | "mg995" | "mg996r" | "mg90s"
                        )
                })
                .collect();
            let Some(peripheral) = self.peripherals.iter().find(|p| p.name == bus_name) else {
                continue;
            };
            let Some(any) = peripheral.dev.as_any() else {
                continue;
            };
            let mut actual: Vec<(Option<String>, u8, bool)> = if let Some(host) =
                any.downcast_ref::<crate::peripherals::i2c::I2c>()
            {
                host.attached_devices_internal()
                    .iter()
                    .map(|cell| {
                        let d = cell.borrow();
                        (
                            d.i2c_component_id().map(str::to_string),
                            d.address(),
                            d.is_i2c_mux(),
                        )
                    })
                    .collect()
            } else if let Some(host) =
                any.downcast_ref::<crate::peripherals::esp32s3::i2c::Esp32s3I2c>()
            {
                host.attached_slaves()
                    .iter()
                    .map(|d| {
                        (
                            d.i2c_component_id().map(str::to_string),
                            d.address(),
                            d.is_i2c_mux(),
                        )
                    })
                    .collect()
            } else if let Some(host) =
                any.downcast_ref::<crate::peripherals::esp32::i2c::Esp32I2c>()
            {
                host.attached_slaves()
                    .iter()
                    .map(|d| {
                        (
                            d.i2c_component_id().map(str::to_string),
                            d.address(),
                            d.is_i2c_mux(),
                        )
                    })
                    .collect()
            } else if let Some(host) =
                any.downcast_ref::<crate::peripherals::esp32c3::i2c::Esp32c3I2c>()
            {
                host.attached_slaves()
                    .iter()
                    .map(|d| {
                        (
                            d.i2c_component_id().map(str::to_string),
                            d.address(),
                            d.is_i2c_mux(),
                        )
                    })
                    .collect()
            } else if let Some(host) =
                any.downcast_ref::<crate::peripherals::nrf52::twim::Nrf52Twim>()
            {
                host.attached_devices_internal()
                    .iter()
                    .map(|cell| {
                        let d = cell.borrow();
                        (
                            d.i2c_component_id().map(str::to_string),
                            d.address(),
                            d.is_i2c_mux(),
                        )
                    })
                    .collect()
            } else if let Some(host) =
                any.downcast_ref::<crate::peripherals::nrf52::serial_instance::Nrf52SerialInstance>(
                )
            {
                host.twim_pwm_host()
                    .attached_devices_internal()
                    .iter()
                    .map(|cell| {
                        let d = cell.borrow();
                        (
                            d.i2c_component_id().map(str::to_string),
                            d.address(),
                            d.is_i2c_mux(),
                        )
                    })
                    .collect()
            } else {
                continue;
            };
            let mut complete = true;
            let mut source_declared = false;
            for declared in expected {
                let matched = actual.iter().position(|(id, addr, is_mux)| {
                    id.as_deref() == Some(declared.id.as_str())
                        && declared.address.is_none_or(|expected| expected == *addr)
                        && (declared.device_type != "tca9548a" || *is_mux)
                });
                if let Some(index) = matched {
                    source_declared |= actual[index].1 == address;
                    actual.remove(index);
                } else {
                    complete = false;
                    break;
                }
            }
            if !complete || !source_declared {
                continue;
            }
            let target = servo.clone();
            let observer: crate::peripherals::device::PwmOutputObserver =
                std::sync::Arc::new(move |ch, us| {
                    if ch == channel {
                        target.apply_pulse_us(us);
                    }
                });
            let (subscribed, existing) = if let Some(host) = peripheral
                .dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::i2c::I2c>())
            {
                (
                    host.subscribe_direct_pwm_output(address, observer),
                    host.direct_pwm_pulse_us(address, channel),
                )
            } else if let Some(host) = peripheral
                .dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::esp32s3::i2c::Esp32s3I2c>())
            {
                (
                    host.subscribe_direct_pwm_output(address, observer),
                    host.direct_pwm_pulse_us(address, channel),
                )
            } else if let Some(host) = peripheral
                .dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::esp32::i2c::Esp32I2c>())
            {
                (
                    host.subscribe_direct_pwm_output(address, observer),
                    host.direct_pwm_pulse_us(address, channel),
                )
            } else if let Some(host) = peripheral
                .dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::esp32c3::i2c::Esp32c3I2c>())
            {
                (
                    host.subscribe_direct_pwm_output(address, observer),
                    host.direct_pwm_pulse_us(address, channel),
                )
            } else if let Some(host) = peripheral
                .dev
                .as_any()
                .and_then(|a| a.downcast_ref::<crate::peripherals::nrf52::twim::Nrf52Twim>())
            {
                (
                    host.subscribe_direct_pwm_output(address, observer),
                    host.direct_pwm_pulse_us(address, channel),
                )
            } else if let Some(host) = peripheral.dev.as_any().and_then(|a| {
                a.downcast_ref::<crate::peripherals::nrf52::serial_instance::Nrf52SerialInstance>()
            }) {
                (
                    host.twim_pwm_host()
                        .subscribe_direct_pwm_output(address, observer),
                    host.twim_pwm_host().direct_pwm_pulse_us(address, channel),
                )
            } else {
                (false, None)
            };
            if subscribed {
                if let Some(us) = existing {
                    servo.apply_pulse_us(us);
                }
            }
        }
    }

    /// Bring every servo wired to a PWM-controller output up to date with the
    /// pulse the firmware last wrote there. Cheap; call before reading servo
    /// state. An absent or unknown pulse leaves the servo where it was.
    pub fn sync_pwm_sourced_servos(&self) {
        self.sync_pwm_sourced_servos_inner(false);
    }

    /// Compatibility readback for hosts without event delivery and existing
    /// raw-mutation callers. Unsupported hosts remain inspection-dependent.
    pub fn sync_legacy_pwm_servos(&self) {
        self.bind_direct_pwm_servos();
        self.sync_pwm_sourced_servos_inner(true);
    }

    fn sync_pwm_sourced_servos_inner(&self, legacy_only: bool) {
        for servo in self.observed_of::<crate::peripherals::components::servo::Servo>() {
            if let (Some((address, channel)), Some(bus_name)) =
                (servo.source(), self.declared_connection(servo.id()))
            {
                if legacy_only {
                    let event_host = self
                        .peripherals
                        .iter()
                        .find(|p| p.name == bus_name)
                        .and_then(|p| p.dev.as_any())
                        .is_some_and(|a| {
                            a.downcast_ref::<crate::peripherals::i2c::I2c>()
                                .is_some_and(|h| !h.legacy_view_escaped())
                                || a.is::<crate::peripherals::esp32s3::i2c::Esp32s3I2c>()
                                || a.is::<crate::peripherals::esp32::i2c::Esp32I2c>()
                                || a.is::<crate::peripherals::esp32c3::i2c::Esp32c3I2c>()
                                || a.downcast_ref::<crate::peripherals::nrf52::twim::Nrf52Twim>().is_some_and(|h| !h.legacy_view_escaped())
                                || a.downcast_ref::<crate::peripherals::nrf52::serial_instance::Nrf52SerialInstance>().is_some_and(|h| !h.twim_pwm_host().legacy_view_escaped())
                        });
                    if event_host {
                        continue;
                    }
                }
                // Supported hosts must qualify even explicit polling: a raw
                // view is not required for an undeclared device to appear.
                let mut escaped_devices = Vec::new();
                let mut escaped = false;
                if let Some(any) = self
                    .peripherals
                    .iter()
                    .find(|p| p.name == bus_name)
                    .and_then(|p| p.dev.as_any())
                {
                    let mut record = |d: &dyn crate::peripherals::i2c::I2cDevice| {
                        escaped_devices.push((
                            d.i2c_component_id().map(str::to_owned),
                            d.address(),
                            d.is_i2c_mux(),
                        ))
                    };
                    if let Some(host) = any.downcast_ref::<crate::peripherals::i2c::I2c>() {
                        escaped = true; host.visit_attached_devices(&mut record);
                    } else if let Some(host) = any.downcast_ref::<crate::peripherals::nrf52::twim::Nrf52Twim>() {
                        escaped = true; host.visit_attached_devices(&mut record);
                    } else if let Some(host) = any.downcast_ref::<crate::peripherals::nrf52::serial_instance::Nrf52SerialInstance>() {
                        escaped = true; host.twim_pwm_host().visit_attached_devices(&mut record);
                    } else if let Some(host) = any.downcast_ref::<crate::peripherals::esp32s3::i2c::Esp32s3I2c>() {
                        escaped = true; for d in host.attached_slaves() { record(d.as_ref()); }
                    } else if let Some(host) = any.downcast_ref::<crate::peripherals::esp32::i2c::Esp32I2c>() {
                        escaped = true; for d in host.attached_slaves() { record(d.as_ref()); }
                    } else if let Some(host) = any.downcast_ref::<crate::peripherals::esp32c3::i2c::Esp32c3I2c>() {
                        escaped = true; for d in host.attached_slaves() { record(d.as_ref()); }
                    }
                }
                if escaped {
                    let mut complete = true;
                    let mut source_declared = false;
                    for declared in self.external_device_decls.iter().filter(|d| {
                        d.connection == bus_name
                            && !matches!(
                                d.device_type.as_str(),
                                "servo" | "sg90" | "mg995" | "mg996r" | "mg90s"
                            )
                    }) {
                        if let Some(index) = escaped_devices.iter().position(|(id, addr, mux)| {
                            id.as_deref() == Some(declared.id.as_str())
                                && declared.address.is_none_or(|a| a == *addr)
                                && (declared.device_type != "tca9548a" || *mux)
                        }) {
                            source_declared |= escaped_devices[index].1 == address;
                            escaped_devices.remove(index);
                        } else {
                            complete = false;
                            break;
                        }
                    }
                    if !complete || !source_declared {
                        continue;
                    }
                }
                // Match the servo's declared bus, not just the address: the
                // same I2C address may exist on multiple controllers. A routed
                // device needs mux/channel wiring evidence that this source
                // declaration does not yet carry, so do not guess that route.
                let mut matches = 0;
                let mut pulse = None;
                let mut nested_route_possible = false;
                self.for_each_attached_device(&mut |bus, d| {
                    if bus == Some(bus_name)
                        && d.transport == "i2c"
                        && d.mux_address.is_some()
                        && d.evidence.is_some_and(|e| e.is_i2c_mux())
                    {
                        nested_route_possible = true;
                    }
                    if bus == Some(bus_name) && d.transport == "i2c" && d.address == Some(address) {
                        matches += 1;
                        if d.mux_address.is_none() {
                            pulse = d.evidence.and_then(|e| e.pwm_channel_pulse_us(channel));
                        }
                    }
                });
                if matches == 1 && !nested_route_possible {
                    if let Some(us) = pulse {
                        servo.apply_pulse_us(us);
                    }
                }
            }
        }
    }

    /// Observe CPU-owned AVR TWI slaves at the instant a TWI operation completes.
    /// No callback escapes the CPU's publicly mutable slave collection.
    pub(crate) fn sync_cpu_i2c_pwm_servos(
        &self,
        devices: &[Box<dyn crate::peripherals::i2c::I2cDevice>],
    ) {
        // CPU-owned mux routing is not qualified here; fail closed.
        if devices.iter().any(|d| d.is_i2c_mux()) {
            return;
        }
        for servo in self.observed_of::<crate::peripherals::components::servo::Servo>() {
            let (Some((address, channel)), Some(bus_name)) =
                (servo.source(), self.declared_connection(servo.id()))
            else {
                continue;
            };
            if !matches!(bus_name, "i2c" | "i2c0" | "twi") {
                continue;
            }
            let mut actual: Vec<_> = devices.iter().collect();
            let mut source_declared = false;
            let mut complete = true;
            for declared in self.external_device_decls.iter().filter(|d| {
                d.connection == bus_name
                    && !matches!(
                        d.device_type.as_str(),
                        "servo" | "sg90" | "mg995" | "mg996r" | "mg90s"
                    )
            }) {
                if let Some(index) = actual.iter().position(|d| {
                    d.i2c_component_id() == Some(declared.id.as_str())
                        && declared.address.is_none_or(|a| a == d.address())
                        && (declared.device_type != "tca9548a" || d.is_i2c_mux())
                }) {
                    source_declared |= actual[index].address() == address;
                    actual.remove(index);
                } else {
                    complete = false;
                    break;
                }
            }
            if !complete || !source_declared {
                continue;
            }
            let mut matching = devices.iter().filter(|d| d.address() == address);
            let Some(source) = matching.next() else {
                continue;
            };
            if matching.next().is_some() {
                continue;
            }
            if let Some(pulse) = source.pwm_channel_pulse_us(channel) {
                servo.apply_pulse_us(pulse);
            }
        }
    }

    /// The `external_devices:` connection recorded for `id`, when the manifest
    /// declared one. This is the author's own text, not an inference.
    fn declared_connection(&self, id: &str) -> Option<&str> {
        self.external_device_decls
            .iter()
            .find(|d| d.id == id)
            .map(|d| d.connection.as_str())
    }

    /// Emit one bus-resident device.
    ///
    /// `r.bus` is what the MODEL knows (a CAN tester and an analog source both
    /// record their `connection:`); when it knows nothing, the declaration it
    /// was built from supplies it. Neither is a guess — both are the manifest
    /// text — and when there is no declaration either, the owner is reported as
    /// unknown rather than invented.
    fn emit_resident(
        &self,
        f: &mut dyn FnMut(Option<&str>, AttachedDeviceRef<'_>),
        r: Resident<'_>,
    ) {
        let bus = r.bus.or_else(|| self.declared_connection(r.declared_id));
        f(
            bus,
            AttachedDeviceRef {
                transport: r.transport,
                address: None,
                cs_pin: None,
                mux_address: None,
                channel: r.channel,
                declared_id: Some(r.declared_id),
                instance_id: r.instance_id,
                bus,
                evidence: r.evidence,
            },
        );
    }

    /// Every collection of author-placed device models that lives DIRECTLY on
    /// the bus rather than inside a controller.
    ///
    /// **Adding a collection to [`SystemBus`] means adding an arm here.** A
    /// model the bus holds but this function does not emit is a device that
    /// simulates and reports nothing — the exact failure this seam exists to
    /// make impossible. The gate test named on
    /// [`Self::for_each_attached_device`] enforces it at the source level, so a
    /// forgotten arm fails the build rather than one customer's rig.
    fn for_each_bus_resident_device(&self, f: &mut dyn FnMut(Option<&str>, AttachedDeviceRef<'_>)) {
        for dev in self.gpio_devices.iter().filter(|d| d.is_attached_device()) {
            // `evidence()` is what lets a bus-resident DISPLAY report at all.
            // The TM1637 and the direct-drive 7-segment digit used to need a
            // typed `SystemBus` field and an arm of their own here precisely
            // because this list could not carry their artifacts; now they are
            // two more entries in it.
            self.emit_resident(f, Resident::gpio(dev.id(), dev.evidence()));
        }
        for dev in &self.observed {
            // ONE arm for every model the bus holds and does nothing with —
            // strip, servo, both stepper kinds, H-bridge channel, parallel
            // panel. Each states its own manifest id, its own channel identity
            // when one declaration built several models, and its own evidence
            // when it is a display. See `crate::bus::ObservedDevice`.
            let mut r = Resident::gpio(dev.manifest_id(), dev.evidence());
            if let Some(model) = dev.model_id() {
                r = r.instance(model);
            }
            self.emit_resident(f, r);
        }
        for dev in &self.analog_inputs {
            // An analog source has no `Any` view, so it is listed but not read:
            // `model: None` says "cannot be read", never "has nothing to show".
            let id = dev.source.component_id().unwrap_or("analog");
            self.emit_resident(f, Resident::analog(id, &dev.connection, dev.channel));
        }
        for mux in &self.analog_muxes {
            // An analog mux drives one ADC channel (its common pin), like an
            // analog source; the sources behind it report the mux as theirs.
            self.emit_resident(f, Resident::analog(&mux.id, &mux.connection, mux.channel));
        }
        for dev in &self.can_diagnostic_testers {
            self.emit_resident(f, Resident::can(&dev.id, &dev.connection));
        }
        for dev in &self.can_uds_testers {
            self.emit_resident(f, Resident::can(&dev.id, &dev.connection));
        }
        for dev in &self.can_log_players {
            self.emit_resident(f, Resident::can(&dev.id, &dev.connection));
        }
        for dev in &self.can_bridges {
            self.emit_resident(f, Resident::can(&dev.id, &dev.controller));
        }
    }

    /// Enumerate the external (off-chip) devices attached to this machine,
    /// joining each live model to the manifest declaration that asked for it.
    ///
    /// Two halves, deliberately in that order:
    ///
    /// 1. **What is really there** comes from walking
    ///    ([`Self::for_each_attached_device`]). A device is listed because a
    ///    live model was found, never because a manifest mentioned one. A
    ///    declaration that failed to build (the `"unsupported type … skipping"`
    ///    path) therefore produces no entry — the record cannot claim a device
    ///    the engine does not have.
    /// 2. **What it is called** comes from
    ///    [`crate::bus::SystemBus::external_device_decls`]. When no declaration
    ///    matches, the id is synthesized from the attachment and `declared` is
    ///    `false`, so a caller can always tell a named device from a guessed
    ///    one.
    ///
    /// This lives on the bus because the bus is the one thing that holds BOTH
    /// the live models and the declarations. `Machine::inspect` delegates to it.
    pub fn inspect_devices(
        &self,
        filter: Option<&str>,
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::DeviceInspect> {
        self.join_devices(filter, None, opts)
    }

    /// Like [`Self::inspect_devices`], but also walks devices supplied by
    /// `extra` (CPU-held SPI/I²C after a `take_*`). Same join against
    /// `external_device_decls`.
    #[allow(clippy::type_complexity)]
    pub fn inspect_devices_with_extra(
        &self,
        filter: Option<&str>,
        opts: &crate::inspect::InspectOpts,
        extra: &mut dyn FnMut(&mut dyn FnMut(Option<&str>, AttachedDeviceRef<'_>)),
    ) -> Vec<crate::inspect::DeviceInspect> {
        self.join_devices_with_extra(filter, None, opts, extra)
    }

    /// What the device the author called `id` is currently showing — THE door
    /// for reading a display, whatever bound it.
    ///
    /// One call, one argument: the name in the manifest. Not the model, not the
    /// transport, not the controller, and above all not a `board_io` binding.
    ///
    /// Each of those used to be a clause a panel could fail. The accessors this
    /// replaces began by looking the display up in `board_io`, so a display
    /// declared only under `external_devices:` — which is a perfectly ordinary
    /// way to place one, and what several shipped rigs do — had no placement for
    /// the renderer to query and painted nothing. Not for one model: for every
    /// model, on every chip. Then, having found a binding, each accessor
    /// re-enumerated by downcast the controllers it believed that panel could
    /// hang off, so a chip whose SPI block was missing from one list rendered
    /// blank there and only there.
    ///
    /// This asks the same walk `inspect` asks and joins on the same manifest
    /// name, so it reaches a bus-resident TM1637 on GPIO pads and an I²C slave
    /// behind a bus switch by the same code path that reaches an SPI panel. A
    /// display the walk cannot reach is invisible to `inspect` too — which is a
    /// bug with one home and one fix, instead of a surface that silently
    /// disagrees with its sibling.
    ///
    /// The answer carries geometry and packing as DATA (`meta.w`, `meta.h`,
    /// `meta.format`), because a caller must be able to render a panel it has
    /// never heard of. A per-accessor doc contract — "153,600 bytes, 240×320,
    /// big-endian RGB565" — is lore that lives in the reader; a new model
    /// cannot supply it, so every new model needed a new accessor and a new
    /// renderer branch.
    ///
    /// Evidence is extracted for the matched device ONLY: the join computes an
    /// id before it asks anything to describe itself, so polling one panel does
    /// not build every other panel's framebuffer.
    pub fn display_artifact(
        &self,
        id: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> Option<crate::inspect::Artifact> {
        self.join_devices(None, Some(id), opts)
            .into_iter()
            .next()?
            .artifacts
            .into_iter()
            .find(crate::inspect::is_display_artifact)
    }

    /// Like [`Self::display_artifact`], also walking CPU-held devices via `extra`.
    #[allow(clippy::type_complexity)]
    pub fn display_artifact_with_extra(
        &self,
        id: &str,
        opts: &crate::inspect::InspectOpts,
        extra: &mut dyn FnMut(&mut dyn FnMut(Option<&str>, AttachedDeviceRef<'_>)),
    ) -> Option<crate::inspect::Artifact> {
        self.join_devices_with_extra(None, Some(id), opts, extra)
            .into_iter()
            .next()?
            .artifacts
            .into_iter()
            .find(crate::inspect::is_display_artifact)
    }

    /// Every display artifact on this machine whose `meta.format` is one of
    /// `formats`, in walk order.
    ///
    /// The format-keyed twin of [`Self::display_artifact`], for callers that
    /// know WHAT they are looking for but not what the author called it — the
    /// `--display-out` exporter, and the byte-exact i80 tests.
    ///
    /// ⚠️ This replaced `bus.observed_of::<Ili9341Parallel>()` at every site
    /// that asked "what did the panel paint". A concrete-type lookup answers
    /// `0` for a panel of any other type — including the SAME panel once it
    /// becomes a descriptor — so a test written that way turns green by
    /// measuring nothing the day the model is ported. Asking for a FORMAT
    /// cannot: `rgb565_be` is what the panel says it holds, in its own words,
    /// and a panel that stopped reporting is a missing artifact rather than a
    /// silent zero.
    pub fn display_artifacts_of_format(
        &self,
        formats: &[&str],
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::Artifact> {
        self.join_devices(None, None, opts)
            .into_iter()
            .flat_map(|d| d.artifacts)
            .filter(|a| {
                a.meta
                    .get("format")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|f| formats.contains(&f))
            })
            .collect()
    }

    /// The join behind [`Self::inspect_devices`] and [`Self::display_artifact`].
    ///
    /// `only_id` restricts the result to the single device that resolves to that
    /// id. It is applied AFTER the id is resolved and BEFORE evidence is asked
    /// for, which is what keeps a single-panel poll cheap without giving the
    /// one-device path its own, subtly different, copy of the join.
    fn join_devices(
        &self,
        filter: Option<&str>,
        only_id: Option<&str>,
        opts: &crate::inspect::InspectOpts,
    ) -> Vec<crate::inspect::DeviceInspect> {
        self.join_devices_with_extra(filter, only_id, opts, &mut |_| {})
    }

    #[allow(clippy::type_complexity)]
    fn join_devices_with_extra(
        &self,
        filter: Option<&str>,
        only_id: Option<&str>,
        opts: &crate::inspect::InspectOpts,
        extra: &mut dyn FnMut(&mut dyn FnMut(Option<&str>, AttachedDeviceRef<'_>)),
    ) -> Vec<crate::inspect::DeviceInspect> {
        self.join_devices_visit(filter, only_id, opts, &mut |_, _| {}, extra)
    }

    /// The named logs of the attached device whose manifest id is `id`
    /// (see [`crate::peripheral_log`]). `None` when no attached device
    /// resolves to that id. The id is resolved by the same join as
    /// [`Self::inspect_devices`], so a device answers to the same name here
    /// as in `inspect`.
    pub fn device_logs(&self, id: &str) -> Option<Vec<crate::peripheral_log::PeripheralLog>> {
        let mut logs = None;
        let opts = crate::inspect::InspectOpts::default();
        self.join_devices_visit(
            None,
            Some(id),
            &opts,
            &mut |_, d| {
                if logs.is_none() {
                    logs = Some(d.evidence.map(|e| e.evidence_logs()).unwrap_or_default());
                }
            },
            &mut |_| {},
        );
        logs
    }

    /// [`Self::join_devices`], calling `visit(id, device)` for each device
    /// that passes the filters, after its id is resolved.
    #[allow(clippy::type_complexity)]
    fn join_devices_visit(
        &self,
        filter: Option<&str>,
        only_id: Option<&str>,
        opts: &crate::inspect::InspectOpts,
        visit: &mut dyn FnMut(&str, &AttachedDeviceRef<'_>),
        extra: &mut dyn FnMut(&mut dyn FnMut(Option<&str>, AttachedDeviceRef<'_>)),
    ) -> Vec<crate::inspect::DeviceInspect> {
        let decls = &self.external_device_decls;
        // A declaration's controller is its own `connection` unless that names
        // another declaration (a bus switch), in which case it inherits the
        // switch's controller and address. Resolved once, up front.
        let effective: Vec<(usize, String, Option<u8>)> = decls
            .iter()
            .enumerate()
            .map(|(i, d)| match decls.iter().find(|p| p.id == d.connection) {
                Some(parent) => (i, parent.connection.clone(), parent.address),
                None => (i, d.connection.clone(), None),
            })
            .collect();

        let mut used = vec![false; decls.len()];
        let mut out = Vec::new();
        let mut one = |bus: Option<&str>, d: AttachedDeviceRef<'_>| {
            if filter.is_some_and(|f| bus != Some(f)) {
                return;
            }
            // Prefer the author's own id stamped onto the model at attach.
            // Some devices have no address; SPI cascade modules share one CS.
            let matched = match d.declared_id {
                // A declaration that fans out to several models (`instance_id`)
                // is claimed by all of them: `used` exists to stop two DIFFERENT
                // devices from taking one name, not to stop a board's own
                // channels from sharing theirs.
                Some(id) => decls
                    .iter()
                    .position(|decl| decl.id == id)
                    .filter(|&i| !used[i] || d.instance_id.is_some()),
                None => Self::match_addressed_decl(decls, &effective, &used, bus, &d),
            };
            if let Some(i) = matched {
                if d.instance_id.is_none() {
                    used[i] = true;
                }
            }

            let id = match (d.instance_id, matched, d.declared_id) {
                (Some(instance), _, _) => instance.to_string(),
                (None, Some(i), _) => decls[i].id.clone(),
                (None, None, Some(stamped)) => stamped.to_string(),
                (None, None, None) => {
                    let bus_name = bus.unwrap_or(d.transport);
                    match (d.address, d.cs_pin) {
                        (Some(a), _) => format!("{bus_name}@0x{a:02x}"),
                        (None, Some(cs)) => format!("{bus_name}@{cs}"),
                        (None, None) => bus_name.to_string(),
                    }
                }
            };
            if only_id.is_some_and(|want| want != id) {
                return;
            }
            visit(&id, &d);
            // The device decides what it can show; this only supplies the id
            // it is addressed by, which is not known until the join above.
            let artifacts = d
                .evidence
                .map(|e| e.artifacts(&id, opts))
                .unwrap_or_default();
            out.push(crate::inspect::DeviceInspect {
                device_type: matched.map(|i| decls[i].device_type.clone()),
                declared: matched.is_some(),
                attachment: crate::inspect::DeviceAttachment {
                    transport: d.transport.to_string(),
                    bus: bus.map(str::to_string),
                    address: d.address,
                    cs_pin: d.cs_pin.map(str::to_string),
                    mux_address: d.mux_address,
                    channel: d.channel,
                },
                id,
                artifacts,
            });
        };
        self.for_each_attached_device(&mut one);
        extra(&mut one);
        out
    }

    /// What the device at a given PLACEMENT is currently showing.
    ///
    /// [`Self::inspect_devices`] answers "what is attached to this machine, and
    /// what is each one called". This answers the different question a renderer
    /// asks sixty times a second: "the panel the author placed on THIS
    /// controller, at THIS address — what is on its screen right now?"
    ///
    /// Both are joins over the same [`Self::for_each_attached_device`] walk and
    /// both read pixels through [`DeviceEvidence::artifacts`], which is the
    /// entire point of this existing. `crates/wasm` used to carry a SECOND
    /// display-evidence system: one bespoke accessor per panel, each
    /// re-enumerating by downcast every controller that panel might hang off.
    /// A controller missing from one of those hand-written lists did not fail
    /// loudly — it rendered a working, actively-painted panel blank. The lists
    /// did not even agree with each other: the ILI9341 could be read off an
    /// ESP32-C3 but not off a classic ESP32, for no reason but which arm
    /// somebody had remembered to write. Controller coverage is now a property
    /// of the walk, so a controller it reaches is readable from `inspect` and
    /// from the renderer at the same instant, and one it does not reach is
    /// invisible to both rather than to one.
    ///
    /// Matching is on PLACEMENT and on what the device says it is — never on a
    /// Rust type and never on a declared type string:
    ///
    /// * `bus` — the controller peripheral name, as the caller's binding states
    ///   it, compared against the name the walk reports for the device.
    /// * `address` — the I²C address the device answers to. `None` does not
    ///   constrain, which is what an SPI caller wants: the accessors this
    ///   replaces never compared chip-selects either.
    /// * `formats` — accepted `meta.format` values. A model names its own
    ///   format next to the buffer that format describes, so this asks the
    ///   DEVICE what it is instead of the caller downcasting to a concrete
    ///   struct. Panels that genuinely share a wire format share an entry: the
    ///   SSD1680 and the UC8151D both emit `epaper_tricolor_1bpp_planes`, and
    ///   treating them as interchangeable is what the UC8151D accessor already
    ///   did by hand — precisely because the DECLARED type is not reliable
    ///   (system YAMLs written before the split still say `ssd1680_*` for a
    ///   panel the ESP32 builder attaches as a `Uc8151dTricolor290`).
    ///
    /// `id` is stamped onto the returned artifact exactly as the manifest join
    /// stamps a declaration id in [`Self::inspect_devices`]; the caller passes
    /// the id it addressed the device by.
    ///
    /// The first matching device wins, in walk order. Evidence is asked for
    /// only AFTER `bus` and `address` have failed to exclude a device, so a
    /// panel's (sometimes expensive) artifact is never built to answer a query
    /// about a different device on a different controller.
    pub fn device_artifact_at(
        &self,
        bus: &str,
        address: Option<u8>,
        formats: &[&str],
        id: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> Option<crate::inspect::Artifact> {
        let mut found: Option<crate::inspect::Artifact> = None;
        self.for_each_attached_device(&mut |on, d| {
            if found.is_some() || on != Some(bus) {
                return;
            }
            if address.is_some() && d.address != address {
                return;
            }
            let Some(evidence) = d.evidence else {
                return;
            };
            found = evidence.artifacts(id, opts).into_iter().find(|a| {
                a.meta
                    .get("format")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|f| formats.contains(&f))
            });
        });
        found
    }

    /// [`Self::device_artifact_at`] without a format filter: whatever DISPLAY
    /// sits at this placement, whichever model it turns out to be.
    ///
    /// The second key of the one door — used only when the manifest join could
    /// not name the device (a model attached programmatically), where the
    /// caller's binding still knows the wire.
    pub fn device_artifact_at_any(
        &self,
        bus: &str,
        address: Option<u8>,
        id: &str,
        opts: &crate::inspect::InspectOpts,
    ) -> Option<crate::inspect::Artifact> {
        let mut found: Option<crate::inspect::Artifact> = None;
        self.for_each_attached_device(&mut |on, d| {
            if found.is_some() || on != Some(bus) {
                return;
            }
            if address.is_some() && d.address != address {
                return;
            }
            let Some(evidence) = d.evidence else {
                return;
            };
            found = evidence
                .artifacts(id, opts)
                .into_iter()
                .find(crate::inspect::is_display_artifact);
        });
        found
    }

    /// The addressed join: which declaration, if any, describes the device
    /// found at this placement.
    ///
    /// A declaration that STATES an address (or chip-select) must match it.
    /// Only a declaration that states none falls back to "the one remaining
    /// candidate", which is the case where the manifest deliberately left the
    /// address to the model's own default. Without that split, the
    /// classic-ESP32 board BMP280 at 0x76 — real, on the bus, and declared by
    /// nobody — was handed the `mux` declaration's name purely for being found
    /// first, which is a fabricated identity.
    fn match_addressed_decl(
        decls: &[super::ExternalDeviceDecl],
        effective: &[(usize, String, Option<u8>)],
        used: &[bool],
        bus: Option<&str>,
        d: &AttachedDeviceRef<'_>,
    ) -> Option<usize> {
        let bus_name = bus?;
        // Candidates: declarations that sit on this controller, at this
        // bus-switch position, and are not already spoken for.
        let candidates: Vec<usize> = effective
            .iter()
            .filter(|(i, conn, mux_addr)| {
                !used[*i]
                    && conn == bus_name
                    && *mux_addr == d.mux_address
                    && decls[*i].channel == d.channel
            })
            .map(|(i, _, _)| *i)
            .collect();
        let key_of = |i: usize| match d.transport {
            "spi" => decls[i].cs_pin.is_some(),
            _ => decls[i].address.is_some(),
        };
        let hit = |i: usize| match d.transport {
            "spi" => decls[i].cs_pin.as_deref() == d.cs_pin,
            _ => decls[i].address == d.address,
        };
        let exact: Vec<usize> = candidates.iter().copied().filter(|&i| hit(i)).collect();
        if exact.len() == 1 {
            return Some(exact[0]);
        }
        if !exact.is_empty() {
            return None;
        }
        let loose: Vec<usize> = candidates.iter().copied().filter(|&i| !key_of(i)).collect();
        (loose.len() == 1).then(|| loose[0])
    }
}

/// One bus-resident device's placement, as the walk knows it — the argument
/// bundle of [`SystemBus::emit_resident`].
///
/// `transport` says how the device BINDS, never what it is: `"gpio"` covers
/// everything wired to pins (bit-banged protocols, one-wire framing,
/// quadrature, PWM), `"analog"` a source that drives an ADC channel's level,
/// `"can"` a second node on a CAN bus. The three constructors below are the
/// only ways to build one, so a new family has to say which of the three it is
/// rather than inventing a fourth vocabulary.
struct Resident<'a> {
    declared_id: &'a str,
    instance_id: Option<&'a str>,
    transport: &'static str,
    bus: Option<&'a str>,
    channel: Option<u8>,
    /// A bus-resident model that can show something of itself implements
    /// [`crate::inspect::DeviceEvidence`] directly and is passed here — the
    /// TM1637 and the direct-driven 7-segment digit are displays that happen to
    /// be wired to pins. `None` is the honest answer for everything else: a
    /// servo and a distance sensor have no display surface, and `None` says
    /// that rather than promising an empty screen.
    evidence: Option<&'a dyn DeviceEvidence>,
}

impl<'a> Resident<'a> {
    /// A device wired to MCU pins. Its owner comes from its declaration: the
    /// model holds pin addresses, not a peripheral name.
    fn gpio(declared_id: &'a str, evidence: Option<&'a dyn DeviceEvidence>) -> Self {
        Self {
            declared_id,
            instance_id: None,
            transport: "gpio",
            bus: None,
            channel: None,
            evidence,
        }
    }

    /// A source driving one ADC channel's level. The ADC channel is placement,
    /// exactly as a bus-switch channel is: "which channel of `bus`".
    fn analog(declared_id: &'a str, connection: &'a str, channel: u8) -> Self {
        Self {
            declared_id,
            instance_id: None,
            transport: "analog",
            bus: Some(connection),
            channel: Some(channel),
            evidence: None,
        }
    }

    /// A second node on a CAN bus, which records the controller it talks to.
    fn can(declared_id: &'a str, connection: &'a str) -> Self {
        Self {
            declared_id,
            instance_id: None,
            transport: "can",
            bus: Some(connection),
            channel: None,
            evidence: None,
        }
    }

    /// Mark this as ONE of several models built from a single declaration.
    fn instance(mut self, instance_id: &'a str) -> Self {
        self.instance_id = Some(instance_id);
        self
    }
}
