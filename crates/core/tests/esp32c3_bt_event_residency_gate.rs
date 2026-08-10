// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! THE GATE FOR THE `esp32c3::bt` EVENT-RESIDENCY FIX.
//!
//! # What was broken, and what nothing was checking
//!
//! `Esp32c3Bt::take_scheduled_events` runs after **every MMIO write** to the BT
//! window. It used to bump [`Esp32c3Bt::arm_seq`] unconditionally, so every poll
//! produced a fresh `(peripheral, token, deadline)` key that the scheduler's
//! dedup index could not collapse. There is no scheduler-side cancel by design,
//! so each duplicate stayed **live** until it fired and was rejected as stale.
//! Measured on the two-node BLE Pong lab (96 M cycles/node, release):
//!
//! ```text
//! LIVE_HWM [bt=789 systimer=4]   max_queued=792  live_event_ceiling_trips=3310
//! ```
//!
//! 789 simultaneously-live events against a ceiling of
//! [`MAX_LIVE_EVENTS_PER_PERIPHERAL`] = 8. The fix (`arm_seq` gated on
//! [`Esp32c3Bt::armed_wake`], plus `CHAIN_REEVALUATE_HORIZON`) drives that to
//! `bt <= 8` and `live_event_ceiling_trips == 0`.
//!
//! **Nothing asserted the headline number.** `live_event_ceiling_trips`'s only
//! reader was `tests/esp32c3_ble_pong_perf_probe.rs`, whose own module docs say
//! "Every test here is `#[ignore]`d and asserts nothing". And the
//! `debug_assert!` behind the counter — the in-engine backstop — was unreachable
//! in BOTH CI configurations:
//!
//! | lane | why the assert cannot fire |
//! |------|----------------------------|
//! | `cargo test -p labwired-core` (debug, default features) | `event-scheduler` is not a default feature, so no scheduler runs at all |
//! | `cargo test --release -p labwired-core --features event-scheduler` | `--release` compiles `debug_assert!` out |
//!
//! The only armed corner is **debug + `--features event-scheduler`**, which is
//! exactly what the `pr-scheduler-observable` job runs. This file lives there.
//!
//! # Why the assertions are explicit and not left to the `debug_assert!`
//!
//! In a debug build the scheduler panics on the ninth live event, so a
//! regression fails here before the assertions at the bottom are reached — that
//! is fine, and it is the loudest possible failure. The explicit checks exist so
//! that the property is stated where someone looks for it, and so that the same
//! file is still a gate if it is ever run in a configuration where
//! `debug_assert!` is compiled out (`core-integrity`'s release scheduler lane).
//!
//! # Why the run has to be a real one
//!
//! The leak is driven by MMIO-write cadence during link-layer activity, so a
//! synthetic bus poke would not reproduce it. This boots the shipped
//! `esp32c3-ble-pong` flash through the mask-ROM fast start and runs until the
//! controller has actually transmitted advertising PDUs onto a private
//! [`BleAirBus`]. The preconditions below refuse to let the test pass without
//! that: no advertising, or too few arms, is a vacuous run and is reported as a
//! failure rather than a pass.

#![cfg(feature = "event-scheduler")]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::boot::esp32c3_rom::{
    build_rom_boot_machine, c3_rom_data_init_writes, inject_rom_regions, RomBootOpts,
};
use labwired_core::boot::esp32s3_rom::RomImages;
use labwired_core::bus::SystemBus;
use labwired_core::cpu::RiscV;
use labwired_core::memory::ProgramImage;
use labwired_core::peripherals::ble_air::BleAirBus;
use labwired_core::peripherals::esp32c3::bt::Esp32c3Bt;
use labwired_core::sched::MAX_LIVE_EVENTS_PER_PERIPHERAL;
use labwired_core::{Arch, Bus, Cpu, DebugControl, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

const ESP_IMAGE_HEADER_LEN: usize = 24;
const ESP_IMAGE_MAGIC: u8 = 0xE9;

/// Cycles per `machine.run` slice. Small enough that the stop condition is
/// checked often, large enough not to dominate.
const SLICE: u32 = 500_000;

/// Hard ceiling on the run. Sized from measurement, not guessed: this image
/// puts its first advertising PDU on the air at 44.5 M cycles (measured, debug,
/// `idle_fast_forward_enabled`), so this is ~1.8x headroom. A run that needs the
/// whole budget fails its precondition rather than passing quietly.
const BUDGET: u64 = 80_000_000;

/// The block has to have transmitted, not merely initialised.
const MIN_ADVERTISEMENTS: u64 = 1;

/// The block has to have armed enough wakes for the ceiling to be a meaningful
/// question. Pre-fix the ceiling (8) was crossed within the first handful of
/// arms, so this is comfortably above the threshold at which the old defect is
/// observable — it is a floor on "did this exercise the arm path at all", not a
/// tuned number.
const MIN_BT_ARMS: u64 = 32;

fn bootloader_image(flash: &[u8]) -> ProgramImage {
    assert!(flash.len() > ESP_IMAGE_HEADER_LEN, "flash image truncated");
    assert_eq!(flash[0], ESP_IMAGE_MAGIC, "bad bootloader image magic");
    let segment_count = flash[1] as usize;
    let entry = u32::from_le_bytes(flash[4..8].try_into().unwrap()) as u64;
    let mut program = ProgramImage::new(entry, Arch::RiscV);
    let mut cursor = ESP_IMAGE_HEADER_LEN;
    for _ in 0..segment_count {
        let load_addr = u32::from_le_bytes(flash[cursor..cursor + 4].try_into().unwrap()) as u64;
        let len = u32::from_le_bytes(flash[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
        cursor += 8;
        program.add_segment(load_addr, flash[cursor..cursor + len].to_vec());
        cursor += len;
    }
    program
}

/// One `esp32c3-ble-pong` node, mask-ROM fast start, shipped flash, private air.
/// Returns the machine, the serial sink and the air so the caller can prove the
/// node really advertised.
fn build_advertising_node() -> (Machine<RiscV>, Arc<Mutex<Vec<u8>>>, BleAirBus) {
    let chip = ChipDescriptor::from_file(root().join("../../configs/chips/esp32c3.yaml"))
        .expect("load esp32c3 chip yaml");
    let manifest =
        SystemManifest::from_file(root().join("../../configs/systems/esp32c3-ble-pong.yaml"))
            .expect("load esp32c3-ble-pong system yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build ble-pong bus");

    let irom = std::fs::read(root().join("roms/esp32c3/esp32c3_rom.bin")).expect("read C3 IROM");
    let drom = std::fs::read(root().join("roms/esp32c3/esp32c3_drom.bin")).expect("read C3 DROM");
    let flash = std::fs::read(root().join("tests/fixtures/esp32c3-ble-pong-flash.bin"))
        .expect("read BLE Pong flash image");
    assert!(
        inject_rom_regions(
            &mut bus,
            &RomImages {
                irom: irom.clone(),
                drom,
            },
        ),
        "chip yaml must declare the C3 IROM region"
    );
    for (dst, bytes) in c3_rom_data_init_writes(&irom) {
        for (i, b) in bytes.iter().enumerate() {
            let _ = bus.write_u8(dst as u64 + i as u64, *b);
        }
    }

    let serial = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(serial.clone(), false);

    // A PRIVATE air: "did THIS node transmit?" must not be answerable by some
    // other controller sharing a process-global bus.
    let air = BleAirBus::new();
    {
        let idx = bus
            .find_peripheral_index_by_name("bt")
            .expect("esp32c3 chip yaml registers the `bt` block");
        bus.peripherals[idx]
            .dev
            .as_any_mut()
            .and_then(|a| a.downcast_mut::<Esp32c3Bt>())
            .expect("`bt` is the C3 BT controller model")
            .set_air(air.clone());
    }
    assert_eq!(air.current_seq(), 0, "private air starts silent");

    let boot = bootloader_image(&flash);
    let mut machine: Machine<RiscV> = build_rom_boot_machine(
        bus,
        flash.clone(),
        RomBootOpts {
            pinned_efuse_mac: None,
            usb_serial_sink: None,
        },
        |c| c,
    );
    for segment in &boot.segments {
        if machine.bus.flash.load_from_segment(segment)
            || machine.bus.ram.load_from_segment(segment)
            || machine
                .bus
                .extra_mem
                .iter_mut()
                .any(|m| m.load_from_segment(segment))
        {
            continue;
        }
        for (i, byte) in segment.data.iter().enumerate() {
            machine
                .bus
                .write_u8(segment.start_addr + i as u64, *byte)
                .expect("load bootloader segment");
        }
    }
    let sp_top = (chip.ram.base + labwired_config::parse_size(&chip.ram.size).unwrap_or(0)) as u32;
    machine.cpu.set_sp(sp_top & !0xF);
    machine.cpu.set_pc(boot.entry_point as u32);

    // Exactly the browser's own policy, read the way `build_node` does.
    let tick = machine.bus.max_safe_tick_interval();
    machine.config.peripheral_tick_interval = tick;
    machine.bus.config.peripheral_tick_interval = tick;
    machine.config.idle_fast_forward_enabled = true;

    (machine, serial, air)
}

/// THE GATE. An advertising `esp32c3-ble-pong` node must keep `bt` inside
/// [`MAX_LIVE_EVENTS_PER_PERIPHERAL`], and the scheduler must record ZERO
/// ceiling trips.
///
/// This is the assertion the `arm_seq` residency fix was measured by
/// (`live_event_ceiling_trips` 3310 -> 0) and which nothing in the tree
/// previously made. Deliberately NOT `#[ignore]`d: the whole defect it covers is
/// a metric that no lane checked.
#[test]
fn advertising_ble_pong_node_keeps_bt_inside_the_live_event_ceiling() {
    let (mut machine, serial, air) = build_advertising_node();
    let bt_idx = machine
        .bus
        .find_peripheral_index_by_name("bt")
        .expect("`bt` peripheral index");

    let started = std::time::Instant::now();
    let mut steps = 0u64;
    while steps < BUDGET && air.current_seq() < MIN_ADVERTISEMENTS {
        let _ = machine.run(Some(SLICE));
        steps += u64::from(SLICE);
    }

    let stats = machine.sched.stats();
    let bt_arms = stats.arms_per_peripheral.get(bt_idx).copied().unwrap_or(0);
    let bt_live_hwm = stats
        .max_live_per_peripheral
        .get(bt_idx)
        .copied()
        .unwrap_or(0);
    let console = String::from_utf8_lossy(&serial.lock().unwrap()).to_string();

    eprintln!(
        "RESIDENCY steps={steps} wall={:.2}s total_cycles={} air_frames={} \
         bt_arms={bt_arms} bt_live_hwm={bt_live_hwm} ceiling={MAX_LIVE_EVENTS_PER_PERIPHERAL} \
         ceiling_trips={} max_queued={} serial_bytes={}",
        started.elapsed().as_secs_f64(),
        machine.total_cycles,
        air.current_seq(),
        stats.live_event_ceiling_trips,
        stats.max_queued_events,
        console.len(),
    );

    // ── PRECONDITIONS: refuse to be a vacuous gate ────────────────────────────
    // A run that never advertises, or never arms, would satisfy every assertion
    // below trivially. Both are failures, not passes.
    assert!(
        air.current_seq() >= MIN_ADVERTISEMENTS,
        "precondition: the node must actually transmit advertising PDUs \
         (got {} frames in {steps} steps). Without a live link layer the \
         residency assertions below prove nothing. Console tail:\n{}",
        air.current_seq(),
        console
            .lines()
            .rev()
            .take(20)
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        bt_arms >= MIN_BT_ARMS,
        "precondition: `bt` must have armed at least {MIN_BT_ARMS} scheduler \
         events (got {bt_arms}). Too few arms means the arm path was barely \
         exercised and the ceiling below is not a meaningful question."
    );

    // ── THE PROPERTY ──────────────────────────────────────────────────────────
    assert!(
        bt_live_hwm <= MAX_LIVE_EVENTS_PER_PERIPHERAL,
        "`bt` held {bt_live_hwm} simultaneously-live scheduler events (ceiling \
         {MAX_LIVE_EVENTS_PER_PERIPHERAL}) over {bt_arms} arms: it is re-arming \
         without superseding its prior wake. This is the pre-fix leak — 789 live \
         events from ~3.3k arms — which inflated the scheduler's linearly-scanned \
         dedup index until `EventScheduler::{{drain_due_into,schedule}}` cost 4.7x \
         the RISC-V interpreter. See `arm_seq` / `armed_wake` in \
         `peripherals/esp32c3/bt.rs`."
    );
    assert_eq!(
        stats.live_event_ceiling_trips, 0,
        "the scheduler recorded {} live-event ceiling trips (max live on any \
         peripheral: {}). This is the number the `esp32c3::bt` residency fix was \
         measured by: 3310 -> 0. Non-zero means it regressed.",
        stats.live_event_ceiling_trips, stats.max_live_events_per_peripheral
    );
}
