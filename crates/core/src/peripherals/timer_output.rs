// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Register-independent STM32 output-stage pulse windows. Both pad publication
//! and motor integration use these exact engine-cycle boundaries.

/// One high interval per period, in engine cycles. Zero/full width are levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutputPulse {
    pub period: u64,
    pub start: u64,
    pub width: u64,
}

impl OutputPulse {
    pub fn level_at(self, phase: u64) -> bool {
        self.width >= self.period
            || (self.width != 0
                && (phase % self.period + self.period - self.start % self.period) % self.period
                    < self.width)
    }

    pub fn inverted(self) -> Self {
        Self {
            start: (self.start + self.width) % self.period,
            width: self.period - self.width,
            ..self
        }
    }

    /// Dead time delays activation, never deactivation. A pulse shorter than
    /// the delay disappears. A constant OCREF has no periodic transition to
    /// delay (RM0351 Rev 10, section 30.3.15, figures 264–266).
    fn delay_activation(self, delay: u64) -> Self {
        if self.width == 0 || self.width == self.period {
            return self;
        }
        Self {
            start: (self.start + delay % self.period) % self.period,
            width: self.width.saturating_sub(delay),
            ..self
        }
    }

    pub fn edges(self) -> Option<[u64; 2]> {
        (self.width != 0 && self.width < self.period).then_some([
            self.start % self.period,
            (self.start + self.width) % self.period,
        ])
    }
}

/// Running output-stage truth from RM0351 table 199. An N-only output follows
/// OCREF, not its inverse. Complementation and dead time apply when BOTH
/// outputs are enabled. Polarity acts after that output-stage operation.
pub(crate) fn output_pair(
    reference: OutputPulse,
    enabled: [bool; 2],
    active_low: [bool; 2],
    dead_time_cycles: u64,
) -> [Option<OutputPulse>; 2] {
    let both = enabled[0] && enabled[1];
    std::array::from_fn(|side| {
        if !enabled[side] {
            return None;
        }
        let mut pulse = if side == 1 && both {
            reference.inverted()
        } else {
            reference
        };
        if both {
            pulse = pulse.delay_activation(dead_time_cycles);
        }
        if active_low[side] {
            pulse = pulse.inverted();
        }
        Some(pulse)
    })
}

impl super::TimerOutputSnapshot {
    pub(crate) fn phase_cycles(self) -> u64 {
        u64::from(self.counter_ticks) * self.prescaler_divisor
            + u64::from(self.prescaler_phase).min(self.prescaler_divisor - 1)
    }

    fn reference_pulse(self, channel: usize) -> Option<OutputPulse> {
        use super::TimerChannelOutputMode;
        let config = self.channels[channel];
        let pulse = OutputPulse {
            period: self.period_ticks * self.prescaler_divisor,
            start: 0,
            width: config.compare_ticks.min(self.period_ticks) * self.prescaler_divisor,
        };
        match config.mode {
            TimerChannelOutputMode::ForcedInactive => Some(OutputPulse { width: 0, ..pulse }),
            TimerChannelOutputMode::ForcedActive => Some(OutputPulse {
                width: pulse.period,
                ..pulse
            }),
            TimerChannelOutputMode::Pwm1 => Some(pulse),
            TimerChannelOutputMode::Pwm2 => Some(pulse.inverted()),
            TimerChannelOutputMode::Unsupported => None,
        }
    }

    /// No synthetic all-off substitute for an enabled, unsupported OC mode.
    /// Input-capture and disabled channels have no output enables; MOE idle
    /// states are resolved by their output-stage metadata without an OC mode.
    pub(crate) fn motor_gate_states_supported(self) -> bool {
        self.channels[..3].iter().all(|channel| {
            channel.idle.is_some()
                || (!channel.enabled && !channel.complementary_enabled)
                || channel.mode != super::TimerChannelOutputMode::Unsupported
        })
    }

    pub(crate) fn channel_pulses(self, channel: usize) -> [Option<OutputPulse>; 2] {
        let config = self.channels[channel];
        let Some(reference) = self.reference_pulse(channel) else {
            return [None; 2];
        };
        output_pair(
            reference,
            [config.enabled, config.complementary_enabled],
            [config.active_low, config.complementary_active_low],
            self.dead_time_cycles,
        )
    }

    /// Physical gate level after the write-induced transition, on the native
    /// absolute cycle axis. The periodic pulse alone cannot describe a CCR
    /// update halfway through an active interval.
    pub(crate) fn gate_level(self, channel: usize, side: usize, phase: u64, cycle: u64) -> bool {
        let config = self.channels[channel];
        let polarity = [config.active_low, config.complementary_active_low][side];
        if let Some(idle) = config.idle {
            return idle.levels[side].is_some_and(|level| {
                if cycle < idle.until_cycle {
                    polarity
                } else {
                    level
                }
            });
        }
        let Some(pulse) = self.channel_pulses(channel)[side] else {
            return config.inactive_when_disabled[side].unwrap_or(false);
        };
        if let Some(t) = config
            .transition
            .filter(|t| t.side == side && cycle < t.until_cycle)
        {
            return t.active ^ polarity;
        }
        if !self.counter_enabled || self.counter_frozen {
            let reference = self.reference_pulse(channel).expect("supported output");
            return output_pair(
                reference,
                [config.enabled, config.complementary_enabled],
                [config.active_low, config.complementary_active_low],
                0,
            )[side]
                .expect("enabled output")
                .level_at(self.phase_cycles());
        }
        pulse.level_at(phase)
    }
}

impl super::Timer {
    /// Reconcile the dead-time state at a real MMIO write. Unrelated or
    /// repeated writes retain the original deadline; a different reference,
    /// disabled gate or MOE change cancels the pending activation.
    pub(super) fn update_output_transition(
        &mut self,
        before: super::TimerOutputSnapshot,
        now: u64,
    ) {
        use super::TimerOutputTransition;
        // Anchor idle on the write, even when no GPIO pad has been bound.
        // A later motor snapshot must not restart the idle dead-time delay.
        if self.bdtr & (1 << 15) != 0 {
            self.wave.idle_since = [None; 4];
        } else {
            for ch in 0..4 {
                let was_driven = before.channels[ch]
                    .idle
                    .is_some_and(|idle| idle.levels.iter().any(Option::is_some));
                let driven = self.idle_output_levels(ch).iter().any(Option::is_some);
                if before.main_output_enabled
                    || (!was_driven && driven)
                    || self.wave.idle_since[ch].is_none()
                {
                    self.wave.idle_since[ch] = Some(now);
                }
            }
        }
        let after = self.output_snapshot();
        for channel in 0..3 {
            let old = before.channels[channel];
            let new = after.channels[channel];
            let reference = after.reference_pulse(channel);
            if !after.main_output_enabled
                || !new.enabled
                || !new.complementary_enabled
                || reference.is_none()
            {
                self.output_transition[channel] = None;
                continue;
            }
            let reference = reference.unwrap();
            let phase = after.phase_cycles();
            let reference_high = reference.level_at(phase);
            let side = usize::from(!reference_high);
            let old_reference = before
                .reference_pulse(channel)
                .map(|p| p.level_at(before.phase_cycles()));
            let was_active = before.main_output_enabled
                && before.gate_level(channel, side, before.phase_cycles(), now)
                    != [old.active_low, old.complementary_active_low][side]
                && [old.enabled, old.complementary_enabled][side];
            let changed = old_reference != Some(reference_high)
                || before.main_output_enabled != after.main_output_enabled
                || [old.enabled, old.complementary_enabled]
                    != [new.enabled, new.complementary_enabled];
            if changed {
                self.output_transition[channel] = (!was_active).then_some(TimerOutputTransition {
                    side,
                    active: false,
                    until_cycle: now.saturating_add(after.dead_time_cycles),
                });
            } else if let Some(held) = old.transition.filter(|t| t.active && now < t.until_cycle) {
                // A resumed active level follows the current raw falling
                // edge after a CCR/ARR or CNT/kernel-clock geometry change.
                // Unrelated writes retain its original absolute deadline.
                let active = if side == 0 {
                    reference
                } else {
                    reference.inverted()
                };
                let age = (phase % active.period + active.period - active.start) % active.period;
                let end = now.saturating_add(active.width.saturating_sub(age));
                let geometry_changed = before.reference_pulse(channel) != Some(reference)
                    || before.phase_revision != after.phase_revision;
                self.output_transition[channel] = Some(TimerOutputTransition {
                    until_cycle: if geometry_changed {
                        end
                    } else {
                        held.until_cycle
                    },
                    ..held
                });
            } else if before.counter_enabled
                && !after.counter_enabled
                && !was_active
                && old.transition.is_none_or(|t| t.until_cycle <= now)
            {
                // Stopping CNT does not stop the dead-time generator clock.
                let active = if side == 0 {
                    reference
                } else {
                    reference.inverted()
                };
                let age = (phase % active.period + active.period - active.start) % active.period;
                let remaining = after.dead_time_cycles.saturating_sub(age);
                self.output_transition[channel] = Some(TimerOutputTransition {
                    side,
                    active: false,
                    until_cycle: now.saturating_add(remaining),
                });
            } else if !before.counter_enabled && after.counter_enabled && was_active {
                // Restarting CNT without a reference transition must not
                // manufacture another dead-time gap at the old counter phase.
                let active = if side == 0 {
                    reference
                } else {
                    reference.inverted()
                };
                let remaining = active.width.saturating_sub(
                    (phase % active.period + active.period - active.start) % active.period,
                );
                self.output_transition[channel] = Some(TimerOutputTransition {
                    side,
                    active: true,
                    until_cycle: now.saturating_add(remaining),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(width: u64) -> OutputPulse {
        OutputPulse {
            period: 200,
            start: 0,
            width,
        }
    }

    #[test]
    fn n_only_follows_reference_without_dead_time() {
        let pulse = output_pair(reference(80), [false, true], [false; 2], 20);
        assert_eq!(pulse, [None, Some(reference(80))]);
        assert!(pulse[1].unwrap().level_at(0));
        assert!(!pulse[1].unwrap().level_at(80));
        let full = output_pair(reference(200), [false, true], [false; 2], 20);
        assert!(full[1].unwrap().level_at(199));
    }

    #[test]
    fn both_enabled_delay_only_active_edges() {
        let pair = output_pair(reference(80), [true; 2], [false; 2], 2);
        assert_eq!(pair[0].unwrap().edges(), Some([2, 80]));
        assert_eq!(pair[1].unwrap().edges(), Some([82, 0]));
        for cycle in 0..400 {
            assert!(!(pair[0].unwrap().level_at(cycle) && pair[1].unwrap().level_at(cycle)));
        }
    }

    #[test]
    fn narrow_pulses_disappear_and_constants_do_not_gain_periodic_gaps() {
        let pair = output_pair(reference(2), [true; 2], [false; 2], 3);
        assert_eq!(pair[0].unwrap().width, 0);
        assert_eq!(pair[1].unwrap().edges(), Some([5, 0]));
        let full = output_pair(reference(200), [true; 2], [false; 2], 3);
        assert_eq!(full[0].unwrap().width, 200);
        assert_eq!(full[1].unwrap().width, 0);
    }

    #[test]
    fn pwm2_and_each_polarity_use_the_same_native_windows() {
        let pwm2 = reference(80).inverted();
        let pair = output_pair(pwm2, [true; 2], [true, false], 2);
        assert_eq!(pair[0].unwrap().edges(), Some([0, 82]));
        assert_eq!(pair[1].unwrap().edges(), Some([2, 80]));
        let n = output_pair(reference(80), [false, true], [false, true], 2);
        assert_eq!(n[1].unwrap().edges(), Some([80, 0]));
    }

    fn timer() -> super::super::Timer {
        use crate::Peripheral;
        let mut timer = super::super::Timer::new_with_layout(16, true);
        timer.output_lines_arc();
        for (offset, value) in [
            (0x2c, 199),
            (0x34, 80),
            (0x18, 6 << 4),
            (0x20, 5),
            (0x44, 0x8002),
            (0x00, 1),
        ] {
            timer.write_u32(offset, value).unwrap();
        }
        timer
    }

    #[test]
    fn dead_time_uses_kernel_clock_and_ckd_not_counter_prescaler() {
        use crate::Peripheral;
        let mut t = timer();
        t.write_u32(0x28, 4).unwrap(); // Counter period grows by five.
        t.set_kernel_clock_div(2);
        t.write_u32(0x00, 1 | (2 << 8)).unwrap(); // tDTS = four kernel clocks.
        let snapshot = t.output_snapshot();
        assert_eq!(snapshot.prescaler_divisor, 10);
        assert_eq!(snapshot.dead_time_cycles, 16); // DTG 2 × CKD 4 × kernel 2.
        assert_eq!(
            snapshot.channel_pulses(0)[0].unwrap().edges(),
            Some([16, 800])
        );
        assert_eq!(
            snapshot.channel_pulses(0)[1].unwrap().edges(),
            Some([816, 0])
        );
        for cycle in 0..2000 {
            assert_eq!(
                t.output_level(0),
                (16..800).contains(&cycle),
                "OC1 at {cycle}"
            );
            assert_eq!(
                t.output_level(4),
                (816..2000).contains(&cycle),
                "OC1N at {cycle}"
            );
            t.tick();
        }
    }

    #[test]
    fn moe_and_ossi_drive_idle_after_dead_time_and_ossr_owns_unused_side() {
        use crate::peripherals::pad_lines::LineDrive;
        use crate::Peripheral;
        let mut t = timer();
        let lines = t.output_lines_arc();
        t.write_u32(0x20, 4).unwrap();
        t.write_u32(0x44, 0x8802).unwrap(); // MOE + OSSR, only N enabled.
        assert_eq!(lines.mode(0), LineDrive::PushPull);
        assert!(!lines.level(0));
        assert!(lines.level(4));
        t.write_u32(0x04, 1 << 9).unwrap(); // OIS1N high.
        t.write_u32(0x44, 0x0402).unwrap(); // MOE falls, OSSI retains both outputs.
        assert_eq!(lines.mode(4), LineDrive::PushPull);
        assert!(!lines.level(4), "inactive before idle delay");
        t.tick();
        assert!(!lines.level(4));
        t.tick();
        assert!(lines.level(4), "OIS1N after two native cycles");
        t.write_u32(0x44, 2).unwrap(); // OSSI clear releases both pads.
        assert_eq!(lines.mode(0), LineDrive::Input);
        assert_eq!(lines.mode(4), LineDrive::Input);
    }

    #[test]
    fn enable_and_polarity_changes_do_not_wait_for_pwm_boundary() {
        use crate::Peripheral;
        let mut t = timer();
        for _ in 0..10 {
            t.tick();
        }
        assert!(t.output_level(0));
        assert!(!t.output_level(4));
        t.write_u32(0x20, 4).unwrap(); // N-only now follows the current OCREF.
        assert!(t.output_level(4));
        t.write_u32(0x20, 12).unwrap(); // Independent N polarity at this cycle.
        assert!(!t.output_level(4));
    }

    #[test]
    fn live_compare_changes_anchor_dead_time_to_the_write_and_cancel_old_turn_on() {
        use crate::Peripheral;
        let mut t = timer();
        for _ in 0..20 {
            t.tick();
        }
        assert!(t.output_level(0));
        t.write_u32(0x34, 10).unwrap(); // OCREF falls at cycle 20, not at count 10.
        assert!(!t.output_level(0));
        assert!(!t.output_level(4));
        let transition = t.output_snapshot().channels[0].transition.unwrap();
        assert_eq!((transition.side, transition.until_cycle), (1, 22));
        t.tick(); // 21: reverse the reference before N can turn on.
        t.write_u32(0x34, 120).unwrap();
        assert!(!t.output_level(0));
        assert!(!t.output_level(4));
        t.tick(); // 22: cancelled N deadline must not publish an edge.
        assert!(!t.output_level(0));
        assert!(!t.output_level(4));
        t.tick(); // 23: new main activation, delayed from the second write.
        assert!(t.output_level(0));
        assert!(!t.output_level(4));
    }

    #[test]
    fn enable_and_moe_cancel_pending_turn_on_without_overlap() {
        use crate::Peripheral;
        let mut t = timer();
        let lines = t.output_lines_arc();
        for _ in 0..20 {
            t.tick();
        }
        t.write_u32(0x20, 4).unwrap(); // N-only follows OCREF immediately.
        assert!(!lines.drives(0));
        assert!(t.output_level(4));
        t.write_u32(0x20, 5).unwrap(); // Both enabled: N off, main waits until 22.
        assert!(!t.output_level(0));
        assert!(!t.output_level(4));
        t.tick();
        t.write_u32(0x44, 0x0402).unwrap(); // MOE falls with low idle states.
        for _ in 0..5 {
            assert!(!t.output_level(0) && !t.output_level(4));
            t.tick();
        }
        t.write_u32(0x44, 0x8002).unwrap(); // Reactivation gets a fresh deadline.
        assert!(!t.output_level(0) && !t.output_level(4));
        t.tick();
        assert!(!t.output_level(0) && !t.output_level(4));
        t.tick();
        assert!(t.output_level(0) && !t.output_level(4));
    }

    #[test]
    fn stopping_counter_does_not_stop_pending_dead_time_or_restart_an_active_gate() {
        use crate::Peripheral;
        let mut t = timer();
        t.tick(); // Cycle 1, main is still in its two-cycle activation delay.
        t.write_u32(0x00, 0).unwrap();
        assert!(!t.output_level(0));
        t.tick(); // Dead time completes while CNT remains 1.
        assert!(t.output_level(0));
        assert_eq!(t.output_snapshot().counter_ticks, 1);
        for _ in 0..5 {
            t.tick();
        }
        t.write_u32(0x00, 1).unwrap();
        assert!(
            t.output_level(0),
            "CEN rising without an OCREF transition has no new gap"
        );
        for _ in 0..78 {
            t.tick();
            assert!(t.output_level(0));
            assert!(!t.output_level(4));
        }
        t.tick(); // CNT reaches CCR=80.
        assert!(!t.output_level(0));
        assert!(!t.output_level(4));
    }

    #[test]
    fn n_only_compare_write_matches_snapshot_without_preload() {
        use crate::Peripheral;
        let mut t = timer();
        t.write_u32(0x20, 4).unwrap();
        for _ in 0..20 {
            t.tick();
        }
        t.write_u32(0x34, 10).unwrap();
        let snapshot = t.output_snapshot();
        assert!(!t.output_level(4));
        assert!(!snapshot.gate_level(0, 1, snapshot.phase_cycles(), 20));
        assert_eq!(snapshot.channels[0].transition, None);
    }

    #[test]
    fn native_word_and_halfword_writes_do_not_publish_intermediate_byte_references() {
        use crate::Peripheral;
        for halfword in [false, true] {
            let mut t = timer();
            t.write_u32(0x2c, 999).unwrap();
            t.write_u32(0x34, 0x01ff).unwrap();
            for _ in 0..300 {
                t.tick();
            }
            assert!(t.output_level(0));
            // CNT=300 is below both old511 and new512. A byte-split write
            // would transiently compare against256 and invent a turn-off.
            if halfword {
                t.write_u16(0x34, 0x0200).unwrap();
            } else {
                t.write_u32(0x34, 0x0200).unwrap();
            }
            assert!(t.output_level(0), "halfword={halfword}");
            assert!(!t.output_level(4));
        }
    }

    #[test]
    fn idle_snapshot_matches_native_pads_and_does_not_restart_dead_time_on_read() {
        use crate::Peripheral;
        let mut t = timer();
        for _ in 0..20 {
            t.tick();
        }
        t.write_u32(0x04, 1 << 9).unwrap(); // Only OIS1N is high.
        t.write_u32(0x44, 0x0402).unwrap(); // MOE=0, OSSI=1, DT=2.
        for cycle in 20..25 {
            let snapshot = t.output_snapshot();
            assert_eq!(snapshot.channels[0].idle.unwrap().until_cycle, 22);
            for side in 0..2 {
                assert_eq!(
                    snapshot.gate_level(0, side, snapshot.phase_cycles(), cycle),
                    t.output_level(if side == 0 { 0 } else { 4 })
                );
            }
            t.tick();
        }
        assert!(t.output_level(4));
    }

    #[test]
    fn idle_deadline_is_recorded_without_a_pad_binding() {
        use crate::Peripheral;
        let mut t = super::super::Timer::new_with_layout(16, true);
        t.write_u32(0x20, 4).unwrap();
        t.write_u32(0x04, 1 << 9).unwrap();
        t.write_u32(0x44, 0x0402).unwrap();
        for _ in 0..5 {
            t.tick();
        }
        let snapshot = t.output_snapshot();
        assert_eq!(snapshot.channels[0].idle.unwrap().until_cycle, 2);
        assert!(snapshot.gate_level(0, 1, 0, 5));
    }

    #[test]
    fn resumed_active_override_ends_at_a_shortened_compare_edge() {
        use crate::Peripheral;
        let mut t = timer();
        t.write_u32(0x44, 0x8014).unwrap(); // DT=20.
        for _ in 0..5 {
            t.tick();
        }
        t.write_u32(0x00, 0).unwrap(); // Stop CNT=5 while dead time completes.
        for _ in 0..45 {
            t.tick();
        }
        t.write_u32(0x00, 1).unwrap(); // Resume at cycle50, hold active until125.
        assert_eq!(
            t.output_snapshot().channels[0]
                .transition
                .unwrap()
                .until_cycle,
            125
        );
        for _ in 0..10 {
            t.tick();
        }
        t.write_u32(0x34, 20).unwrap(); // At cycle60/CNT15, new falling edge is65.
        assert_eq!(
            t.output_snapshot().channels[0]
                .transition
                .unwrap()
                .until_cycle,
            65
        );
        for cycle in 60..140 {
            assert!(
                !(t.output_level(0) && t.output_level(4)),
                "overlap at {cycle}"
            );
            if cycle == 65 {
                assert!(!t.output_level(0));
            }
            if cycle == 85 {
                assert!(t.output_level(4));
            }
            t.tick();
        }
    }

    #[test]
    fn resumed_active_override_follows_a_widened_compare_without_an_extra_gap() {
        use crate::Peripheral;
        let mut t = timer();
        t.write_u32(0x44, 0x8014).unwrap(); // DT=20.
        t.write_u32(0x34, 8).unwrap();
        for _ in 0..5 {
            t.tick();
        }
        t.write_u32(0x00, 0).unwrap();
        for _ in 0..45 {
            t.tick();
        }
        t.write_u32(0x00, 1).unwrap(); // Cycle50/CNT5: held active until53.
        assert_eq!(
            t.output_snapshot().channels[0]
                .transition
                .unwrap()
                .until_cycle,
            53
        );
        t.tick();
        t.write_u32(0x34, 80).unwrap(); // Cycle51/CNT6: falling edge moves to125.
        assert_eq!(
            t.output_snapshot().channels[0]
                .transition
                .unwrap()
                .until_cycle,
            125
        );
        t.write_u32(0x04, 0).unwrap(); // An unrelated write keeps the deadline.
        assert_eq!(
            t.output_snapshot().channels[0]
                .transition
                .unwrap()
                .until_cycle,
            125
        );
        for cycle in 51..150 {
            let snapshot = t.output_snapshot();
            for side in 0..2 {
                assert_eq!(
                    t.output_level(if side == 0 { 0 } else { 4 }),
                    snapshot.gate_level(0, side, snapshot.phase_cycles(), cycle),
                    "native/snapshot mismatch at {cycle}, side{side}"
                );
            }
            assert!(
                !(t.output_level(0) && t.output_level(4)),
                "overlap at {cycle}"
            );
            assert_eq!(t.output_level(0), cycle < 125, "main at {cycle}");
            assert_eq!(t.output_level(4), cycle >= 145, "complement at {cycle}");
            t.tick();
        }
    }

    fn assert_native_snapshot_parity(t: &mut super::super::Timer, first_cycle: u64, count: u64) {
        use crate::Peripheral;
        for cycle in first_cycle..first_cycle + count {
            let snapshot = t.output_snapshot();
            for side in 0..2 {
                assert_eq!(
                    t.output_level(if side == 0 { 0 } else { 4 }),
                    snapshot.gate_level(0, side, snapshot.phase_cycles(), cycle),
                    "native/snapshot mismatch at {cycle}, side{side}"
                );
            }
            t.tick();
        }
    }

    #[test]
    fn native_phase_anchor_survives_prescaler_and_cnt_ahead_of_absolute_time() {
        use crate::Peripheral;
        let mut t = timer();
        for _ in 0..100 {
            t.tick();
        }
        t.write_u32(0x28, 3).unwrap(); // Phase400 of800 at absolute cycle100.
        assert_native_snapshot_parity(&mut t, 100, 200);
        t.set_kernel_clock_div(2);
        assert_native_snapshot_parity(&mut t, 300, 100);
        let mut ahead = timer();
        ahead.write_u32(0x24, 100).unwrap(); // Native counter phase exceeds cycle0.
        assert_native_snapshot_parity(&mut ahead, 0, 220);
    }

    #[test]
    fn unsupported_running_outputs_are_explicit_but_idle_and_disabled_states_are_resolvable() {
        use crate::Peripheral;
        let mut t = timer();
        t.write_u32(0x18, 0).unwrap(); // Enabled frozen OC mode is not a PWM reference.
        assert!(!t.output_snapshot().motor_gate_states_supported());
        t.write_u32(0x20, 0).unwrap();
        assert!(t.output_snapshot().motor_gate_states_supported());
        t.write_u32(0x20, 5).unwrap();
        t.write_u32(0x44, 0x0402).unwrap();
        assert!(t.output_snapshot().motor_gate_states_supported());
    }
}
