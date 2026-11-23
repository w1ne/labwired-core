// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Virtual-time arithmetic shared by session execution and trace timestamps.

use std::time::Duration;

/// Convert at a nonzero session clock, retaining Duration's full seconds range.
pub(crate) fn duration_for_cycles(cycles: u64, cpu_hz: u64) -> Duration {
    let seconds = cycles / cpu_hz;
    let nanos = u128::from(cycles % cpu_hz) * 1_000_000_000 / u128::from(cpu_hz);
    Duration::new(seconds, nanos as u32)
}

/// Round up positive durations and saturate only at the cycle counter's limit.
pub(super) fn cycles_for_duration(duration: Duration, cpu_hz: u64) -> u64 {
    // Overflow here already implies a quotient larger than u64::MAX, even
    // after dividing by 1e9. Saturating the product preserves that result.
    let cycles = duration
        .as_nanos()
        .saturating_mul(u128::from(cpu_hz))
        .div_ceil(1_000_000_000);
    u64::try_from(cycles).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_never_requests_execution() {
        for hz in [1, 32_768, 64_000_000, u64::MAX] {
            assert_eq!(cycles_for_duration(Duration::ZERO, hz), 0);
        }
    }

    #[test]
    fn positive_durations_round_up_at_subcycle_boundaries() {
        assert_eq!(cycles_for_duration(Duration::from_nanos(1), 32_768), 1);
        assert_eq!(cycles_for_duration(Duration::from_nanos(125), 8_000_000), 1);
        assert_eq!(cycles_for_duration(Duration::from_nanos(126), 8_000_000), 2);
        assert_eq!(
            cycles_for_duration(Duration::from_millis(10), 64_000_000),
            640_000
        );
    }

    #[test]
    fn cycle_range_boundary_is_exact() {
        let max = Duration::from_secs(u64::MAX);
        assert_eq!(cycles_for_duration(max, 1), u64::MAX);
        assert_eq!(
            cycles_for_duration(max - Duration::from_secs(1), 1),
            u64::MAX - 1
        );
        assert_eq!(cycles_for_duration(max, 2), u64::MAX);
    }

    #[test]
    fn huge_durations_saturate_without_product_overflow() {
        for hz in [1, 64_000_000, u64::MAX] {
            assert_eq!(cycles_for_duration(Duration::MAX, hz), u64::MAX);
        }
    }

    #[test]
    fn conversion_matches_independent_seconds_and_fraction_oracle() {
        for seconds in [0, 1, 17, 1_000_000_000, u64::MAX / 2, u64::MAX] {
            for nanos in [0, 1, 125, 126, 500_000_000, 999_999_999] {
                for hz in [1, 3, 32_768, 8_000_000, 64_000_000, u64::MAX] {
                    // Each product fits u128 without saturation. Even the
                    // largest sum is below 2^128, unlike nanos * hz.
                    let whole = u128::from(seconds) * u128::from(hz);
                    let fraction = (u128::from(nanos) * u128::from(hz)).div_ceil(1_000_000_000);
                    let expected = u64::try_from(whole + fraction).unwrap_or(u64::MAX);
                    assert_eq!(
                        cycles_for_duration(Duration::new(seconds, nanos), hz),
                        expected
                    );
                }
            }
        }
    }

    #[test]
    fn timestamps_retain_seconds_beyond_u64_nanoseconds() {
        assert_eq!(
            duration_for_cycles(u64::MAX, 1),
            Duration::from_secs(u64::MAX)
        );
        assert_eq!(
            duration_for_cycles(u64::MAX, 2),
            Duration::new(u64::MAX / 2, 500_000_000)
        );
        assert_eq!(
            duration_for_cycles(u64::MAX, u64::MAX),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn fractional_timestamps_round_down_without_intermediate_overflow() {
        assert_eq!(duration_for_cycles(1, 3), Duration::from_nanos(333_333_333));
        assert_eq!(
            duration_for_cycles(u64::MAX - 1, u64::MAX),
            Duration::new(0, 999_999_999)
        );
    }
}
