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

/// Auto-derivation is CONSERVATIVE at the real-config level: with the Nokia lab's
/// explicit flag REMOVED (`None`), the bus does NOT derive walk-deletion, because
/// the stm32l476 descriptor instantiates native timers / SysTick / ADC / DMA /
/// EXTI / CAN whose `tick()` does real work once firmware arms them. Their
/// byte-identity walk-free is a *firmware-specific* fact (this firmware never arms
/// them) that no config-time predicate can prove — so the walk stays on and the
/// explicit flag remains load-bearing.
#[test]
fn nokia_without_flag_does_not_auto_derive_deletion() {
    let (chip, mut manifest) = nokia();
    manifest.walk_deleted = None; // simulate removing the yaml line
    let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    assert!(
        !bus.legacy_walk_disabled,
        "conservative derivation must keep the walk (native timers are armable)"
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
