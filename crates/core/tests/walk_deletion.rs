//! Guards for the per-config walk-deletion opt-in (`SystemManifest.walk_deleted`
//! → `SystemBus.legacy_walk_disabled`). The flag is only *consulted* under the
//! `event-scheduler` feature, but it is *plumbed* unconditionally, so these
//! tests run in both flag states.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use std::path::PathBuf;

fn root(rel: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("../..");
    p.push(rel);
    p
}

/// The Nokia 5110 lab opts into walk-deletion (its firmware was verified
/// byte-identical walk-free with SPI event-migrated), and that opt-in must
/// reach the bus.
#[test]
fn nokia_walk_deleted_flag_reaches_bus() {
    let chip = ChipDescriptor::from_file(&root("configs/chips/stm32l476.yaml"))
        .expect("load stm32l476 chip");
    let manifest = SystemManifest::from_file(&root("examples/nokia5110-invaders-lab/system.yaml"))
        .expect("load nokia manifest");
    assert!(
        manifest.walk_deleted,
        "the Nokia lab manifest must keep walk_deleted: true"
    );
    let bus = SystemBus::from_config(&chip, &manifest).expect("build bus");
    assert!(
        bus.legacy_walk_disabled,
        "walk_deleted manifest flag must set bus.legacy_walk_disabled"
    );
}

/// Walk-deletion is strictly opt-in: a manifest without the field keeps the
/// per-cycle walk (the safe default).
#[test]
fn walk_deletion_is_off_by_default() {
    let yaml = "name: t\nchip: x\n";
    let manifest: SystemManifest = serde_yaml::from_str(yaml).expect("parse minimal manifest");
    assert!(
        !manifest.walk_deleted,
        "walk_deleted must default to false (walk preserved)"
    );
}
