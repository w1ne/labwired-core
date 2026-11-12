// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! The chip's supply supervisor: power-on, power-down and brown-out reset
//! driven by the VDD a board's circuit actually delivers.
//!
//! Without a routed supply the MCU runs from an ideal rail at its descriptor's
//! `io_voltage_v`, exactly as it always has, and [`crate::fidelity`] records
//! the approximation as [`crate::fidelity::UNPOWERED_RAIL_ASSUMED`]. With one —
//! an analog model output routed to `board.power.vdd_volts` — the supervisor
//! holds the core in reset until VDD reaches the release threshold, resets it
//! again when VDD falls through the power-down or active brown-out threshold,
//! and tells the chip's reset-cause register why (see
//! [`crate::Peripheral::on_supply_reset`]).
//!
//! Thresholds come from the chip descriptor's `supply_monitor:`
//! ([`labwired_config::SupplyMonitor`]); a chip without one refuses the route.
//!
//! What a supply reset does to the machine is what `SYSRESETREQ` already does
//! — the CPU restarts through its reset vector — plus the reset-cause bits.
//! Peripheral registers other than the reset cause, and RAM, keep their
//! values; on silicon a power-on reset loses both.

use labwired_config::SupplyMonitor;

/// Why the supervisor last released the core from reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupplyResetCause {
    /// VDD had fallen below the power-down threshold (or had never been up):
    /// a power-on reset.
    PowerOn,
    /// VDD fell below the active brown-out level but not below power-down.
    BrownOut,
}

impl SupplyResetCause {
    /// The word the browser and the CLI print.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PowerOn => "power_on",
            Self::BrownOut => "brown_out",
        }
    }
}

/// A snapshot of the supervisor, for the browser and for tests.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SupplyStatus {
    /// A circuit drives VDD (`board.power.vdd_volts` is routed). `false` means
    /// the ideal rail at `io_voltage_v` is assumed.
    pub routed: bool,
    /// The last VDD the circuit delivered, volts. `None` before the first
    /// model step, and always on an unrouted board.
    pub vdd_volts: Option<f64>,
    /// The core is being held in reset by its supply right now.
    pub held_in_reset: bool,
    /// Power-on resets the supervisor has released the core from.
    pub power_on_resets: u32,
    /// Brown-out resets the supervisor has released the core from.
    pub brown_out_resets: u32,
    /// The cause of the last release, if any.
    pub last_cause: Option<SupplyResetCause>,
    /// VDD the core needs, rising, to leave reset. `None` on a chip with no
    /// `supply_monitor`.
    pub release_volts: Option<f64>,
    /// VDD below which a running core is reset (the higher of power-down and
    /// the active brown-out level). `None` on a chip with no `supply_monitor`.
    pub reset_volts: Option<f64>,
    /// The active brown-out level's name, `None` when brown-out reset is off.
    pub bor_level: Option<String>,
}

/// The supervisor state carried on the bus.
#[derive(Debug, Clone, Default)]
pub struct SupplySupervisor {
    monitor: Option<SupplyMonitor>,
    routed: bool,
    vdd: Option<f64>,
    held: bool,
    /// VDD has been below power-down since the last release (or has never
    /// been up), so the next release is a power-on reset.
    lost_power: bool,
    pending_release: Option<SupplyResetCause>,
    power_on_resets: u32,
    brown_out_resets: u32,
    last_cause: Option<SupplyResetCause>,
}

impl SupplySupervisor {
    /// A supervisor with `monitor`'s thresholds, unrouted: the ideal rail.
    pub fn new(monitor: Option<SupplyMonitor>) -> Self {
        Self {
            monitor,
            ..Self::default()
        }
    }

    /// Whether the chip descriptor declares thresholds at all.
    pub fn has_monitor(&self) -> bool {
        self.monitor.is_some()
    }

    /// Whether a circuit drives VDD.
    pub fn is_routed(&self) -> bool {
        self.routed
    }

    /// The latest circuit-observed supply, absent before its first observation.
    pub fn vdd_volts(&self) -> Option<f64> {
        self.vdd
    }

    /// A circuit now drives VDD. The core starts unpowered and held in reset;
    /// the first VDD the circuit delivers decides when it comes out. Calling
    /// it again does nothing.
    pub fn attach(&mut self) {
        if self.routed || self.monitor.is_none() {
            return;
        }
        self.routed = true;
        self.held = true;
        self.lost_power = true;
    }

    /// The core is held in reset by its supply.
    pub fn is_held(&self) -> bool {
        self.held
    }

    /// Feed the VDD the circuit delivered. `NaN` reads as no supply at all.
    pub fn set_vdd(&mut self, volts: f64) {
        let Some(monitor) = self.monitor.as_ref() else {
            return;
        };
        if !self.routed {
            return;
        }
        let volts = if volts.is_nan() { 0.0 } else { volts };
        self.vdd = Some(volts);
        if volts < monitor.pdr_falling_v {
            self.lost_power = true;
        }
        if !self.held {
            let below_bor = monitor
                .active_bor()
                .is_some_and(|level| volts < level.falling_v);
            if volts < monitor.pdr_falling_v || below_bor {
                self.held = true;
            }
        } else if volts >= monitor.release_v() {
            let cause = if self.lost_power {
                SupplyResetCause::PowerOn
            } else {
                SupplyResetCause::BrownOut
            };
            self.held = false;
            self.lost_power = false;
            self.pending_release = Some(cause);
            self.last_cause = Some(cause);
            match cause {
                SupplyResetCause::PowerOn => self.power_on_resets += 1,
                SupplyResetCause::BrownOut => self.brown_out_resets += 1,
            }
        }
    }

    /// The release the machine has not acted on yet: take it once, reset the
    /// core, and set the reset cause.
    pub fn take_release(&mut self) -> Option<SupplyResetCause> {
        self.pending_release.take()
    }

    /// Everything the browser shows about the supply.
    pub fn status(&self) -> SupplyStatus {
        let reset_volts = self.monitor.as_ref().map(|monitor| {
            monitor.active_bor().map_or(monitor.pdr_falling_v, |level| {
                level.falling_v.max(monitor.pdr_falling_v)
            })
        });
        SupplyStatus {
            routed: self.routed,
            vdd_volts: self.vdd,
            held_in_reset: self.held,
            power_on_resets: self.power_on_resets,
            brown_out_resets: self.brown_out_resets,
            last_cause: self.last_cause,
            release_volts: self.monitor.as_ref().map(SupplyMonitor::release_v),
            reset_volts,
            bor_level: self.monitor.as_ref().and_then(|m| m.bor_level.clone()),
        }
    }
}

#[derive(Debug)]
pub(crate) enum RailBatchError {
    Device(String),
    Mcu(String),
}

/// Admit a raw rail observation without changing its value. A declared maximum
/// permits only signed roundoff within one relative f64 epsilon of zero.
/// Receivers without that explicit maximum retain the nonnegative contract.
/// Operating-range qualification is separate, so admitted negative values
/// remain unavailable to the device.
pub(crate) fn rail_observation_is_admissible(
    voltage_v: f64,
    declared_maximum_v: Option<f64>,
) -> bool {
    let minimum = match declared_maximum_v {
        Some(maximum) if maximum.is_finite() && maximum >= 0.0 => -f64::EPSILON * maximum,
        None => 0.0,
        Some(_) => return false,
    };
    voltage_v.is_finite() && voltage_v >= minimum
}

#[cfg(test)]
mod tests {
    use super::*;
    use labwired_config::BrownOutLevel;

    /// STM32F401-like: POR 1.72 V rising, PDR 1.68 V falling, BOR level 3
    /// at 2.83 V falling / 2.92 V rising.
    fn f401(bor: bool) -> SupplySupervisor {
        let mut levels = std::collections::BTreeMap::new();
        levels.insert(
            "bor3".to_string(),
            BrownOutLevel {
                falling_v: 2.83,
                rising_v: 2.92,
            },
        );
        SupplySupervisor::new(Some(SupplyMonitor {
            por_rising_v: 1.72,
            pdr_falling_v: 1.68,
            bor_levels: levels,
            bor_level: bor.then(|| "bor3".to_string()),
        }))
    }

    #[test]
    fn unrouted_supervisor_never_holds() {
        let mut supply = f401(true);
        supply.set_vdd(0.0);
        assert!(!supply.is_held());
        assert_eq!(supply.take_release(), None);
        assert!(!supply.status().routed);
    }

    #[test]
    fn routed_core_is_held_until_por_then_released_as_power_on() {
        let mut supply = f401(false);
        supply.attach();
        assert!(supply.is_held());
        for volts in [0.0, 1.0, 1.70, 1.719] {
            supply.set_vdd(volts);
            assert!(supply.is_held(), "{volts} V is below the 1.72 V release");
        }
        supply.set_vdd(1.72);
        assert!(!supply.is_held());
        assert_eq!(supply.take_release(), Some(SupplyResetCause::PowerOn));
        assert_eq!(supply.take_release(), None, "taken once");
    }

    #[test]
    fn pdr_hysteresis_and_power_on_cause() {
        let mut supply = f401(false);
        supply.attach();
        supply.set_vdd(3.3);
        supply.take_release();
        // Inside the 1.68–1.72 V window: keeps running.
        supply.set_vdd(1.70);
        assert!(!supply.is_held());
        supply.set_vdd(1.67);
        assert!(supply.is_held());
        // Back into the window: still held (hysteresis).
        supply.set_vdd(1.70);
        assert!(supply.is_held());
        supply.set_vdd(3.3);
        assert_eq!(supply.take_release(), Some(SupplyResetCause::PowerOn));
        assert_eq!(supply.status().power_on_resets, 2);
    }

    #[test]
    fn brown_out_above_pdr_is_a_brown_out_reset() {
        let mut supply = f401(true);
        supply.attach();
        supply.set_vdd(3.3);
        assert_eq!(supply.take_release(), Some(SupplyResetCause::PowerOn));
        supply.set_vdd(2.84);
        assert!(!supply.is_held(), "above the 2.83 V BOR");
        supply.set_vdd(2.5);
        assert!(supply.is_held());
        // 2.90 V is above the falling threshold but below the rising one.
        supply.set_vdd(2.90);
        assert!(supply.is_held(), "BOR hysteresis: 2.90 V < 2.92 V");
        supply.set_vdd(3.3);
        assert_eq!(supply.take_release(), Some(SupplyResetCause::BrownOut));
        let status = supply.status();
        assert_eq!((status.power_on_resets, status.brown_out_resets), (1, 1));
        assert_eq!(status.reset_volts, Some(2.83));
        assert_eq!(status.release_volts, Some(2.92));
    }

    #[test]
    fn nan_reads_as_no_supply() {
        let mut supply = f401(false);
        supply.attach();
        supply.set_vdd(3.3);
        supply.take_release();
        supply.set_vdd(f64::NAN);
        assert!(supply.is_held());
        assert_eq!(supply.status().vdd_volts, Some(0.0));
    }

    #[test]
    fn attach_without_a_monitor_is_refused_silently() {
        let mut supply = SupplySupervisor::new(None);
        supply.attach();
        assert!(!supply.is_routed() && !supply.is_held());
    }
}
