// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Complete, synchronous external mechanical frames for one MCU.
//! The external solver owns q/qd; native motor plants own electrical current
//! and integrate torque impulse over every service substep.

use super::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

const MAX_SAFE_CYCLE: u64 = (1_u64 << 53) - 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalShaftState {
    pub id: String,
    pub position_rad: f64,
    pub angular_velocity_rad_s: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalMechanicalInput {
    pub component: String,
    pub channel: String,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalMechanicalFrame {
    pub at_cycle: u64,
    pub shafts: Vec<ExternalShaftState>,
    pub inputs: Vec<ExternalMechanicalInput>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExternalShaftObservation {
    pub id: String,
    pub position_rad: f64,
    pub angular_velocity_rad_s: f64,
    pub current_a: f64,
    pub torque_impulse_nm_s: f64,
    pub torque_average_nm: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExternalMechanicalInterval {
    pub from_cycle: u64,
    pub at_cycle: u64,
    pub clock_hz: u64,
    pub elapsed_s: f64,
    pub shafts: Vec<ExternalShaftObservation>,
}

impl SystemBus {
    pub fn has_external_mechanical_shafts(&self) -> bool {
        self.motors.iter().any(|motor| {
            matches!(
                motor, MotorRuntime::Dc { plant, .. } if plant.shaft_owner() == ShaftOwner::External
            )
        })
    }

    pub fn external_mechanical_is_primed(&self) -> bool {
        self.external_mechanical_frontier.is_some()
    }

    /// Close electrical integration at the current cycle and observe the
    /// complete interval. Reads do not consume its impulse.
    pub fn get_external_mechanical_interval(
        &mut self,
    ) -> Result<ExternalMechanicalInterval, String> {
        self.validate_external_mechanical_clock()?;
        let from_cycle = self.external_mechanical_frontier.unwrap_or(0);
        if self.external_mechanical_frontier.is_none() && self.current_cycle != 0 {
            return Err("external mechanical shafts must be primed at cycle 0".to_owned());
        }
        if self.current_cycle < from_cycle {
            return Err("external mechanical clock moved behind its committed frame".to_owned());
        }
        self.service_motor_models();
        let elapsed_s = (self.current_cycle - from_cycle) as f64 / self.cpu_hz as f64;
        let mut shafts = Vec::new();
        for motor in &self.motors {
            if let MotorRuntime::Dc {
                id, plant, series, ..
            } = motor
            {
                if plant.shaft_owner() != ShaftOwner::External {
                    continue;
                }
                if let Some(error) = &series.live_error {
                    return Err(format!(
                        "external motor '{id}' electrical integration failed: {error}"
                    ));
                }
                let state = plant.snapshot();
                let impulse = plant.external_torque_impulse_nm_s();
                shafts.push(ExternalShaftObservation {
                    id: id.clone(),
                    position_rad: state.position_rad,
                    angular_velocity_rad_s: state.angular_velocity_rad_s,
                    current_a: state.current_a,
                    torque_impulse_nm_s: impulse,
                    torque_average_nm: (elapsed_s > 0.0).then(|| impulse / elapsed_s),
                });
            }
        }
        Ok(ExternalMechanicalInterval {
            from_cycle,
            at_cycle: self.current_cycle,
            clock_hz: self.cpu_hz,
            elapsed_s,
            shafts,
        })
    }

    /// Commit all externally owned shafts and explicitly addressed inputs at
    /// the current cycle. Preflight runs before even flushing the old electrical
    /// interval: a bad frame changes neither state nor accumulated impulse.
    pub fn apply_external_mechanical_frame(
        &mut self,
        frame: &ExternalMechanicalFrame,
    ) -> Result<(), String> {
        self.validate_external_mechanical_clock()?;
        if frame.at_cycle > MAX_SAFE_CYCLE || frame.at_cycle != self.current_cycle {
            return Err(format!(
                "external frame cycle {} must equal current cycle {}",
                frame.at_cycle, self.current_cycle
            ));
        }
        match self.external_mechanical_frontier {
            Some(previous) if frame.at_cycle <= previous => {
                return Err("external mechanical frame is stale or already committed".to_owned());
            }
            None if frame.at_cycle != 0 => {
                return Err("external mechanical shafts must be primed at cycle 0".to_owned());
            }
            _ => {}
        }
        let indices = self.validate_external_shafts(&frame.shafts)?;
        let mut input_keys = HashSet::new();
        for input in &frame.inputs {
            if input.component.trim().is_empty()
                || input.channel.trim().is_empty()
                || !input.value.is_finite()
                || !input_keys.insert((&input.component, &input.channel))
            {
                return Err(
                    "external frame inputs require unique explicit targets and finite values"
                        .to_owned(),
                );
            }
        }
        let sets: Vec<_> = frame
            .inputs
            .iter()
            .map(|input| {
                (
                    Some(input.component.as_str()),
                    input.channel.as_str(),
                    input.value,
                )
            })
            .collect();
        self.validate_atomic_inputs(&sets)
            .map_err(|error| error.to_string())?;
        let input_keys = self.canonical_atomic_input_keys(&sets);
        if input_keys.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err("external frame repeats a sensor input through aliases".to_owned());
        }
        if self.external_mechanical_frontier.is_some()
            && input_keys != self.external_mechanical_inputs
        {
            return Err("external frame must contain the same complete sensor input set as its priming frame".to_owned());
        }

        // Only a fully valid frame may close or consume the preceding interval.
        self.get_external_mechanical_interval()?;
        for &(component, channel, value) in &sets {
            self.apply_input_at_frontier(component, channel, value)
                .expect("atomic SimInput preflight guarantees the ordered apply pass");
        }
        let mut drives = Vec::new();
        for (index, state) in indices.into_iter().zip(&frame.shafts) {
            if let MotorRuntime::Dc {
                plant,
                encoder,
                feedback,
                index,
                speed_extremes,
                ..
            } = &mut self.motors[index]
            {
                plant
                    .set_external_shaft_state(state.position_rad, state.angular_velocity_rad_s)
                    .expect("shaft kinematics passed the same immutable-parameter preflight");
                plant.clear_external_torque_impulse();
                let snapshot = plant.snapshot();
                speed_extremes.observe(snapshot.speed_rpm);
                let pins = encoder
                    .sample(snapshot.position_rad)
                    .expect("finite position passed encoder preflight");
                if let Some(pin) = feedback.encoder_a {
                    drives.push((pin, pins.a));
                }
                if let Some(pin) = feedback.encoder_b {
                    drives.push((pin, pins.b));
                }
                if let Some(pin) = index {
                    drives.push((*pin, pins.index));
                }
            }
        }
        for (pin, level) in drives {
            self.drive_input(pin, level);
        }
        self.external_mechanical_frontier = Some(frame.at_cycle);
        self.external_mechanical_inputs = input_keys;
        Ok(())
    }

    fn validate_external_mechanical_clock(&self) -> Result<(), String> {
        if !self.has_external_mechanical_shafts() {
            return Err("this MCU has no externally owned bridge DC shafts".to_owned());
        }
        if self.cpu_hz == 0 || self.cpu_hz > MAX_SAFE_CYCLE || self.current_cycle > MAX_SAFE_CYCLE {
            return Err(
                "external mechanical clock must use positive JS-safe integer cycles and Hz"
                    .to_owned(),
            );
        }
        Ok(())
    }

    fn validate_external_shafts(
        &self,
        states: &[ExternalShaftState],
    ) -> Result<Vec<usize>, String> {
        let mut seen = HashSet::new();
        let mut indices = Vec::new();
        for state in states {
            if !seen.insert(state.id.as_str()) {
                return Err(format!("duplicate external shaft '{}'", state.id));
            }
            let index = self
                .motors
                .iter()
                .position(|motor| match motor {
                    MotorRuntime::Dc { id, .. } | MotorRuntime::Bldc { id, .. } => id == &state.id,
                })
                .ok_or_else(|| format!("unknown external shaft '{}'", state.id))?;
            match &self.motors[index] {
                MotorRuntime::Dc { plant, encoder, .. }
                    if plant.shaft_owner() == ShaftOwner::External =>
                {
                    let mut checked = plant.clone();
                    checked
                        .set_external_shaft_state(state.position_rad, state.angular_velocity_rad_s)
                        .map_err(|error| format!("external shaft '{}': {error}", state.id))?;
                    encoder
                        .sample(state.position_rad)
                        .map_err(|error| error.to_string())?;
                }
                _ => {
                    return Err(format!(
                        "shaft '{}' is not owned by the external mechanical solver",
                        state.id
                    ))
                }
            }
            indices.push(index);
        }
        let expected = self.motors.iter().filter(|motor| matches!(
            motor, MotorRuntime::Dc { plant, .. } if plant.shaft_owner() == ShaftOwner::External
        )).count();
        if states.len() != expected {
            return Err(format!(
                "external frame must contain all {expected} owned shafts exactly once"
            ));
        }
        Ok(indices)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::Bus;
    use crate::peripherals::components::declarative_i2c::GenericI2cDevice;
    use crate::peripherals::i2c::I2c;
    use crate::sim_input::SimInput;
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::f64::consts::TAU;

    fn fixture() -> SystemBus {
        let chip: ChipDescriptor = serde_yaml::from_str(
            r#"
name: external-shaft-test
arch: arm
core: cortex-m4
cpu_hz: 1000000
flash: { base: 0x08000000, size: "64KB" }
ram: { base: 0x20000000, size: "32KB" }
peripherals:
  - id: gpioa
    type: gpio
    base_address: 0x48000000
    size: "1KB"
    config: { profile: stm32v2 }
  - id: i2c1
    type: i2c
    base_address: 0x40005400
    size: "1KB"
"#,
        )
        .unwrap();
        let mut manifest: SystemManifest = serde_yaml::from_str(
            r#"
name: external-shaft-test
chip: unused
motor_models:
  - kind: dc
    id: left
    shaft_owner: external
    resistance_ohm: 2.0
    inductance_h: 0.001
    torque_constant_nm_per_a: 0.02
    back_emf_constant_v_per_rad_s: 0.02
    rotor_inertia_kg_m2: 0.0
    viscous_friction_nm_per_rad_s: 0.0
    supply_voltage_v: 3.7
    load_torque_nm: 0.0
    encoder_cpr: 4
    simulation_clock_hz: 1000000
    pwm_pin: PA0
    direction_pin: 3V3
    encoder_a_pin: PA2
    encoder_b_pin: PA3
"#,
        )
        .unwrap();
        let mut right = manifest.motor_models[0].clone();
        let MotorModelConfig::Dc(config) = &mut right else {
            unreachable!()
        };
        config.id = "right".to_owned();
        config.encoder_a_pin = Some("PA4".to_owned());
        config.encoder_b_pin = Some("PA5".to_owned());
        manifest.motor_models.push(right);
        let mut native = manifest.motor_models[0].clone();
        let MotorModelConfig::Dc(config) = &mut native else {
            unreachable!()
        };
        config.id = "native".to_owned();
        config.shaft_owner = MotorShaftOwner::Native;
        config.rotor_inertia_kg_m2 = 0.0001;
        config.viscous_friction_nm_per_rad_s = 0.00001;
        config.encoder_a_pin = None;
        config.encoder_b_pin = None;
        manifest.motor_models.push(native);
        let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
        let mut imu = GenericI2cDevice::from_yaml(
            labwired_config::embedded_device_yaml("mpu6050").unwrap(),
            0x68,
        )
        .unwrap();
        imu.set_component_id("imu".to_owned());
        bus.attach_i2c_slave("i2c1", Box::new(imu)).unwrap();
        bus.write_u32(0x4800_0000, 1).unwrap();
        bus.write_u32(0x4800_0014, 1).unwrap();
        bus
    }

    fn frame(cycle: u64) -> ExternalMechanicalFrame {
        ExternalMechanicalFrame {
            at_cycle: cycle,
            shafts: ["left", "right"]
                .into_iter()
                .map(|id| ExternalShaftState {
                    id: id.to_owned(),
                    position_rad: 0.0,
                    angular_velocity_rad_s: 0.0,
                })
                .collect(),
            inputs: [
                ("ax", 0.0),
                ("ay", 0.0),
                ("az", 1.0),
                ("gx", 0.0),
                ("gy", 0.0),
                ("gz", 0.0),
            ]
            .into_iter()
            .map(|(channel, value)| ExternalMechanicalInput {
                component: "imu".to_owned(),
                channel: channel.to_owned(),
                value,
            })
            .collect(),
        }
    }

    fn imu_value(bus: &mut SystemBus, channel: &str) -> f64 {
        for entry in &mut bus.peripherals {
            if let Some(i2c) = entry
                .dev
                .as_any_mut()
                .and_then(|any| any.downcast_mut::<I2c>())
            {
                for device in i2c.attached_devices() {
                    let mut device = device.borrow_mut();
                    if let Some(imu) = device
                        .as_any_mut()
                        .and_then(|any| any.downcast_mut::<GenericI2cDevice>())
                    {
                        return imu.input_value(channel).unwrap();
                    }
                }
            }
        }
        panic!("IMU fixture was not attached")
    }

    fn plant_states(
        bus: &SystemBus,
    ) -> Vec<(String, crate::physics::motor::BrushedMotorSnapshot, f64)> {
        bus.motors
            .iter()
            .filter_map(|motor| match motor {
                MotorRuntime::Dc { id, plant, .. } => Some((
                    id.clone(),
                    plant.snapshot(),
                    plant.external_torque_impulse_nm_s(),
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn external_frame_advances_electrical_only_and_commits_encoder_and_imu_together() {
        let mut bus = fixture();
        bus.apply_external_mechanical_frame(&frame(0)).unwrap();
        bus.set_current_cycle(1000);
        let interval = bus.get_external_mechanical_interval().unwrap();
        assert_eq!(interval.at_cycle, 1000);
        assert_eq!(interval.elapsed_s, 0.001);
        assert!(interval.shafts.iter().all(|shaft| shaft.current_a > 0.0
            && shaft.torque_impulse_nm_s > 0.0
            && shaft.torque_average_nm.unwrap() > 0.0
            && shaft.position_rad == 0.0
            && shaft.angular_velocity_rad_s == 0.0));
        assert_eq!(bus.get_external_mechanical_interval().unwrap(), interval);
        assert!(bus.motor_speed_rpm("native").unwrap() > 0.0);

        let mut next = frame(1000);
        next.shafts[0].position_rad = TAU * 2.5 / 16.0;
        next.shafts[0].angular_velocity_rad_s = 10.0;
        next.inputs[1].value = 0.35;
        next.inputs[3].value = -12.0;
        bus.apply_external_mechanical_frame(&next).unwrap();
        assert_eq!(imu_value(&mut bus, "ay"), 0.35);
        assert_eq!(imu_value(&mut bus, "gx"), -12.0);
        assert_eq!(bus.read_u32(0x4800_0010).unwrap() & 0b1100, 0b1100);
        let reset = bus.get_external_mechanical_interval().unwrap();
        assert_eq!(reset.from_cycle, 1000);
        assert_eq!(reset.elapsed_s, 0.0);
        assert!(reset
            .shafts
            .iter()
            .all(|shaft| shaft.torque_impulse_nm_s == 0.0 && shaft.torque_average_nm.is_none()));
        assert_eq!(reset.shafts[0].position_rad, next.shafts[0].position_rad);
        assert_eq!(reset.shafts[0].angular_velocity_rad_s, 10.0);
    }

    #[test]
    fn external_frame_bad_targets_or_values_leave_every_state_and_interval_untouched() {
        let mut bus = fixture();
        bus.apply_external_mechanical_frame(&frame(0)).unwrap();
        bus.set_current_cycle(100);
        bus.get_external_mechanical_interval().unwrap();
        bus.set_current_cycle(200); // leave an intentionally unflushed interval
        let before = plant_states(&bus);
        let anchor = bus.motor_cycle_anchor;
        let ay = imu_value(&mut bus, "ay");
        let mut cases = Vec::new();
        let mut bad = frame(199);
        bad.inputs[1].value = 0.7;
        cases.push(bad);
        let mut bad = frame(200);
        bad.shafts[1].id = "missing".to_owned();
        cases.push(bad);
        let mut bad = frame(200);
        bad.shafts[1].id = "native".to_owned();
        cases.push(bad);
        let mut bad = frame(200);
        bad.shafts[1].position_rad = f64::NAN;
        cases.push(bad);
        let mut bad = frame(200);
        bad.shafts[1].angular_velocity_rad_s = f64::INFINITY;
        cases.push(bad);
        let mut bad = frame(200);
        bad.shafts.pop();
        cases.push(bad);
        let mut bad = frame(200);
        bad.shafts[1] = bad.shafts[0].clone();
        cases.push(bad);
        let mut bad = frame(200);
        bad.inputs[5].component = "missing".to_owned();
        cases.push(bad);
        let mut bad = frame(200);
        bad.inputs[5].channel = "missing".to_owned();
        cases.push(bad);
        let mut bad = frame(200);
        bad.inputs[5].value = 10000.0;
        cases.push(bad);
        let mut bad = frame(200);
        bad.inputs[5].value = f64::NAN;
        cases.push(bad);
        let mut bad = frame(200);
        bad.inputs.pop();
        cases.push(bad);
        for bad in cases {
            assert!(
                bus.apply_external_mechanical_frame(&bad).is_err(),
                "{bad:?}"
            );
            assert_eq!(plant_states(&bus), before);
            assert_eq!(bus.motor_cycle_anchor, anchor);
            assert_eq!(bus.external_mechanical_frontier, Some(0));
            assert_eq!(imu_value(&mut bus, "ay"), ay);
        }
        bus.apply_external_mechanical_frame(&frame(200)).unwrap();
        let before = plant_states(&bus);
        assert!(bus.apply_external_mechanical_frame(&frame(200)).is_err());
        assert_eq!(plant_states(&bus), before);
    }

    #[test]
    fn primed_sensor_ownership_blocks_direct_and_aliased_partial_batches() {
        let mut bus = fixture();
        bus.apply_external_mechanical_frame(&frame(0)).unwrap();
        for component in [Some("imu"), Some("i2c1"), None] {
            assert!(bus.set_input(component, "ay", 0.9).is_err());
        }
        // Temperature is unowned and would be a valid write by itself.
        let old_temp = imu_value(&mut bus, "temp");
        assert!(bus
            .set_inputs(&[(Some("imu"), "temp", 30.0), (Some("imu"), "ay", 0.9),])
            .is_err());
        assert_eq!(imu_value(&mut bus, "temp"), old_temp);
        assert_eq!(imu_value(&mut bus, "ay"), 0.0);
        bus.set_input(Some("imu"), "temp", 30.0).unwrap();
        assert_eq!(imu_value(&mut bus, "temp"), 30.0);
        assert!(bus.set_motor_stalled("left", true).is_err());
    }

    #[test]
    fn external_frame_uses_ordered_derived_preflight_before_any_commit() {
        let mut bus = fixture();
        let yaml = r#"
type: exact_sensor
metadata:
  inputs:
    - {key: denominator, label: Denominator, unit: count, min: 0, max: 10, default: 2}
behavior:
  primitive: i2c_device
  derived:
    - {name: quotient, integer: {of: '10 / denominator'}}
  i2c:
    default_address: 0x10
    registers:
      - {name: VALUE, addr: 0, width: 1, endian: be, access: r, source: quotient}
"#;
        let mut sensor = GenericI2cDevice::from_yaml(yaml, 0x10).unwrap();
        assert!(sensor
            .validate_atomic_inputs(&[("denominator", 0.0), ("denominator", 2.0)])
            .is_err());
        sensor.set_component_id("exact".to_owned());
        bus.attach_i2c_slave("i2c1", Box::new(sensor)).unwrap();
        let mut prime = frame(0);
        prime.inputs.push(ExternalMechanicalInput {
            component: "exact".to_owned(),
            channel: "denominator".to_owned(),
            value: 2.0,
        });
        bus.apply_external_mechanical_frame(&prime).unwrap();
        bus.set_current_cycle(100);
        let before = plant_states(&bus);
        let mut invalid = prime.clone();
        invalid.at_cycle = 100;
        invalid.shafts[0].position_rad = 2.0;
        invalid.inputs[1].value = 0.5;
        invalid.inputs.last_mut().unwrap().value = 0.0;
        assert!(bus.apply_external_mechanical_frame(&invalid).is_err());
        assert_eq!(plant_states(&bus), before);
        assert_eq!(imu_value(&mut bus, "ay"), 0.0);
        assert_eq!(bus.motor_cycle_anchor, 0);
    }
    #[test]
    fn native_advance_refuses_unprimed_external_shafts_before_mutation() {
        use crate::{AdvanceRequest, Cpu, Machine};
        let mut machine = Machine::new(crate::cpu::CortexM::new(), fixture());
        let before = plant_states(&machine.bus);
        let pc = machine.cpu.get_pc();
        let error = machine.advance(AdvanceRequest::run(Some(100))).unwrap_err();
        assert!(error.to_string().contains("priming frame"));
        assert_eq!(machine.total_cycles, 0);
        assert_eq!(machine.bus.current_cycle, 0);
        assert_eq!(machine.cpu.get_pc(), pc);
        assert_eq!(plant_states(&machine.bus), before);
        assert!(machine
            .advance_with_window_runner(AdvanceRequest::run(Some(100)), |_, _, _, _, _| panic!(
                "an unprimed machine must not enter a window runner"
            ),)
            .is_err());
    }
}
