// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

use std::f64::consts::TAU;

use super::ModelError;

const MIN_CPR: u32 = 1;
const MAX_CPR: u32 = 1_000_000;
const MAX_EXACT_INTEGER_F64: f64 = 4_503_599_627_370_496.0;

/// Digital outputs produced by a quadrature encoder sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderPins {
    pub a: bool,
    pub b: bool,
    /// Synthetic index pulse that is high only for transition bin zero.
    ///
    /// The pulse is one transition bin wide and repeats once per mechanical
    /// revolution.
    pub index: bool,
}

/// Stateless position-derived quadrature encoder.
///
/// CPR is the number of quadrature cycles per mechanical revolution. Each
/// cycle contains four Gray-code transitions. Samples depend only on the
/// supplied absolute shaft position; the encoder stores no incremental state.
#[derive(Debug, Clone, Copy)]
pub struct QuadratureEncoder {
    cpr: u32,
}

impl QuadratureEncoder {
    /// Creates an encoder with `1..=1_000_000` cycles per revolution.
    pub fn new(cpr: u32) -> Result<Self, ModelError> {
        if !(MIN_CPR..=MAX_CPR).contains(&cpr) {
            return Err(ModelError {
                field: "encoder_cpr",
                message: "must be between 1 and 1,000,000 inclusive".to_owned(),
            });
        }

        Ok(Self { cpr })
    }

    pub fn cpr(&self) -> u32 {
        self.cpr
    }

    /// Returns the number of quadrature state transitions per revolution.
    pub fn transitions_per_revolution(&self) -> u64 {
        u64::from(self.cpr) * 4
    }

    /// Samples the encoder pins at an absolute mechanical shaft position.
    ///
    /// The synthetic index is high only during transition bin zero, making it
    /// one transition bin wide per mechanical revolution.
    pub fn sample(&self, position_rad: f64) -> Result<EncoderPins, ModelError> {
        let transition_index = self.transition_index(position_rad)?;
        let (a, b) = match transition_index % 4 {
            0 => (false, false),
            1 => (false, true),
            2 => (true, true),
            3 => (true, false),
            _ => unreachable!(),
        };

        Ok(EncoderPins {
            a,
            b,
            index: transition_index == 0,
        })
    }

    /// Returns the wrapped transition bin containing an absolute position.
    pub fn transition_index(&self, position_rad: f64) -> Result<u64, ModelError> {
        if !position_rad.is_finite() {
            return Err(ModelError {
                field: "position_rad",
                message: "must be finite".to_owned(),
            });
        }

        let wrapped_position_rad = position_rad.rem_euclid(TAU);
        if wrapped_position_rad == 0.0 || wrapped_position_rad == TAU {
            return Ok(0);
        }

        let transitions = self.transitions_per_revolution();
        let transitions_f64 = transitions as f64;
        let raw_transition = position_rad / TAU * transitions_f64;
        if raw_transition.is_finite() && raw_transition.abs() <= MAX_EXACT_INTEGER_F64 {
            let approximate_bin = raw_transition.floor() as i64;
            let mut lower_bin = approximate_bin - 1;
            let mut upper_bin = approximate_bin + 2;
            let boundary = |bin: i64| bin as f64 * TAU / transitions_f64;

            while boundary(lower_bin) > position_rad {
                lower_bin -= 1;
            }
            while boundary(upper_bin) <= position_rad {
                upper_bin += 1;
            }
            while lower_bin + 1 < upper_bin {
                let candidate_bin = lower_bin + (upper_bin - lower_bin) / 2;
                if boundary(candidate_bin) <= position_rad {
                    lower_bin = candidate_bin;
                } else {
                    upper_bin = candidate_bin;
                }
            }

            return Ok(lower_bin.rem_euclid(transitions as i64) as u64);
        }

        let mut lower_bin = 0_u64;
        let mut upper_bin = transitions;

        while lower_bin + 1 < upper_bin {
            let candidate_bin = lower_bin + (upper_bin - lower_bin) / 2;
            let candidate_boundary = candidate_bin as f64 * TAU / transitions_f64;
            if candidate_boundary <= wrapped_position_rad {
                lower_bin = candidate_bin;
            } else {
                upper_bin = candidate_bin;
            }
        }

        Ok(lower_bin)
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::TAU;

    use super::{EncoderPins, QuadratureEncoder};

    fn pins(a: bool, b: bool, index: bool) -> EncoderPins {
        EncoderPins { a, b, index }
    }

    fn previous_float(value: f64) -> f64 {
        if value.is_sign_negative() {
            f64::from_bits(value.to_bits() + 1)
        } else {
            f64::from_bits(value.to_bits() - 1)
        }
    }

    fn next_float(value: f64) -> f64 {
        if value.is_sign_negative() {
            f64::from_bits(value.to_bits() - 1)
        } else {
            f64::from_bits(value.to_bits() + 1)
        }
    }

    #[test]
    fn zero_position_starts_at_zero_state_with_index() {
        let encoder = QuadratureEncoder::new(4).unwrap();

        assert_eq!(encoder.sample(0.0).unwrap(), pins(false, false, true));
    }

    #[test]
    fn forward_rotation_follows_gray_code_order() {
        let encoder = QuadratureEncoder::new(1).unwrap();
        let samples = [TAU / 8.0, 3.0 * TAU / 8.0, 5.0 * TAU / 8.0, 7.0 * TAU / 8.0]
            .map(|angle| encoder.sample(angle).unwrap());

        assert_eq!(
            samples,
            [
                pins(false, false, true),
                pins(false, true, false),
                pins(true, true, false),
                pins(true, false, false),
            ]
        );
    }

    #[test]
    fn reverse_rotation_naturally_reverses_gray_code_order() {
        let encoder = QuadratureEncoder::new(1).unwrap();
        let samples = [
            -TAU / 8.0,
            -3.0 * TAU / 8.0,
            -5.0 * TAU / 8.0,
            -7.0 * TAU / 8.0,
        ]
        .map(|angle| encoder.sample(angle).unwrap());

        assert_eq!(
            samples,
            [
                pins(true, false, false),
                pins(true, true, false),
                pins(false, true, false),
                pins(false, false, true),
            ]
        );
    }

    #[test]
    fn positive_multi_revolution_positions_wrap_to_the_same_state() {
        let encoder = QuadratureEncoder::new(4).unwrap();
        let position = 3.5 * TAU / encoder.transitions_per_revolution() as f64;

        assert_eq!(
            encoder.sample(position).unwrap(),
            encoder.sample(position + 5.0 * TAU).unwrap()
        );
    }

    #[test]
    fn negative_multi_revolution_positions_wrap_to_the_same_state() {
        let encoder = QuadratureEncoder::new(4).unwrap();
        let position = 13.5 * TAU / encoder.transitions_per_revolution() as f64;

        assert_eq!(
            encoder.sample(position).unwrap(),
            encoder.sample(position - 5.0 * TAU).unwrap()
        );
    }

    #[test]
    fn one_revolution_contains_exactly_four_transitions_per_cpr() {
        let encoder = QuadratureEncoder::new(7).unwrap();
        let transitions = encoder.transitions_per_revolution();
        let observed = (0..transitions)
            .map(|bin| {
                let position = (bin as f64 + 0.5) * TAU / transitions as f64;
                encoder.transition_index(position).unwrap()
            })
            .collect::<Vec<_>>();

        assert_eq!(encoder.cpr(), 7);
        assert_eq!(transitions, 28);
        assert_eq!(observed, (0..transitions).collect::<Vec<_>>());
    }

    #[test]
    fn cpr_four_matches_expected_transition_samples() {
        let encoder = QuadratureEncoder::new(4).unwrap();

        assert_eq!(encoder.sample(0.0).unwrap(), pins(false, false, true));
        assert_eq!(
            encoder.sample(TAU / 16.0).unwrap(),
            pins(false, true, false)
        );
        assert_eq!(
            encoder.sample(-TAU / 16.0).unwrap(),
            pins(true, false, false)
        );
    }

    #[test]
    fn index_pulse_is_one_transition_bin_wide_each_revolution() {
        let encoder = QuadratureEncoder::new(4).unwrap();
        let bin_width = TAU / encoder.transitions_per_revolution() as f64;

        assert!(encoder.sample(0.5 * bin_width).unwrap().index);
        assert!(!encoder.sample(1.5 * bin_width).unwrap().index);
        assert!(!encoder.sample(TAU - 0.5 * bin_width).unwrap().index);
        assert!(encoder.sample(TAU + 0.5 * bin_width).unwrap().index);
    }

    #[test]
    fn tiny_negative_position_clamps_rounded_tau_to_bin_zero() {
        let encoder = QuadratureEncoder::new(4).unwrap();

        assert_eq!(encoder.transition_index(-f64::EPSILON).unwrap(), 0);
    }

    #[test]
    fn cpr_accepts_inclusive_bounds() {
        let minimum = QuadratureEncoder::new(1).unwrap();
        let maximum = QuadratureEncoder::new(1_000_000).unwrap();

        assert_eq!(minimum.transitions_per_revolution(), 4);
        assert_eq!(maximum.transitions_per_revolution(), 4_000_000);
    }

    #[test]
    fn maximum_cpr_does_not_advance_before_a_transition_boundary() {
        let encoder = QuadratureEncoder::new(1_000_000).unwrap();
        let boundary = TAU / encoder.transitions_per_revolution() as f64;

        assert_eq!(encoder.transition_index(boundary - 1e-16).unwrap(), 0);
        assert_eq!(encoder.transition_index(boundary).unwrap(), 1);
        assert_eq!(encoder.transition_index(boundary + 1e-16).unwrap(), 1);
    }

    #[test]
    fn positive_boundary_preserves_adjacent_float_ordering() {
        for cpr in [1, 1_000_000] {
            let encoder = QuadratureEncoder::new(cpr).unwrap();
            let boundary = TAU / encoder.transitions_per_revolution() as f64;

            assert_eq!(
                encoder.transition_index(previous_float(boundary)).unwrap(),
                0
            );
            assert_eq!(encoder.transition_index(boundary).unwrap(), 1);
            assert_eq!(encoder.transition_index(next_float(boundary)).unwrap(), 1);
        }
    }

    #[test]
    fn negative_boundary_preserves_adjacent_float_ordering() {
        for cpr in [1, 1_000_000] {
            let encoder = QuadratureEncoder::new(cpr).unwrap();
            let transitions = encoder.transitions_per_revolution();
            let boundary = -TAU / transitions as f64;

            assert_eq!(
                encoder.transition_index(previous_float(boundary)).unwrap(),
                transitions - 2
            );
            assert_eq!(encoder.transition_index(boundary).unwrap(), transitions - 1);
            assert_eq!(
                encoder.transition_index(next_float(boundary)).unwrap(),
                transitions - 1
            );
        }
    }

    #[test]
    fn positive_revolution_boundary_preserves_adjacent_float_ordering() {
        let encoder = QuadratureEncoder::new(25).unwrap();
        let transitions = encoder.transitions_per_revolution();

        assert_eq!(
            encoder.transition_index(previous_float(TAU)).unwrap(),
            transitions - 1
        );
        assert_eq!(encoder.transition_index(TAU).unwrap(), 0);
        assert_eq!(encoder.transition_index(next_float(TAU)).unwrap(), 0);
    }

    #[test]
    fn negative_multi_revolution_boundary_preserves_adjacent_float_ordering() {
        let encoder = QuadratureEncoder::new(25).unwrap();
        let transitions = encoder.transitions_per_revolution();
        let boundary = -3.0 * TAU;

        assert_eq!(
            encoder.transition_index(previous_float(boundary)).unwrap(),
            transitions - 1
        );
        assert_eq!(encoder.transition_index(boundary).unwrap(), 0);
        assert_eq!(encoder.transition_index(next_float(boundary)).unwrap(), 0);
    }

    #[test]
    fn interior_boundary_preserves_adjacent_float_ordering() {
        for cpr in [25, 1_000_000] {
            let encoder = QuadratureEncoder::new(cpr).unwrap();
            let transitions = encoder.transitions_per_revolution();
            let expected = transitions / 2;
            let boundary = expected as f64 * TAU / transitions as f64;

            assert_eq!(
                encoder.transition_index(previous_float(boundary)).unwrap(),
                expected - 1
            );
            assert_eq!(encoder.transition_index(boundary).unwrap(), expected);
            assert_eq!(
                encoder.transition_index(next_float(boundary)).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn large_finite_positions_preserve_wrapped_phase() {
        for cpr in [1, 4] {
            let encoder = QuadratureEncoder::new(cpr).unwrap();
            let transitions = encoder.transitions_per_revolution();
            let mut checked_nonzero_phase = false;

            for position in [f64::MAX, 1e300, 1e200, 1e100] {
                let wrapped_position = position.rem_euclid(TAU);
                let expected = (wrapped_position / TAU * transitions as f64).floor() as u64;
                if expected != 0 {
                    checked_nonzero_phase = true;
                    assert_eq!(encoder.transition_index(position).unwrap(), expected);
                }
            }

            assert!(checked_nonzero_phase);
        }
    }

    #[test]
    fn cpr_rejects_values_outside_bounds() {
        for cpr in [0, 1_000_001] {
            let error = QuadratureEncoder::new(cpr).err().unwrap();
            assert_eq!(error.field, "encoder_cpr");
        }
    }

    #[test]
    fn sample_rejects_non_finite_positions() {
        let encoder = QuadratureEncoder::new(4).unwrap();

        for position_rad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let error = encoder.sample(position_rad).unwrap_err();
            assert_eq!(error.field, "position_rad");
        }
    }

    #[test]
    fn encoder_types_are_reexported_from_motor_module() {
        let encoder = crate::motor::QuadratureEncoder::new(4).unwrap();
        let pins: crate::motor::EncoderPins = encoder.sample(0.0).unwrap();

        assert!(pins.index);
    }
}
