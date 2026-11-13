// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! One native actuator observation shape for single-machine and World clients.

use super::SystemBus;

fn motor_states_json(snapshots: Vec<super::MotorSnapshot>) -> Vec<serde_json::Value> {
    snapshots
        .into_iter()
        .map(|snapshot| {
            let mut state = serde_json::Map::from_iter([
                ("id".to_owned(), serde_json::json!(snapshot.id)),
                (
                    "kind".to_owned(),
                    serde_json::json!(match snapshot.kind {
                        "dc" => "dc-motor",
                        "bldc" => "bldc-motor",
                        other => other,
                    }),
                ),
                (
                    "position_rad".to_owned(),
                    serde_json::json!(snapshot.position_rad),
                ),
                (
                    "speed_rpm".to_owned(),
                    serde_json::json!(snapshot.speed_rpm),
                ),
                (
                    "torque_nm".to_owned(),
                    serde_json::json!(snapshot.torque_nm),
                ),
                (
                    "current_a".to_owned(),
                    serde_json::json!(snapshot.current_a.unwrap_or(0.0)),
                ),
                (
                    "bus_voltage_v".to_owned(),
                    serde_json::json!(snapshot.bus_voltage_v),
                ),
                (
                    "control_state".to_owned(),
                    serde_json::json!(snapshot.control_state),
                ),
                ("faults".to_owned(), serde_json::json!(snapshot.faults)),
            ]);
            if let Some(currents) = snapshot.phase_currents_a {
                state.insert("phase_currents_a".to_owned(), serde_json::json!(currents));
            }
            if let Some(sector) = snapshot.commutation_sector {
                state.insert("commutation_sector".to_owned(), serde_json::json!(sector));
            }
            serde_json::Value::Object(state)
        })
        .collect()
}

impl SystemBus {
    /// Observed actuator state from the actual attached native models. No
    /// telemetry-derived estimate or simulation advancement occurs here.
    pub fn actuator_states(&self) -> Vec<serde_json::Value> {
        let mut states: Vec<serde_json::Value> = Vec::new();

        for servo in self.observed_of::<crate::peripherals::components::servo::Servo>() {
            let id = servo.id();
            if id.is_empty() {
                continue;
            }
            states.push(serde_json::json!({
                "id": id,
                "kind": "servo",
                "angle": servo.angle_degrees() as f64,
                "active": servo.is_commanded(),
                "pulse_us": servo.pulse_us() as f64,
            }));
        }
        for stepper in
            self.observed_of::<crate::peripherals::components::step_dir_motor::StepDirMotor>()
        {
            let id = stepper.id();
            if id.is_empty() {
                continue;
            }
            // Capture fields before the JSON object so string keys sit more than
            // two lines away from device_type() — the device-identity scan
            // treats nearby literals as board_io type keys.
            let kind = stepper.device_type().to_string();
            let angle = stepper.angle_degrees_f64();
            let steps = stepper.steps();
            let active = stepper.is_enabled();
            states.push(serde_json::json!({
                "id": id,
                "kind": kind,
                "angle": angle,
                "steps": steps,
                "active": active,
            }));
        }
        // DC/BLDC only — steppers are emitted above (motor_snapshots also
        // folds them for prove evidence, but their actuator shape is angle/steps).
        let dc_bldc: Vec<_> = self
            .motor_snapshots()
            .into_iter()
            .filter(|s| s.kind != crate::bus::STEPPER_SNAPSHOT_KIND)
            .collect();
        states.extend(motor_states_json(dc_bldc));

        states
    }
}

#[cfg(test)]
mod motor_state_tests {
    use super::*;
    use crate::bus::MotorSnapshot;

    #[test]
    fn motor_states_use_stable_browser_kinds_and_current_shapes() {
        let states = motor_states_json(vec![
            MotorSnapshot {
                id: "wheel".into(),
                kind: "dc",
                position_rad: 1.25,
                speed_rpm: 42.0,
                speed_rpm_peak_abs: 42.0,
                speed_rpm_max: 42.0,
                speed_rpm_min: 0.0,
                torque_nm: 0.3,
                current_a: Some(2.5),
                phase_currents_a: None,
                bus_voltage_v: 12.0,
                commutation_sector: None,
                control_state: "forward".into(),
                faults: vec!["stalled".into()],
            },
            MotorSnapshot {
                id: "spindle".into(),
                kind: "bldc",
                position_rad: 2.5,
                speed_rpm: 84.0,
                speed_rpm_peak_abs: 84.0,
                speed_rpm_max: 84.0,
                speed_rpm_min: 0.0,
                torque_nm: 0.6,
                current_a: Some(3.0),
                phase_currents_a: Some([1.0, -0.25, -0.75]),
                bus_voltage_v: 24.0,
                commutation_sector: Some(4),
                control_state: "sector:4".into(),
                faults: vec!["open-phase-a".into()],
            },
        ]);

        assert_eq!(states[0]["kind"], "dc-motor");
        assert_eq!(states[0]["current_a"], 2.5);
        assert!(states[0].get("phase_currents_a").is_none());
        assert!(states[0].get("commutation_sector").is_none());
        assert_eq!(states[1]["kind"], "bldc-motor");
        assert_eq!(states[1]["current_a"], 3.0);
        assert_eq!(
            states[1]["phase_currents_a"],
            serde_json::json!([1.0, -0.25, -0.75])
        );
        assert_eq!(states[1]["commutation_sector"], 4);
        assert_eq!(states[1]["faults"], serde_json::json!(["open-phase-a"]));
    }
}
