// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! WALK-PINNER LEDGER FOR A **BT-ACTIVE** ESP32-C3 BUS.
//!
//! `crates/core/tests/esp32c3_walk_differential.rs`'s
//! `oled_lab_walk_pinners_after_rtc_migration` proves the pinner set is empty
//! on the `esp32c3-oled-demo` rom-boot bus — a bus whose own doc comment says
//! "The OLED demo never enables WiFi, so the pump arms nothing and the bus is
//! fully walk-deletable". The BT block (`bt`, chip-yaml id at 0x6003_1000) IS
//! on that bus, but only as reset-state storage: nothing ever writes it, so it
//! is never un-gated, no comparator is ever armed and no radio event is ever
//! programmed.
//!
//! The 12.6x tick-widening win is all-or-nothing
//! (`SystemBus::max_safe_tick_interval` returns `RECOMMENDED_TICK_INTERVAL`
//! only while `legacy_walk_disabled`, which needs EVERY peripheral to prove
//! walk-independence). So "idle BT does not pin" is not evidence for the BLE
//! labs, where the block is un-gated, comparators are armed every advertising
//! interval and the radio engine is running. This file closes that gap on the
//! real `esp32c3-ble-pong` lab.
//!
//! Two arms, deliberately separate:
//!
//! 1. `ble_pong_bus_with_bt_block_exercised_walk_pinners` — cheap, runs in
//!    BOTH feature configs. Builds the shipped `esp32c3-ble-pong` bus and
//!    programs the BT block the way the BLE ROM does (un-gate + program the
//!    half-slot comparator target + enable it in `INTCNTL`), so
//!    `legacy_tick_active()` is TRUE — the state in which a per-cycle walk
//!    would have real work — then enumerates pinners.
//!    `scheduler_mode()` is `cfg!(feature = "event-scheduler") &&
//!    self.clock.is_some()`, and the clock is handed over unconditionally at
//!    `push_peripheral`, so the two configs must answer differently and this
//!    test says which is which instead of only exercising one.
//!
//! 2. `ble_pong_advertising_node_walk_pinners` — the real thing: the shipped
//!    BLE Pong flash, mask-ROM-injected fast start, run until the controller
//!    has actually TRANSMITTED advertising PDUs onto the air
//!    (`BleAirBus::current_seq() > 0`), then enumerate. Advertising is the
//!    configuration the 12.6x claim has to survive, and only a real run gets
//!    the block into it. `#[ignore]`d + release-gated in the same style as the
//!    other C3 rom-boot gates.

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::Bus;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// BT window offsets, mirrored from `peripherals::esp32c3::bt` (private there).
const BT_RWBLECNTL: u64 = 0x000;
const BT_INTCNTL: u64 = 0x00C;
const BT_TIMER_HS_TARGET: u64 = 0x0E8;
/// `INTCNTL`/`INTRAWSTAT` bit for the half-slot timer comparator.
const BT_INT_TIMER_HS: u32 = 1 << 10;

/// The pinner set, computed with the EXACT predicate
/// `SystemBus::derive_walk_deletable` negates (and the exact one
/// `oled_lab_walk_pinners_after_rtc_migration` uses).
// The non-minimal form is the point: it is `derive_walk_deletable`'s `.all()`
// predicate verbatim under a `!`, so the two can be diffed by eye against
// bus/tick.rs. Clippy's `!uses_scheduler() && needs_legacy_walk()` is equivalent
// but no longer mirrors the source it is supposed to track.
#[allow(clippy::nonminimal_bool)]
fn walk_pinners(bus: &SystemBus) -> Vec<String> {
    bus.peripherals
        .iter()
        .filter(|p| !(p.dev.uses_scheduler() || !p.dev.needs_legacy_walk()))
        .map(|p| p.name.clone())
        .collect()
}

fn report(bus: &SystemBus, label: &str) -> Vec<String> {
    let pinners = walk_pinners(bus);
    eprintln!("--- {label} ---");
    eprintln!(
        "  event-scheduler feature: {}",
        cfg!(feature = "event-scheduler")
    );
    eprintln!("  peripherals: {}", bus.peripherals.len());
    eprintln!("  walk pinners ({}): {:?}", pinners.len(), pinners);
    eprintln!("  legacy_walk_disabled:    {}", bus.legacy_walk_disabled);
    eprintln!(
        "  max_safe_tick_interval:  {}",
        bus.max_safe_tick_interval()
    );
    pinners
}

fn build_ble_pong_bus() -> SystemBus {
    let chip = ChipDescriptor::from_file(root().join("../../configs/chips/esp32c3.yaml"))
        .expect("load esp32c3 chip yaml");
    let manifest =
        SystemManifest::from_file(root().join("../../configs/systems/esp32c3-ble-pong.yaml"))
            .expect("load esp32c3-ble-pong system yaml");
    SystemBus::from_config(&chip, &manifest).expect("build ble-pong bus")
}

/// Base address of the `bt` window on this bus.
fn bt_base(bus: &SystemBus) -> u64 {
    let idx = bus
        .find_peripheral_index_by_name("bt")
        .expect("esp32c3 chip yaml registers the `bt` block");
    bus.peripherals[idx].base
}

/// Program the BT block the way the BLE ROM's `r_rwip_timer_hs_set` does:
/// touch the window (un-gates it → `clock_base` starts CLKN), write a
/// comparator target, then OR its enable into `INTCNTL`.
fn bring_bt_up(bus: &mut SystemBus) {
    let base = bt_base(bus);
    // Un-gate: the first write to the window is what starts CLKN.
    bus.write_u32(base + BT_RWBLECNTL, 0x0000_0001)
        .expect("write RWBLECNTL");
    // Arm the half-slot comparator on a real future deadline.
    bus.write_u32(base + BT_TIMER_HS_TARGET, 0x0000_1000)
        .expect("write TIMER_HS_TARGET");
    bus.write_u32(base + BT_INTCNTL, BT_INT_TIMER_HS)
        .expect("write INTCNTL");
}

/// Arm 1 (both feature configs): the shipped BLE Pong bus with the BT block
/// UN-GATED and a comparator armed — `legacy_tick_active()` true, i.e. the walk
/// would have real work — still has an empty pinner set under
/// `event-scheduler`, and `max_safe_tick_interval()` stays above 1.
///
/// Without the feature `Esp32c3Bt::scheduler_mode()` is false by construction,
/// so `bt` pins and the policy returns 1 — asserted here too, so this test can
/// never pass by accident in a build where the fast path does not exist.
#[test]
fn ble_pong_bus_with_bt_block_exercised_walk_pinners() {
    let mut bus = build_ble_pong_bus();
    bring_bt_up(&mut bus);

    // PRECONDITION: the block really is exercised, not merely instantiated.
    // `legacy_tick_active()` is `irq_work_pending()`: un-gated (`clock_base`)
    // AND an enabled+programmed comparator that has not fired. If this is
    // false the rest of the test is vacuous.
    let bt_idx = bus.find_peripheral_index_by_name("bt").unwrap();
    assert!(
        bus.peripherals[bt_idx].dev.legacy_tick_active(),
        "precondition: the BT block must be un-gated with an armed comparator \
         (legacy_tick_active), otherwise this test proves nothing about an active BT bus"
    );

    // The walk-deletion flag is latched at build, before the block was
    // programmed; re-derive over the live (BT-active) set.
    bus.recompute_walk_deletable();
    let pinners = report(&bus, "esp32c3-ble-pong, BT un-gated + comparator armed");

    if cfg!(feature = "event-scheduler") {
        assert!(
            !pinners.iter().any(|p| p == "bt"),
            "an ACTIVE BT block must not pin the walk under event-scheduler; pinners: {pinners:?}"
        );
        assert!(
            pinners.is_empty(),
            "a BT-active esp32c3-ble-pong bus must stay walk-deletable, but these pin: {pinners:?}"
        );
        assert!(
            bus.max_safe_tick_interval() > 1,
            "a BT-active bus must keep the widened tick interval (got {})",
            bus.max_safe_tick_interval()
        );
    } else {
        // No feature → `scheduler_mode()` is false for every migrated model, so
        // `bt` is on the walk and the whole policy is compiled out.
        assert!(
            pinners.iter().any(|p| p == "bt"),
            "without event-scheduler `Esp32c3Bt::scheduler_mode()` is false, so `bt` must pin \
             the walk; pinners: {pinners:?}"
        );
        assert_eq!(
            bus.max_safe_tick_interval(),
            1,
            "without event-scheduler the widened interval does not exist"
        );
    }
}

/// Arm 2: THE REAL CONFIGURATION. One `esp32c3-ble-pong` node, booted through
/// the mask-ROM fast start with the shipped flash image, run until its
/// controller has transmitted advertising PDUs onto the air. Then: pinners and
/// `max_safe_tick_interval()`, both as the browser latched them at build and as
/// re-derived over the live advertising bus.
/// Feature-gated, not `debug_assertions`-gated on purpose: the release-only
/// ratchet (`release_only_cfg_ratchet.rs`) requires any `cfg(not(debug_
/// assertions))` block to be named in the CI release lane, and `#[ignore]` +
/// "run with --release" is the convention the other C3 rom-boot gates use.
#[cfg(feature = "event-scheduler")]
#[test]
#[ignore = "boots the real C3 BLE Pong image (~60M steps); run with --release --ignored"]
fn ble_pong_advertising_node_walk_pinners() {
    use labwired_core::boot::esp32c3_rom::{
        build_rom_boot_machine, c3_rom_data_init_writes, inject_rom_regions, RomBootOpts,
    };
    use labwired_core::boot::esp32s3_rom::RomImages;
    use labwired_core::cpu::RiscV;
    use labwired_core::memory::ProgramImage;
    use labwired_core::peripherals::ble_air::BleAirBus;
    use labwired_core::peripherals::esp32c3::bt::Esp32c3Bt;
    use labwired_core::{Arch, Cpu, DebugControl, Machine};
    use std::sync::{Arc, Mutex};

    const ESP_IMAGE_HEADER_LEN: usize = 24;
    const ESP_IMAGE_MAGIC: u8 = 0xE9;

    fn bootloader_image(flash: &[u8]) -> ProgramImage {
        assert!(flash.len() > ESP_IMAGE_HEADER_LEN, "flash image truncated");
        assert_eq!(flash[0], ESP_IMAGE_MAGIC, "bad bootloader image magic");
        let segment_count = flash[1] as usize;
        let entry = u32::from_le_bytes(flash[4..8].try_into().unwrap()) as u64;
        let mut program = ProgramImage::new(entry, Arch::RiscV);
        let mut cursor = ESP_IMAGE_HEADER_LEN;
        for _ in 0..segment_count {
            let load_addr =
                u32::from_le_bytes(flash[cursor..cursor + 4].try_into().unwrap()) as u64;
            let len =
                u32::from_le_bytes(flash[cursor + 4..cursor + 8].try_into().unwrap()) as usize;
            cursor += 8;
            program.add_segment(load_addr, flash[cursor..cursor + len].to_vec());
            cursor += len;
        }
        program
    }

    let chip = ChipDescriptor::from_file(root().join("../../configs/chips/esp32c3.yaml"))
        .expect("load esp32c3 chip yaml");
    let mut bus = build_ble_pong_bus();

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

    // A PRIVATE air, so "did this node transmit?" cannot be answered by some
    // other controller in the same test process.
    let air = BleAirBus::new();
    {
        let idx = bus.find_peripheral_index_by_name("bt").unwrap();
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
    let sp_top = (chip.ram.base + chip.ram.size) as u32;
    machine.cpu.set_sp(sp_top & !0xF);
    machine.cpu.set_pc(boot.entry_point as u32);

    // Exactly the browser policy, read at build the way `build_node` does.
    let at_build = machine.bus.max_safe_tick_interval();
    let pinners_at_build = report(
        &machine.bus,
        "esp32c3-ble-pong rom-boot bus, at build (BT idle)",
    );
    machine.config.peripheral_tick_interval = at_build;
    machine.bus.config.peripheral_tick_interval = at_build;
    machine.config.idle_fast_forward_enabled = true;

    // Run until the controller has actually put advertising PDUs on the air.
    const SLICE: u32 = 1_000_000;
    const BUDGET: u64 = 120_000_000;
    const MIN_ADVERTISEMENTS: u64 = 4;
    let mut steps = 0u64;
    while steps < BUDGET && air.current_seq() < MIN_ADVERTISEMENTS {
        let _ = machine.run(Some(SLICE));
        steps += SLICE as u64;
    }
    let console = String::from_utf8_lossy(&serial.lock().unwrap()).to_string();
    eprintln!(
        "ran {steps} steps, total_cycles={}, air frames transmitted={}",
        machine.total_cycles,
        air.current_seq()
    );
    for line in console.lines().filter(|l| l.starts_with("ROLE ")) {
        eprintln!("SIM| {line}");
    }

    // PRECONDITION: BLE is genuinely ACTIVE — real advertising PDUs crossed the
    // air. Without this the pinner ledger below would be the idle-BT claim
    // again, which the OLED gate already covers.
    assert!(
        air.current_seq() >= MIN_ADVERTISEMENTS,
        "precondition: the node must actually advertise (got {} frames in {steps} steps); \
         console tail:\n{}",
        air.current_seq(),
        console
            .lines()
            .rev()
            .take(20)
            .collect::<Vec<_>>()
            .join("\n")
    );

    let pinners_live = report(&machine.bus, "esp32c3-ble-pong rom-boot bus, ADVERTISING");
    assert!(
        pinners_at_build.is_empty(),
        "the browser latches its tick interval at build; the build-time set must be empty: \
         {pinners_at_build:?}"
    );
    assert!(
        pinners_live.is_empty(),
        "an ADVERTISING BLE bus must keep every peripheral walk-independent, but these pin: \
         {pinners_live:?}"
    );
    assert!(
        at_build > 1,
        "the browser must widen the tick interval on a BLE lab (got {at_build})"
    );

    // The live re-derivation must agree: nothing about being advertising may
    // silently take the widened interval away.
    machine.bus.recompute_walk_deletable();
    assert!(
        machine.bus.legacy_walk_disabled,
        "an advertising BLE bus must still derive walk-deletion"
    );
    assert_eq!(
        machine.bus.max_safe_tick_interval(),
        at_build,
        "max_safe_tick_interval must not change once BLE is up"
    );
}
