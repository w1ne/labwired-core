//! Guards for the walk-deletion decision (`SystemManifest.walk_deleted:
//! Option<bool>` → `SystemBus.legacy_walk_disabled`). The field is only
//! *consulted* under the `event-scheduler` feature, but it is *plumbed*
//! unconditionally, so these tests run in both flag states.
//!
//! Semantics:
//!   Some(true)  → force walk deleted (hand opt-in / escape hatch)
//!   Some(false) → pin the walk ON, overriding auto-derivation
//!   None        → auto-derive (conservative: delete iff EVERY peripheral is
//!                 provably walk-independent for all firmware states)

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
#[allow(unused_imports)]
use labwired_core::bus::RECOMMENDED_TICK_INTERVAL;
#[allow(unused_imports)]
use labwired_core::Peripheral;
use std::path::PathBuf;

fn root(rel: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../..");
    p.push(rel);
    p
}

fn nokia() -> (ChipDescriptor, SystemManifest) {
    let chip = ChipDescriptor::from_file(root("configs/chips/stm32l476.yaml"))
        .expect("load stm32l476 chip");
    let manifest = SystemManifest::from_file(root("examples/nokia5110-invaders-lab/system.yaml"))
        .expect("load nokia manifest");
    (chip, manifest)
}

/// The Nokia 5110 lab carries an explicit `walk_deleted: true` (its firmware was
/// hand-verified byte-identical walk-free), and that opt-in must reach the bus.
#[test]
fn nokia_explicit_flag_reaches_bus() {
    let (chip, manifest) = nokia();
    assert_eq!(
        manifest.walk_deleted,
        Some(true),
        "the Nokia lab manifest must keep walk_deleted: true"
    );
    let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    assert!(
        bus.legacy_walk_disabled,
        "explicit walk_deleted: true must set bus.legacy_walk_disabled"
    );
}

/// Auto-derivation with the Nokia lab's explicit flag REMOVED (`None`).
///
/// The stm32l476 descriptor instantiates native timers / SysTick / ADC / DMA /
/// EXTI / I2C / CAN. The walk-free campaign migrated every one of them to the
/// event scheduler (each proven byte-identical walk-free for ALL firmware states
/// by its own walk-vs-scheduler differential), so the model-level predicate
/// `derive_walk_deletable` now legitimately flips:
///
///   * `event-scheduler` builds: every peripheral reports `uses_scheduler()`
///     (or `!needs_legacy_walk()`), the forcing set is empty, and the bus
///     auto-derives walk-deletion with no hand flag — the explicit
///     `walk_deleted: true` is now redundant, not load-bearing.
///   * featureless builds: the scheduler does not exist, so the migrated models
///     honestly stay on the walk and derivation keeps it on.
#[test]
fn nokia_without_flag_auto_derivation_tracks_scheduler_feature() {
    let (chip, mut manifest) = nokia();
    manifest.walk_deleted = None; // simulate removing the yaml line
    let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    #[cfg(feature = "event-scheduler")]
    assert!(
        bus.legacy_walk_disabled,
        "every L476 peripheral is now event-scheduled → derivation deletes the walk"
    );
    #[cfg(not(feature = "event-scheduler"))]
    assert!(
        !bus.legacy_walk_disabled,
        "featureless build has no scheduler → migrated models stay on the walk"
    );
}

/// An explicit `Some(false)` pins the walk ON — the escape hatch.
#[test]
fn explicit_false_pins_walk_on() {
    let (chip, mut manifest) = nokia();
    manifest.walk_deleted = Some(false);
    let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    assert!(
        !bus.legacy_walk_disabled,
        "walk_deleted: false must pin the walk on"
    );
}

/// The headline campaign result: a real **stm32f103** lab bus, built exactly the
/// way the run path does (`from_config` + `configure_cortex_m`) with any hand
/// `walk_deleted` flag stripped, auto-derives walk-deletion under
/// `event-scheduler` — the whole f103 peripheral set (i2c/exti/adc migrated,
/// rtc/afio/bxcan no-op-gated, timers/dma/systick from earlier batches) is now
/// walk-free, so the browser fast path lifts recommended_tick_interval 1→512 and
/// engages idle fast-forward. Featureless builds honestly keep the walk.
#[test]
fn f103_lab_bus_flips_walk_deletable_under_scheduler() {
    let chip = ChipDescriptor::from_file(root("configs/chips/stm32f103.yaml"))
        .expect("load stm32f103 chip");
    let mut manifest = SystemManifest::from_file(root("examples/ssd1306-hello-lab/system.yaml"))
        .expect("load f103 i2c lab manifest");
    manifest.walk_deleted = None; // derive from the models, not the manifest hatch
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build f103 bus");
    let _ = labwired_core::system::cortex_m::configure_cortex_m(&mut bus);

    let forcing: Vec<&str> = bus
        .peripherals
        .iter()
        .filter(|p| p.dev.needs_legacy_walk() && !p.dev.uses_scheduler())
        .map(|p| p.name.as_str())
        .collect();

    #[cfg(feature = "event-scheduler")]
    {
        assert!(
            forcing.is_empty(),
            "f103 lab bus still has walk-forcing peripherals: {forcing:?}"
        );
        assert_eq!(bus.max_safe_tick_interval(), RECOMMENDED_TICK_INTERVAL);
    }
    #[cfg(not(feature = "event-scheduler"))]
    {
        // Featureless: the migrated models stay on the walk (no scheduler).
        assert!(!forcing.is_empty());
        assert_eq!(bus.max_safe_tick_interval(), 1);
    }
}

/// A manifest without the field parses to `None` (auto-derive), not a hard-coded
/// off.
#[test]
fn walk_deleted_defaults_to_none() {
    let yaml = "name: t\nchip: x\n";
    let manifest: SystemManifest = serde_yaml::from_str(yaml).expect("parse minimal manifest");
    assert_eq!(
        manifest.walk_deleted, None,
        "walk_deleted must default to None (auto-derive)"
    );
}
