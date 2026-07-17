// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! THROUGHPUT GATE FOR THE FIRMWARE USERS ACTUALLY BOOT.
//!
//! Why this file exists
//! ===================
//! Every existing C3 benchmark and differential drives
//! `crates/wasm/tests/fixtures/esp32c3-oled-demo-flash.bin`. The deployed
//! playground boots a DIFFERENT image — `demo-esp32c3-display-workshop-flash.bin`
//! (named in the superproject at
//! `packages/playground/src/.../bundled-configs.ts:894`). Those two firmwares
//! stress the engine completely differently:
//!
//! | image                        | mean batch | RTF (native) |
//! |------------------------------|-----------:|-------------:|
//! | fixture (what the gates ran) |      42.29 |         1.27 |
//! | shipped (what users run)     |       1.38 |        0.010 |
//!
//! So a ~330x-below-real-time collapse shipped while every gate stayed green:
//! the shipped app leaves the UART's TXFIFO-empty interrupt enabled, which held
//! a per-cycle scheduler wakeup open and pinned every batch to width 1. The
//! fixture never sets that bit, so the fixture could not see it.
//!
//! What this gate asserts
//! ======================
//! MEAN BATCH WIDTH, not wall-clock time. Batch width is a pure function of the
//! model (`cpu_instructions / cpu_batches`), so it is bit-deterministic and
//! machine-independent — a wall-clock RTF assert would flake on a loaded CI box.
//! It is also the *direct* proxy for this failure class: the pathology IS the
//! batch collapsing toward 1.
//!
//! Cross-repo note
//! ===============
//! The flash image lives in the SUPERPROJECT, not here, and is deliberately not
//! vendored into core (one artifact, one home — a copy would drift silently and
//! reintroduce exactly the skew this file exists to catch). The gate resolves
//! the image in this order:
//! 1. `$LABWIRED_C3_SHIPPED_FLASH` — explicit override (what CI should set);
//! 2. the sibling superproject checkout, when core is a submodule inside it.
//!
//! If neither resolves the test SKIPS loudly rather than failing, so a
//! standalone core checkout stays green. The superproject CI must set the env
//! var (or run with the playground present) for this gate to bite.

#![cfg(feature = "event-scheduler")]

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::boot::esp32c3_rom::{
    build_rom_boot_machine, c3_rom_data_init_writes, inject_rom_regions, RomBootOpts,
};
use labwired_core::boot::esp32s3_rom::RomImages;
use labwired_core::bus::SystemBus;
use labwired_core::cpu::RiscV;
use labwired_core::memory::ProgramImage;
use labwired_core::peripherals::components::Ssd1306;
use labwired_core::peripherals::esp32c3::i2c::Esp32c3I2c;
use labwired_core::{Arch, Bus, Cpu, DebugControl, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The image the deployed playground boots. See the module docs for why this is
/// resolved rather than vendored.
fn shipped_flash_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("LABWIRED_C3_SHIPPED_FLASH") {
        let p = PathBuf::from(p);
        assert!(
            p.exists(),
            "LABWIRED_C3_SHIPPED_FLASH points at a missing file: {}",
            p.display()
        );
        return Some(p);
    }
    // core/crates/core → core → <superproject>
    let sibling = root()
        .join("../../../packages/playground/public/wasm/demo-esp32c3-display-workshop-flash.bin");
    sibling.exists().then_some(sibling)
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

struct Lab {
    machine: Machine<RiscV>,
}

/// The browser fast-start assembly, verbatim (`esp32c3_walk_differential`'s
/// `build_oled_lab`), but pointed at `flash_path` and running the EXACT browser
/// policy (`apply_browser_c3_policy`: recommended tick interval + idle FF).
fn build_lab(flash_path: &PathBuf) -> Lab {
    let chip = ChipDescriptor::from_file(root().join("../../configs/chips/esp32c3.yaml"))
        .expect("load esp32c3 chip yaml");
    let manifest =
        SystemManifest::from_file(root().join("../../configs/systems/esp32c3-oled-demo.yaml"))
            .expect("load esp32c3-oled-demo system yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build oled bus");

    let irom = std::fs::read(root().join("roms/esp32c3/esp32c3_rom.bin")).expect("read C3 IROM");
    let drom = std::fs::read(root().join("roms/esp32c3/esp32c3_drom.bin")).expect("read C3 DROM");
    let flash = std::fs::read(flash_path)
        .unwrap_or_else(|e| panic!("read shipped flash {}: {e}", flash_path.display()));

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
    bus.attach_uart_tx_sink(serial, false);

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

    // EXACT browser policy (`apply_browser_c3_policy`, crates/wasm/src/lib.rs).
    let rec = machine.bus.max_safe_tick_interval();
    machine.config.peripheral_tick_interval = rec;
    machine.bus.config.peripheral_tick_interval = rec;
    machine.config.idle_fast_forward_enabled = true;
    Lab { machine }
}

fn ssd1306_lit_pixels(machine: &Machine<RiscV>) -> usize {
    let idx = machine
        .bus
        .find_peripheral_index_by_name("i2c0")
        .expect("oled bus exposes i2c0");
    machine.bus.peripherals[idx]
        .dev
        .as_any()
        .expect("i2c0 downcastable")
        .downcast_ref::<Esp32c3I2c>()
        .expect("i2c0 is the C3 command-list controller")
        .attached_slaves()
        .iter()
        .filter(|d| d.address() == 0x3C)
        .find_map(|d| d.as_any().and_then(|a| a.downcast_ref::<Ssd1306>()))
        .expect("SSD1306 attached at 0x3C")
        .framebuffer()
        .iter()
        .map(|b| b.count_ones() as usize)
        .sum()
}

const BUDGET: u64 = 30_000_000;

/// Floor for the shipped lab's mean batch width.
///
/// Measured on this branch: **19.42**. Before the UART wakeup fix: **1.376**
/// (the shipped regression), and the theoretical floor of the collapse is 1.0.
/// 8.0 sits 5.8x above the broken value and 2.4x below the healthy one, so it
/// cannot flake (the metric is deterministic — this margin absorbs legitimate
/// firmware/model churn, not host noise) yet still trips long before a collapse
/// toward 1 could reach users. The remaining clamp owner is `i2c0` at its
/// module clock (CPU/4), which bounds any honest future value near ~4 during
/// active I²C traffic; 8.0 stays above that only because traffic is bursty.
/// If a legitimate change drives this below 8, re-measure and move it
/// deliberately — do not delete the gate.
const MIN_MEAN_BATCH: f64 = 8.0;

/// The OLED must still paint: a fast engine that renders nothing is not a pass.
const MIN_LIT: usize = 400;

#[test]
#[ignore = "runs the real C3 bootloader + shipped app (~30M steps); run with --release --ignored"]
fn shipped_c3_display_workshop_lab_keeps_batching() {
    let Some(flash) = shipped_flash_path() else {
        eprintln!(
            "SKIP: shipped C3 flash image not found. Set LABWIRED_C3_SHIPPED_FLASH \
             to <superproject>/packages/playground/public/wasm/\
             demo-esp32c3-display-workshop-flash.bin, or run with the superproject \
             checked out around this submodule. This gate is the ONLY one that \
             exercises the firmware users actually boot."
        );
        return;
    };
    eprintln!("shipped flash: {}", flash.display());

    let mut lab = build_lab(&flash);
    lab.machine.reset_step_profile();
    let mut fuel: u64 = 0;
    while fuel < BUDGET {
        let n = 1_000_000u32.min((BUDGET - fuel) as u32);
        lab.machine.run(Some(n)).expect("run shipped C3 lab");
        fuel += u64::from(n);
    }

    let profile = lab.machine.step_profile();
    let mean_batch = profile.cpu_instructions as f64 / profile.cpu_batches.max(1) as f64;
    let lit = ssd1306_lit_pixels(&lab.machine);
    let rtf_hint = lab.machine.total_cycles as f64 / 160e6;

    eprintln!(
        "SHIPPED_C3_GATE mean_batch={mean_batch:.3} (min {MIN_MEAN_BATCH}) batches={} \
         interpreted={} idle_ff={} total_cycles={} guest_ms={:.1} lit_px={lit}",
        profile.cpu_batches,
        profile.cpu_instructions,
        lab.machine.idle_fast_forward_cycles_skipped,
        lab.machine.total_cycles,
        rtf_hint * 1000.0,
    );

    assert!(
        lit >= MIN_LIT,
        "shipped lab must still paint the OLED (lit={lit}, min {MIN_LIT}) — a fast \
         engine that renders nothing is not a pass"
    );
    assert!(
        mean_batch >= MIN_MEAN_BATCH,
        "SHIPPED C3 lab batch width collapsed to {mean_batch:.3} (min {MIN_MEAN_BATCH}).\n\
         This is the regression that shipped once already: some peripheral is \
         scheduling an event every ~{mean_batch:.1} guest instructions, which pinned \
         the browser to ~RTF 0.003 (a guest second costing ~5.5 wall minutes).\n\
         MEASURE the owner before assuming it is the UART again — attribute batch \
         clamps by peripheral (`EventScheduler::next_event_deadline`) rather than \
         guessing. Last time the owner was `uart0`, holding a per-cycle wakeup to \
         re-assert a level-triggered IRQ that the C3 bus does not even wire \
         (see `Uart::has_active_work`)."
    );
}
