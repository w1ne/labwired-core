// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Retained drivers for an explicitly pulled-up, open-drain software net.
//!
//! Opted-in declarative device outputs use this resolver when routed to pads.
//! It does not model a floating net, analog voltages, propagation delay, or
//! push-pull contention.

use std::collections::{BTreeMap, BTreeSet};

/// Identity of one output on one resident device instance.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct OpenDrainDriver {
    pub device_id: String,
    pub role: String,
}

/// A net with an explicitly declared pull-up and retained driver states.
/// Each registered driver initially releases the line. `false` drives low;
/// `true` releases it, matching the existing Boolean output-level convention.
#[derive(Debug, Default)]
pub struct PulledUpOpenDrain {
    drivers: BTreeMap<OpenDrainDriver, bool>,
}

impl PulledUpOpenDrain {
    /// Declare a pulled-up net; floating nets require a separate model.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a released driver. Duplicate identities are rejected.
    pub fn register(&mut self, driver: OpenDrainDriver) -> Result<(), &'static str> {
        if self.drivers.contains_key(&driver) {
            return Err("duplicate open-drain driver");
        }
        self.drivers.insert(driver, true);
        Ok(())
    }

    /// Update only the named driver and return the resolved pad level.
    pub fn drive(&mut self, driver: &OpenDrainDriver, level: bool) -> Result<bool, &'static str> {
        let state = self
            .drivers
            .get_mut(driver)
            .ok_or("unknown open-drain driver")?;
        *state = level;
        Ok(self.level())
    }

    /// Release one driver without changing the other drivers.
    pub fn release(&mut self, driver: &OpenDrainDriver) -> Result<bool, &'static str> {
        self.drive(driver, true)
    }

    /// Resolve the net against its declared pull-up.
    pub fn level(&self) -> bool {
        self.drivers.values().all(|released| *released)
    }
}

/// Explicitly pulled-up nets indexed by the sampled GPIO register and bit.
///
/// Registration is separate from ordinary push-pull routing. Callers must
/// reject a legacy output already assigned to a pad before registering it.
#[derive(Debug, Default)]
pub struct OpenDrainNets {
    nets: BTreeMap<(u64, u8), PulledUpOpenDrain>,
    inverted: BTreeMap<(u64, u8), BTreeSet<OpenDrainDriver>>,
}

impl OpenDrainNets {
    /// Register one driver on a pulled-up pad. A legacy driver on the same
    /// pad is an error; this resolver does not model mixed-mode contention.
    pub fn register(
        &mut self,
        pad: (u64, u8),
        driver: OpenDrainDriver,
        has_legacy_driver: bool,
    ) -> Result<(), &'static str> {
        if has_legacy_driver {
            return Err("open-drain pad also has a legacy driver");
        }
        self.nets.entry(pad).or_default().register(driver)
    }

    /// Register a driver through an explicit digital inverting open-collector
    /// stage: a high source pulls the shared net low, and low releases it.
    /// The source is initially assumed low. No analog transistor behavior is
    /// modeled. Registration failures leave the net and adapter unchanged.
    pub fn register_inverted(
        &mut self,
        pad: (u64, u8),
        driver: OpenDrainDriver,
        has_legacy_driver: bool,
    ) -> Result<(), &'static str> {
        self.register(pad, driver.clone(), has_legacy_driver)?;
        self.inverted.entry(pad).or_default().insert(driver);
        Ok(())
    }

    /// Whether this pad has been explicitly configured as a pulled-up net.
    pub fn contains_pad(&self, pad: (u64, u8)) -> bool {
        self.nets.contains_key(&pad)
    }

    /// Return the current resolved level of an explicitly registered pad.
    pub fn level(&self, pad: (u64, u8)) -> Option<bool> {
        self.nets.get(&pad).map(PulledUpOpenDrain::level)
    }

    /// Update one registered driver; unknown pads and drivers are rejected.
    pub fn drive(
        &mut self,
        pad: (u64, u8),
        driver: &OpenDrainDriver,
        level: bool,
    ) -> Result<bool, &'static str> {
        let inverted = self
            .inverted
            .get(&pad)
            .is_some_and(|drivers| drivers.contains(driver));
        self.nets
            .get_mut(&pad)
            .ok_or("unknown open-drain pad")?
            .drive(driver, if inverted { !level } else { level })
    }

    /// Snapshot resolved pad levels, for applying a pull-up after GPIO clocks
    /// or reset state change without manufacturing a device transition.
    pub fn levels(&self) -> impl Iterator<Item = ((u64, u8), bool)> + '_ {
        self.nets.iter().map(|(&pad, net)| (pad, net.level()))
    }

    /// Release every registered driver on every net and return the affected
    /// pads, so callers can apply the pull-up to their GPIO input registers.
    pub fn release_all(&mut self) -> impl Iterator<Item = (u64, u8)> + '_ {
        for net in self.nets.values_mut() {
            for released in net.drivers.values_mut() {
                *released = true;
            }
        }
        self.nets.keys().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driver(device: &str, role: &str) -> OpenDrainDriver {
        OpenDrainDriver {
            device_id: device.into(),
            role: role.into(),
        }
    }

    #[test]
    fn either_retained_driver_keeps_the_pad_low_in_both_orders() {
        for reverse in [false, true] {
            let (a, b) = (driver("sensor-a", "irq"), driver("sensor-b", "irq"));
            let mut net = PulledUpOpenDrain::new();
            net.register(a.clone()).unwrap();
            net.register(b.clone()).unwrap();
            assert!(net.level());
            let (first, second) = if reverse { (&b, &a) } else { (&a, &b) };
            assert!(!net.drive(first, false).unwrap());
            assert!(!net.drive(second, false).unwrap());
            assert!(!net.release(first).unwrap());
            assert!(net.release(second).unwrap());
        }
    }

    #[test]
    fn duplicate_and_unknown_drivers_cannot_change_the_net() {
        let a = driver("sensor", "irq");
        let unknown = driver("sensor", "other");
        let mut net = PulledUpOpenDrain::new();
        net.register(a.clone()).unwrap();
        net.drive(&a, false).unwrap();
        assert!(net.register(a.clone()).is_err());
        assert!(net.drive(&unknown, true).is_err());
        assert!(!net.level());
        assert!(net.release(&a).unwrap());
    }

    #[test]
    fn separate_nets_do_not_affect_one_another() {
        let a = driver("part", "irq");
        let mut first = PulledUpOpenDrain::new();
        let mut second = PulledUpOpenDrain::new();
        first.register(a.clone()).unwrap();
        second.register(a.clone()).unwrap();
        first.drive(&a, false).unwrap();
        assert!(!first.level());
        assert!(second.level());
    }
    #[test]
    fn one_source_resolves_independently_on_direct_and_inverted_pads() {
        let source = driver("sensor", "irq");
        let mut nets = OpenDrainNets::default();
        nets.register_inverted((10, 2), source.clone(), false)
            .unwrap();
        nets.register((11, 3), source.clone(), false).unwrap();
        assert_eq!(nets.level((10, 2)), Some(true));
        assert_eq!(nets.level((11, 3)), Some(true));

        assert!(!nets.drive((10, 2), &source, true).unwrap());
        assert!(nets.drive((11, 3), &source, true).unwrap());
        assert_eq!(nets.level((10, 2)), Some(false));
        assert_eq!(nets.level((11, 3)), Some(true));

        assert!(nets.drive((10, 2), &source, false).unwrap());
        assert!(!nets.drive((11, 3), &source, false).unwrap());
        assert_eq!(nets.level((10, 2)), Some(true));
        assert_eq!(nets.level((11, 3)), Some(false));
    }

    #[test]
    fn pad_registry_rejects_mixed_and_unknown_routes_and_releases_all() {
        let a = driver("part-a", "irq");
        let b = driver("part-b", "irq");
        let mut nets = OpenDrainNets::default();
        assert!(nets.register((10, 2), a.clone(), true).is_err());
        assert!(!nets.contains_pad((10, 2)));
        nets.register((10, 2), a.clone(), false).unwrap();
        nets.register((10, 2), b.clone(), false).unwrap();
        nets.register((11, 3), a.clone(), false).unwrap();
        assert!(!nets.drive((10, 2), &a, false).unwrap());
        assert!(!nets.drive((11, 3), &a, false).unwrap());
        assert!(nets.drive((12, 3), &a, true).is_err());
        assert!(nets
            .drive((10, 2), &driver("unknown", "irq"), true)
            .is_err());
        assert_eq!(
            nets.release_all().collect::<Vec<_>>(),
            vec![(10, 2), (11, 3)]
        );
        assert!(nets.drive((10, 2), &b, true).unwrap());
        assert!(nets.drive((11, 3), &a, true).unwrap());
    }
    #[test]
    fn inverting_collector_and_direct_driver_share_one_retained_net() {
        let transistor = driver("sensor", "irq");
        let direct = driver("interface", "irq");
        let mut nets = OpenDrainNets::default();
        nets.register_inverted((10, 2), transistor.clone(), false)
            .unwrap();
        nets.register((10, 2), direct.clone(), false).unwrap();
        assert_eq!(nets.level((10, 2)), Some(true));
        assert!(!nets.drive((10, 2), &transistor, true).unwrap());
        assert!(!nets.drive((10, 2), &direct, false).unwrap());
        assert!(!nets.drive((10, 2), &transistor, false).unwrap());
        assert!(nets.drive((10, 2), &direct, true).unwrap());
        assert!(!nets.drive((10, 2), &direct, false).unwrap());
        assert!(!nets.drive((10, 2), &transistor, true).unwrap());
        assert!(!nets.drive((10, 2), &direct, true).unwrap());
        assert!(nets.drive((10, 2), &transistor, false).unwrap());
        assert!(nets
            .register_inverted((10, 2), transistor.clone(), false)
            .is_err());
        assert!(nets.drive((10, 2), &transistor, false).unwrap());
    }
}
