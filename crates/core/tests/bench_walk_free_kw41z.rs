// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Manual perf probe (`--ignored --release --nocapture`) for the FRDM-KW41Z
//! board flip unlocked by the batch-B4 shared-Kinetis-I2C migration: the same
//! tight instruction loop on the real kw41z peripheral bus, timed with the
//! per-cycle walk ON at tick interval 1 (i2c1 pinned legacy — the pre-migration
//! state) vs the walk DELETED at the derived batched interval 64 (post-
//! migration). Reports native MIPS for both; not a correctness gate.

#![cfg(feature = "event-scheduler")]

use labwired_core::bus::SystemBus;
use labwired_core::cpu::CortexM;
use labwired_core::peripherals::i2c::I2c;
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;
use labwired_core::{DebugControl, Machine};
use std::time::Instant;

fn kw41z_machine_parts() -> (SystemBus, CortexM) {
    use labwired_config::{ChipDescriptor, SystemManifest};
    use std::path::PathBuf;
    let system_path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../configs/systems/frdm-kw41z.yaml");
    let mut manifest = SystemManifest::from_file(&system_path).expect("load frdm-kw41z manifest");
    manifest.walk_deleted = None;
    let chip_path = system_path.parent().unwrap().join(&manifest.chip);
    let chip = ChipDescriptor::from_file(&chip_path).expect("load mkw41z4 chip");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build frdm-kw41z bus");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    (bus, cpu)
}

const LOOP_ADDR: u32 = 0x1FFF_8400;
const SP: u32 = 0x1FFF_9000;

/// `adds r0,#1 ; b .-2` — a tight two-instruction spin so the measurement is
/// dominated by per-instruction orchestration (the walk vs the deleted walk).
fn load_loop(bus: &mut SystemBus) {
    bus.write_u16(LOOP_ADDR as u64, 0x3001).unwrap();
    bus.write_u16(LOOP_ADDR as u64 + 2, 0xE7FD).unwrap();
}

fn time_run(scheduler_flip: bool, interval: u32, steps: u64) -> f64 {
    let (mut bus, cpu) = kw41z_machine_parts();
    load_loop(&mut bus);
    if !scheduler_flip {
        // Pre-migration state: pin i2c1 back onto the legacy walk and re-enable
        // the per-cycle walk the `from_config` derivation just deleted.
        if let Some(idx) = bus.find_peripheral_index_by_name("i2c1") {
            if let Some(i2c) = bus.peripherals[idx]
                .dev
                .as_any_mut()
                .and_then(|a| a.downcast_mut::<I2c>())
            {
                i2c.force_legacy_walk();
            }
        }
        bus.legacy_walk_disabled = false;
    }
    let mut m = Machine::new(cpu, bus);
    m.config.peripheral_tick_interval = interval;
    m.bus.config.peripheral_tick_interval = interval;
    m.cpu.sp = SP;
    m.cpu.pc = LOOP_ADDR;
    // Warmup.
    m.run(Some(1_000_000)).unwrap();
    let t = Instant::now();
    m.run(Some(steps as u32)).unwrap();
    let secs = t.elapsed().as_secs_f64();
    (steps as f64) / secs / 1.0e6
}

#[test]
#[ignore = "manual perf probe; run with --release --features event-scheduler -- --ignored --nocapture"]
fn kw41z_walk_free_mips() {
    const STEPS: u64 = 60_000_000;
    let before = time_run(false, 1, STEPS);
    let after = time_run(true, 64, STEPS);
    println!("\n=== FRDM-KW41Z walk-free perf (native release) ===");
    println!("walk ON  @ interval 1  (pre-migration):  {before:8.2} MIPS");
    println!("walk OFF @ interval 64 (post-migration): {after:8.2} MIPS");
    println!("speedup: {:.2}x\n", after / before);
}
