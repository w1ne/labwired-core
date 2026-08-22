//! Measurement-only probe: which ESP32-S3 peripherals keep the per-cycle legacy
//! walk alive, and therefore keep every S3 lab off the walk-free fast path that
//! the C3 has had since its migration.
//!
//! Not a gate. It prints a named list so the migration has a work-list instead
//! of a peripheral count.

use labwired_core::bus::SystemBus;
use labwired_core::system::xtensa::{configure_xtensa_esp32s3, Esp32s3Opts};

#[test]
#[ignore = "diagnostic; run with --release --ignored -- --nocapture"]
fn esp32s3_walk_blockers() {
    let mut bus = SystemBus::new();
    let _wiring = configure_xtensa_esp32s3(&mut bus, &Esp32s3Opts::default());
    bus.refresh_peripheral_index();

    let mut blockers: Vec<&str> = Vec::new();
    let mut sched: Vec<&str> = Vec::new();
    let mut inert: Vec<&str> = Vec::new();
    for p in &bus.peripherals {
        if p.dev.uses_scheduler() {
            sched.push(p.name.as_str());
        } else if !p.dev.needs_legacy_walk() {
            inert.push(p.name.as_str());
        } else {
            blockers.push(p.name.as_str());
        }
    }

    let legacy = bus.legacy_tick_entry_descriptors();
    println!("--- ESP32-S3 walk probe ---");
    println!("peripherals total        : {}", bus.peripherals.len());
    println!("legacy tick entries      : {}", legacy.len());
    println!("scheduler-driven         : {} {:?}", sched.len(), sched);
    println!("walk-independent (inert) : {} {:?}", inert.len(), inert);
    println!(
        "WALK BLOCKERS            : {} {:?}",
        blockers.len(),
        blockers
    );
    println!(
        "esp32s3 matrix routing   : {}",
        bus.irq_fabric.esp32s3.routing
    );
    println!("legacy_walk_disabled     : {}", bus.legacy_walk_disabled);
}
