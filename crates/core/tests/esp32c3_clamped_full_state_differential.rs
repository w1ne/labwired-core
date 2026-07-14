// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Phase-0 event-scheduler CLAMP proof (the "by construction" gate).
//!
//! `Machine::run`'s batch loop now clamps every batch to the next scheduled
//! peripheral event (`sched.next_event_deadline`) whenever
//! `peripheral_tick_interval > 1` — the GENERAL clamp that generalises the
//! HC-SR04 single-peripheral clamp to the whole scheduler heap. The claim it
//! backs is strong: with it, a WIDE tick interval delivers every scheduled
//! event (and thus pends every IRQ and enters every ISR) at the IDENTICAL
//! absolute cycle it would at interval 1, so the two runs are byte-identical
//! INCLUDING the `cpu_state` register snapshot (pc / mepc / GPRs / CSRs) — the
//! one field the CLI interim fidelity gate
//! (`riscv_tick_interval_fidelity_differential`) had to EXCLUDE because,
//! without the clamp, interval-64 serviced interrupts on coarse 64-cycle
//! boundaries and halted the firmware at a different micro-instant.
//!
//! This runs the REAL `esp32c3-oled-demo` lab (the exact browser fast-start
//! assembly) for a fixed instruction budget at interval 1 and interval 64
//! (scheduler-driven RTC/SYSTIMER/I²C0 in BOTH, the shipped config) and
//! asserts the FULL machine snapshot — cpu_state AND every peripheral's state
//! — is byte-identical. This is a STRICTLY stronger assertion than the walk
//! differential's framebuffer-only identity: it also covers the CPU's live
//! registers, which is exactly what the clamp is supposed to restore.
//!
//! It doubles as the Gap-#1 end-to-end probe: the OLED firmware arms SYSTIMER
//! alarms and I²C0 module ticks via ordinary MMIO stores MID-batch, so if a
//! mid-batch-scheduled event were delivered late at interval 64 the firmware
//! would be interrupted at a different instruction and cpu_state would drift.
//! A green run is evidence that the real mid-batch-arming workload stays
//! cycle-exact under the clamp.

#![cfg(feature = "event-scheduler")]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::boot::esp32c3_rom::{
    build_rom_boot_machine, c3_rom_data_init_writes, inject_rom_regions, RomBootOpts,
};
use labwired_core::boot::esp32s3_rom::RomImages;
use labwired_core::bus::SystemBus;
use labwired_core::cpu::RiscV;
use labwired_core::memory::ProgramImage;
use labwired_core::{Arch, Bus, Cpu, DebugControl, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

const ESP_IMAGE_HEADER_LEN: usize = 24;
const ESP_IMAGE_MAGIC: u8 = 0xE9;

fn esp32c3_bootloader_image(flash: &[u8]) -> ProgramImage {
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

struct OledLab {
    machine: Machine<RiscV>,
    #[allow(dead_code)]
    serial: Arc<Mutex<Vec<u8>>>,
}

/// Build the OLED lab exactly as the browser fast-start does, at the given
/// tick interval, fully scheduler-driven (the shipped config). Mirrors
/// `build_oled_lab(interval, false, false, false, false)` in
/// `esp32c3_walk_differential.rs`.
fn build_oled_lab(tick_interval: u32) -> OledLab {
    let chip = ChipDescriptor::from_file(root().join("../../configs/chips/esp32c3.yaml"))
        .expect("load esp32c3 chip yaml");
    let manifest =
        SystemManifest::from_file(root().join("../../configs/systems/esp32c3-oled-demo.yaml"))
            .expect("load esp32c3-oled-demo system yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build oled bus");

    let irom = std::fs::read(root().join("roms/esp32c3/esp32c3_rom.bin")).expect("read C3 IROM");
    let drom = std::fs::read(root().join("roms/esp32c3/esp32c3_drom.bin")).expect("read C3 DROM");
    let flash = std::fs::read(root().join("../wasm/tests/fixtures/esp32c3-oled-demo-flash.bin"))
        .expect("read C3 OLED demo flash image");

    assert!(
        inject_rom_regions(
            &mut bus,
            &RomImages {
                irom: irom.clone(),
                drom
            }
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

    let bootloader = esp32c3_bootloader_image(&flash);
    let mut machine = build_rom_boot_machine(
        bus,
        flash,
        RomBootOpts {
            efuse_mac: None,
            usb_serial_sink: None,
        },
        |c| c,
    );

    for segment in &bootloader.segments {
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
    machine.cpu.set_pc(bootloader.entry_point as u32);

    machine.bus.recompute_walk_deletable();
    machine.config.peripheral_tick_interval = tick_interval;
    machine.bus.config.peripheral_tick_interval = tick_interval;

    OledLab { machine, serial }
}

/// Run exactly `budget` instructions in chunks and return the full snapshot.
fn run_to_snapshot(mut lab: OledLab, budget: u64) -> (u64, labwired_core::snapshot::MachineSnapshot) {
    const CHUNK: u32 = 1_000_000;
    let mut steps = 0u64;
    while steps < budget {
        let n = CHUNK.min((budget - steps) as u32);
        lab.machine.run(Some(n)).expect("run oled lab");
        steps += n as u64;
    }
    (lab.machine.total_cycles, lab.machine.snapshot())
}

/// Instruction budget that covers bootloader + app + first OLED paint (the
/// window where SYSTIMER alarms + I²C0 ticks are actively arming, i.e. Gap #1
/// is live).
const PAINT_BUDGET: u64 = 30_000_000;

/// DIAGNOSTIC: run both arms in lockstep 100k-step chunks and report the FIRST
/// checkpoint at which `pc` diverges — distinguishes a fixable late-event bug
/// (sharp early divergence) from fundamental bounded-stale lazy-counter reads
/// (gradual, small, late divergence).
#[test]
#[ignore = "diagnostic; run with --release --ignored --nocapture"]
fn diag_first_divergence_point_interval_1_vs_64() {
    const CHUNK: u32 = 100_000;
    const MAX: u64 = 30_000_000;
    let mut a = build_oled_lab(1).machine;
    let mut b = build_oled_lab(64).machine;
    let mut steps = 0u64;
    while steps < MAX {
        a.run(Some(CHUNK)).unwrap();
        b.run(Some(CHUNK)).unwrap();
        steps += CHUNK as u64;
        let pa = a.cpu.get_pc();
        let pb = b.cpu.get_pc();
        if pa != pb || a.total_cycles != b.total_cycles {
            eprintln!(
                "FIRST DIVERGENCE at steps={steps}: pc 0x{pa:08x} vs 0x{pb:08x} (Δ={}), \
                 total_cycles {} vs {} (Δ={})",
                pa as i64 - pb as i64,
                a.total_cycles,
                b.total_cycles,
                a.total_cycles as i64 - b.total_cycles as i64,
            );
            return;
        }
    }
    eprintln!("NO pc/total_cycles divergence through {MAX} steps");
}

/// STEP-1 GO/NO-GO measurement: are ALL peripheral tick `costs` ZERO across the
/// full OLED run at BOTH intervals? If yes, intra-batch instruction `i` retires
/// at absolute cycle `batch_start + i` at every interval → exact cpu_state
/// convergence is achievable by anchoring `bus.current_cycle` per instruction.
/// If nonzero costs interleave mid-run, `batch_start + i` cannot equal the
/// interval-1 cycle and full convergence is impossible via that mechanism.
#[test]
#[ignore = "diagnostic; run with --release --ignored --nocapture"]
fn diag_measure_tick_costs_interval_1_and_64() {
    use labwired_core::metrics::PerformanceMetrics;
    for interval in [1u32, 64] {
        let mut lab = build_oled_lab(interval);
        let metrics = Arc::new(PerformanceMetrics::new());
        lab.machine.observers.push(metrics.clone());
        const CHUNK: u32 = 1_000_000;
        let mut steps = 0u64;
        while steps < PAINT_BUDGET {
            let n = CHUNK.min((PAINT_BUDGET - steps) as u32);
            lab.machine.run(Some(n)).expect("run oled lab");
            steps += n as u64;
        }
        let instrs = metrics.get_instructions();
        let cycles = lab.machine.total_cycles;
        let periph_cost = metrics.get_peripheral_cycles_total();
        eprintln!(
            "[step1] interval={interval}: instructions={instrs} total_cycles={cycles} \
             peripheral_cost_cycles={periph_cost} (GO iff peripheral_cost_cycles==0)"
        );
    }
}

/// THE clamp proof: interval-64 (general clamp + Gap-#1 break + exact-cycle
/// clock active) produces a machine snapshot BYTE-IDENTICAL to interval-1 —
/// INCLUDING `cpu_state` (pc / mepc / GPRs / CSRs), the field the CLI interim
/// gate had to exclude. Three mechanisms combine to make this hold by
/// construction:
///   1. the general clamp ends every batch on the next scheduled event, so
///      each IRQ/event is DELIVERED at its exact absolute cycle;
///   2. the Gap-#1 break ends the batch on any mid-batch MMIO write that armed
///      an event, so a just-armed event is enqueued before the batch overruns
///      its deadline;
///   3. the exact-cycle clock republishes `batch_start + retired` before each
///      interpreted instruction, so a mid-batch READ of a lazily-derived
///      RTC/SYSTIMER counter sees the SAME value interval-1 would — a firmware
///      busy-waiting on that counter now exits its poll on the identical
///      instruction at any tick interval.
///
/// With all three, nothing the firmware can observe (event, IRQ, or counter
/// read) differs between intervals, so the CPU executes the identical
/// instruction stream and halts in the identical register state. If this ever
/// fails, one of the three regressed — the diagnostic
/// `diag_first_divergence_point_interval_1_vs_64` bisects to the first
/// diverging instruction.
#[test]
#[ignore = "runs the real C3 bootloader + app (~2x30M steps); run with --release --ignored"]
fn oled_lab_full_state_byte_identical_interval_1_vs_64() {
    let (cycles_1, snap_1) = run_to_snapshot(build_oled_lab(1), PAINT_BUDGET);
    let (cycles_64, snap_64) = run_to_snapshot(build_oled_lab(64), PAINT_BUDGET);

    let json_1 = serde_json::to_value(&snap_1).expect("serialize interval-1 snapshot");
    let json_64 = serde_json::to_value(&snap_64).expect("serialize interval-64 snapshot");

    assert_eq!(
        cycles_1, cycles_64,
        "total_cycles diverged (interval-1={cycles_1}, interval-64={cycles_64})"
    );

    // cpu_state first (the headline: this is what the CLI gate had to exclude).
    assert_eq!(
        json_1["cpu"], json_64["cpu"],
        "cpu_state DIVERGED between interval-1 and interval-64.\n--- interval-1 ---\n{}\n\
         --- interval-64 ---\n{}",
        serde_json::to_string_pretty(&json_1["cpu"]).unwrap_or_default(),
        serde_json::to_string_pretty(&json_64["cpu"]).unwrap_or_default(),
    );

    // Then the whole machine snapshot (cpu + every peripheral) byte-for-byte.
    assert_eq!(
        json_1, json_64,
        "FULL machine snapshot diverged between interval-1 and interval-64 (a \
         peripheral's state differs even though cpu_state matched)."
    );

    eprintln!("[clamp-proof] FULL machine snapshot (cpu_state + all peripherals) byte-identical at interval-1 vs -64");
}
