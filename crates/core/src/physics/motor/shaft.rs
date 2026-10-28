// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::f64::consts::TAU;
use std::fmt;

/// Describes an invalid physical-model parameter or state transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelError {
    pub field: &'static str,
    pub message: String,
}

impl ModelError {
    fn new(field: &'static str, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid model field `{}`: {}",
            self.field, self.message
        )
    }
}

impl std::error::Error for ModelError {}

/// Mechanical properties applied to a motor shaft.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShaftParams {
    pub inertia_kg_m2: f64,
    pub viscous_friction_nm_per_rad_s: f64,
    /// Dry (Coulomb) friction of the bearings and brushes, N·m. It opposes
    /// motion with a constant magnitude, and while the shaft is at rest it
    /// holds it there until the other torques exceed it (stiction). Zero
    /// models a frictionless shaft that only viscous drag slows.
    pub coulomb_friction_nm: f64,
    pub load_torque_nm: f64,
}

/// Observable state of a motor shaft.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShaftSnapshot {
    pub position_rad: f64,
    pub wrapped_position_rad: f64,
    pub angular_velocity_rad_s: f64,
    /// Net torque from the most recently completed integration step.
    ///
    /// This stored value is not recomputed when the load torque is changed.
    pub net_torque_nm: f64,
}

/// Deterministic fixed-step mechanical shaft model.
#[derive(Debug, Clone)]
pub struct Shaft {
    params: ShaftParams,
    position_rad: f64,
    angular_velocity_rad_s: f64,
    net_torque_nm: f64,
}

impl Shaft {
    pub fn new(params: ShaftParams) -> Result<Self, ModelError> {
        validate_finite("inertia_kg_m2", params.inertia_kg_m2)?;
        if params.inertia_kg_m2 <= 0.0 {
            return Err(ModelError::new(
                "inertia_kg_m2",
                "must be greater than zero",
            ));
        }

        validate_finite(
            "viscous_friction_nm_per_rad_s",
            params.viscous_friction_nm_per_rad_s,
        )?;
        if params.viscous_friction_nm_per_rad_s < 0.0 {
            return Err(ModelError::new(
                "viscous_friction_nm_per_rad_s",
                "must be non-negative",
            ));
        }

        validate_finite("coulomb_friction_nm", params.coulomb_friction_nm)?;
        if params.coulomb_friction_nm < 0.0 {
            return Err(ModelError::new(
                "coulomb_friction_nm",
                "must be non-negative",
            ));
        }

        validate_finite("load_torque_nm", params.load_torque_nm)?;

        Ok(Self {
            params,
            position_rad: 0.0,
            angular_velocity_rad_s: 0.0,
            net_torque_nm: 0.0,
        })
    }

    pub fn params(&self) -> ShaftParams {
        self.params
    }

    pub fn inertia_kg_m2(&self) -> f64 {
        self.params.inertia_kg_m2
    }

    pub fn viscous_friction_nm_per_rad_s(&self) -> f64 {
        self.params.viscous_friction_nm_per_rad_s
    }

    pub fn load_torque_nm(&self) -> f64 {
        self.params.load_torque_nm
    }

    pub fn position_rad(&self) -> f64 {
        self.position_rad
    }

    /// Returns the shaft position normalized to the half-open range `[0, TAU)`.
    pub fn wrapped_position_rad(&self) -> f64 {
        let wrapped_position_rad = self.position_rad.rem_euclid(TAU);
        if wrapped_position_rad == TAU {
            0.0
        } else {
            wrapped_position_rad
        }
    }

    pub fn angular_velocity_rad_s(&self) -> f64 {
        self.angular_velocity_rad_s
    }

    /// Returns the net torque from the most recently completed integration step.
    ///
    /// This stored value is not recomputed when [`Self::set_load_torque_nm`] is
    /// called; it changes only after the next successful [`Self::step`].
    pub fn net_torque_nm(&self) -> f64 {
        self.net_torque_nm
    }

    /// Captures the current shaft state.
    ///
    /// The snapshot's net torque is the stored torque from the most recently
    /// completed integration step, not a value recomputed after a load change.
    pub fn snapshot(&self) -> ShaftSnapshot {
        ShaftSnapshot {
            position_rad: self.position_rad(),
            wrapped_position_rad: self.wrapped_position_rad(),
            angular_velocity_rad_s: self.angular_velocity_rad_s(),
            net_torque_nm: self.net_torque_nm(),
        }
    }

    /// Updates the signed load torque without recomputing the stored net torque.
    pub fn set_load_torque_nm(&mut self, load_torque_nm: f64) -> Result<(), ModelError> {
        validate_finite("load_torque_nm", load_torque_nm)?;
        self.params.load_torque_nm = load_torque_nm;
        Ok(())
    }

    /// Holds the current position while clearing dynamic shaft state.
    pub(crate) fn hold_still(&mut self) {
        self.angular_velocity_rad_s = 0.0;
        self.net_torque_nm = 0.0;
    }

    /// Advances the shaft by one fixed semi-implicit Euler step.
    ///
    /// Coulomb friction opposes the start-of-step velocity (or, at rest, the
    /// torque trying to move it) and holds a resting shaft whose other torques
    /// it can balance. A step it would carry through zero velocity ends at
    /// rest instead, so a coasting shaft stops in finite time.
    ///
    /// The viscous term uses the angular velocity at the start of the step. For
    /// pure viscous decay, this explicit damping update is stable only when
    /// `dt_s * viscous_friction_nm_per_rad_s / inertia_kg_m2 < 2`. Callers must
    /// currently respect this timestep envelope; later configuration
    /// integration will enforce it.
    pub fn step(&mut self, drive_torque_nm: f64, dt_s: f64) -> Result<(), ModelError> {
        validate_finite("drive_torque_nm", drive_torque_nm)?;
        validate_finite("dt_s", dt_s)?;
        if dt_s <= 0.0 {
            return Err(ModelError::new("dt_s", "must be greater than zero"));
        }

        let velocity = self.angular_velocity_rad_s;
        let coulomb = self.params.coulomb_friction_nm;
        let applied_nm = drive_torque_nm
            - self.params.load_torque_nm
            - self.params.viscous_friction_nm_per_rad_s * velocity;
        let (net_torque_nm, angular_acceleration_rad_s2, angular_velocity_rad_s) =
            if coulomb > 0.0 && velocity == 0.0 && applied_nm.abs() <= coulomb {
                // Stiction: at rest, dry friction balances anything up to it.
                (0.0, 0.0, 0.0)
            } else {
                let opposing = if velocity != 0.0 {
                    velocity.signum()
                } else {
                    applied_nm.signum()
                };
                let net = applied_nm - coulomb * opposing;
                let acceleration = net / self.params.inertia_kg_m2;
                let next = velocity + acceleration * dt_s;
                // Dry friction can stop a shaft but never reverse it: a step
                // that would carry the velocity through zero ends at rest.
                let next = if coulomb > 0.0 && velocity != 0.0 && next * velocity <= 0.0 {
                    0.0
                } else {
                    next
                };
                (net, acceleration, next)
            };
        let position_rad = self.position_rad + angular_velocity_rad_s * dt_s;

        if !net_torque_nm.is_finite()
            || !angular_acceleration_rad_s2.is_finite()
            || !angular_velocity_rad_s.is_finite()
            || !position_rad.is_finite()
        {
            return Err(ModelError::new(
                "state",
                "step would produce a non-finite shaft state",
            ));
        }

        self.net_torque_nm = net_torque_nm;
        self.angular_velocity_rad_s = angular_velocity_rad_s;
        self.position_rad = position_rad;
        Ok(())
    }
}

fn validate_finite(field: &'static str, value: f64) -> Result<(), ModelError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(ModelError::new(field, "must be finite"))
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use super::{ModelError, Shaft, ShaftParams};

    fn unloaded_shaft() -> Shaft {
        Shaft::new(ShaftParams {
            inertia_kg_m2: 0.01,
            viscous_friction_nm_per_rad_s: 0.0,
            coulomb_friction_nm: 0.0,
            load_torque_nm: 0.0,
        })
        .unwrap()
    }

    #[test]
    fn load_and_drag_reduce_acceleration() {
        let mut shaft = Shaft::new(ShaftParams {
            inertia_kg_m2: 0.01,
            viscous_friction_nm_per_rad_s: 0.1,
            coulomb_friction_nm: 0.0,
            load_torque_nm: 1.0,
        })
        .unwrap();
        shaft.step(2.0, 0.1).unwrap();
        assert!((shaft.angular_velocity_rad_s() - 10.0).abs() < 1e-9);
        assert!((shaft.position_rad() - 1.0).abs() < 1e-9);

        shaft.step(2.0, 0.1).unwrap();
        assert!((shaft.angular_velocity_rad_s() - 10.0).abs() < 1e-9);
        assert!((shaft.position_rad() - 2.0).abs() < 1e-9);
        assert!((shaft.net_torque_nm() - 0.0).abs() < 1e-9);
    }

    fn dry_shaft(coulomb_friction_nm: f64) -> Shaft {
        Shaft::new(ShaftParams {
            inertia_kg_m2: 0.01,
            viscous_friction_nm_per_rad_s: 0.0,
            coulomb_friction_nm,
            load_torque_nm: 0.0,
        })
        .unwrap()
    }

    #[test]
    fn stiction_holds_a_resting_shaft_until_the_drive_exceeds_it() {
        let mut shaft = dry_shaft(0.5);
        shaft.step(0.5, 0.1).unwrap();
        assert_eq!(shaft.angular_velocity_rad_s(), 0.0);
        assert_eq!(shaft.position_rad(), 0.0);
        shaft.step(-0.4, 0.1).unwrap();
        assert_eq!(shaft.angular_velocity_rad_s(), 0.0);

        // Breakaway: only the torque above the friction accelerates it.
        shaft.step(1.5, 0.1).unwrap();
        assert!((shaft.angular_velocity_rad_s() - 10.0).abs() < 1e-9);
        assert!((shaft.net_torque_nm() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn coulomb_friction_stops_a_coasting_shaft_and_never_reverses_it() {
        let mut shaft = dry_shaft(0.5);
        shaft.step(1.5, 0.1).unwrap(); // 10 rad/s
        let mut steps = 0;
        while shaft.angular_velocity_rad_s() != 0.0 {
            shaft.step(0.0, 0.03).unwrap();
            assert!(shaft.angular_velocity_rad_s() >= 0.0, "never reverses");
            steps += 1;
            assert!(steps < 100, "a coasting shaft stops in finite time");
        }
        // 0.5 N·m against 0.01 kg·m² at 10 rad/s: 0.2 s, i.e. on the 7th step.
        assert_eq!(steps, 7);
        let position = shaft.position_rad();
        for _ in 0..10 {
            shaft.step(0.0, 0.03).unwrap();
        }
        assert_eq!(shaft.angular_velocity_rad_s(), 0.0, "stays stopped");
        assert_eq!(shaft.position_rad(), position);
    }

    #[test]
    fn coulomb_friction_opposes_reverse_rotation_too() {
        let mut shaft = dry_shaft(0.5);
        shaft.step(-1.5, 0.1).unwrap();
        assert!((shaft.angular_velocity_rad_s() + 10.0).abs() < 1e-9);
        shaft.step(0.0, 0.1).unwrap();
        assert!((shaft.angular_velocity_rad_s() + 5.0).abs() < 1e-9);
    }

    #[test]
    fn negative_or_non_finite_coulomb_friction_is_rejected() {
        for coulomb_friction_nm in [-0.1, f64::NAN, f64::INFINITY] {
            let error = Shaft::new(ShaftParams {
                inertia_kg_m2: 0.01,
                viscous_friction_nm_per_rad_s: 0.0,
                coulomb_friction_nm,
                load_torque_nm: 0.0,
            })
            .unwrap_err();
            assert_eq!(error.field, "coulomb_friction_nm");
        }
    }

    #[test]
    fn reverse_torque_rotates_the_shaft_backwards() {
        let mut shaft = unloaded_shaft();

        shaft.step(-1.0, 0.1).unwrap();

        assert!((shaft.angular_velocity_rad_s() + 10.0).abs() < 1e-9);
        assert!((shaft.position_rad() + 1.0).abs() < 1e-9);
    }

    #[test]
    fn hold_still_preserves_position_and_clears_velocity_and_torque() {
        let mut shaft = unloaded_shaft();
        shaft.step(1.0, 0.1).unwrap();
        let position_rad = shaft.position_rad();

        shaft.hold_still();

        assert_eq!(shaft.position_rad(), position_rad);
        assert_eq!(shaft.angular_velocity_rad_s(), 0.0);
        assert_eq!(shaft.net_torque_nm(), 0.0);
    }

    #[test]
    fn wrapped_angle_is_normalized_without_losing_unwrapped_position() {
        let mut shaft = Shaft::new(ShaftParams {
            inertia_kg_m2: 1.0,
            viscous_friction_nm_per_rad_s: 0.0,
            coulomb_friction_nm: 0.0,
            load_torque_nm: 0.0,
        })
        .unwrap();

        shaft.step(TAU + 0.25, 1.0).unwrap();
        let snapshot = shaft.snapshot();

        assert!((snapshot.position_rad - (TAU + 0.25)).abs() < 1e-9);
        assert!((snapshot.wrapped_position_rad - 0.25).abs() < 1e-9);
        assert!(snapshot.wrapped_position_rad >= 0.0);
        assert!(snapshot.wrapped_position_rad < TAU);
    }

    #[test]
    fn wrapped_angle_stays_below_tau_for_tiny_negative_position() {
        let mut shaft = Shaft::new(ShaftParams {
            inertia_kg_m2: 1.0,
            viscous_friction_nm_per_rad_s: 0.0,
            coulomb_friction_nm: 0.0,
            load_torque_nm: 0.0,
        })
        .unwrap();

        shaft.step(-f64::EPSILON, 1.0).unwrap();
        let wrapped_position_rad = shaft.wrapped_position_rad();

        assert_eq!(shaft.position_rad(), -f64::EPSILON);
        assert!(wrapped_position_rad >= 0.0);
        assert!(wrapped_position_rad < TAU);
    }

    #[test]
    fn zero_inertia_is_rejected() {
        let error = Shaft::new(ShaftParams {
            inertia_kg_m2: 0.0,
            viscous_friction_nm_per_rad_s: 0.0,
            coulomb_friction_nm: 0.0,
            load_torque_nm: 0.0,
        })
        .unwrap_err();

        assert_eq!(error.field, "inertia_kg_m2");
        assert!(error.message.contains("greater than zero"));
    }

    #[test]
    fn negative_viscous_friction_is_rejected() {
        let error = Shaft::new(ShaftParams {
            inertia_kg_m2: 1.0,
            viscous_friction_nm_per_rad_s: -0.1,
            coulomb_friction_nm: 0.0,
            load_torque_nm: 0.0,
        })
        .unwrap_err();

        assert_eq!(error.field, "viscous_friction_nm_per_rad_s");
        assert!(error.message.contains("non-negative"));
    }

    #[test]
    fn non_finite_parameters_are_rejected() {
        let cases = [
            (
                ShaftParams {
                    inertia_kg_m2: f64::NAN,
                    viscous_friction_nm_per_rad_s: 0.0,
                    coulomb_friction_nm: 0.0,
                    load_torque_nm: 0.0,
                },
                "inertia_kg_m2",
            ),
            (
                ShaftParams {
                    inertia_kg_m2: 1.0,
                    viscous_friction_nm_per_rad_s: f64::INFINITY,
                    coulomb_friction_nm: 0.0,
                    load_torque_nm: 0.0,
                },
                "viscous_friction_nm_per_rad_s",
            ),
            (
                ShaftParams {
                    inertia_kg_m2: 1.0,
                    viscous_friction_nm_per_rad_s: 0.0,
                    coulomb_friction_nm: 0.0,
                    load_torque_nm: f64::NEG_INFINITY,
                },
                "load_torque_nm",
            ),
        ];

        for (params, expected_field) in cases {
            let error = Shaft::new(params).unwrap_err();
            assert_eq!(error.field, expected_field);
            assert!(error.message.contains("finite"));
        }
    }

    #[test]
    fn invalid_step_inputs_are_rejected() {
        let mut shaft = unloaded_shaft();

        for drive_torque_nm in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let error = shaft.step(drive_torque_nm, 0.1).unwrap_err();
            assert_eq!(error.field, "drive_torque_nm");
        }
        for dt_s in [0.0, -0.1, f64::NAN, f64::INFINITY] {
            let error = shaft.step(1.0, dt_s).unwrap_err();
            assert_eq!(error.field, "dt_s");
        }
    }

    #[test]
    fn load_torque_setter_accepts_signed_finite_values_and_rejects_non_finite_values() {
        let mut shaft = unloaded_shaft();

        shaft.set_load_torque_nm(-0.25).unwrap();
        assert_eq!(shaft.load_torque_nm(), -0.25);

        let error = shaft.set_load_torque_nm(f64::NAN).unwrap_err();
        assert_eq!(error.field, "load_torque_nm");
        assert_eq!(shaft.load_torque_nm(), -0.25);
    }

    #[test]
    fn non_finite_candidate_state_is_not_published() {
        let mut shaft = Shaft::new(ShaftParams {
            inertia_kg_m2: f64::MIN_POSITIVE,
            viscous_friction_nm_per_rad_s: 0.0,
            coulomb_friction_nm: 0.0,
            load_torque_nm: 0.0,
        })
        .unwrap();
        let before = shaft.snapshot();

        let error = shaft.step(f64::MAX, f64::MAX).unwrap_err();

        assert_eq!(error.field, "state");
        assert_eq!(shaft.snapshot(), before);
    }

    #[test]
    fn model_error_supports_equality_and_standard_error_formatting() {
        let error = ModelError {
            field: "dt_s",
            message: "must be greater than zero".to_string(),
        };
        let cloned = error.clone();

        assert_eq!(error, cloned);
        assert_eq!(
            error.to_string(),
            "invalid model field `dt_s`: must be greater than zero"
        );
        let as_error: &dyn std::error::Error = &error;
        assert_eq!(
            as_error.to_string(),
            "invalid model field `dt_s`: must be greater than zero"
        );
        assert!(format!("{error:?}").contains("dt_s"));
    }
}
