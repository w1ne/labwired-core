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
//! Generalized over the shipped labs
//! =================================
//! The per-lab table + budget lives in `docs/coverage/lab-perf-budget.json`
//! (one row per shipped lab: chip/system yaml, flash basename, and the
//! `min_mean_batch` / `min_lit_px` / `min_idle_ff_ratio` floors). This test
//! runs every `boot: "esp32c3-rom"` row through the exact browser fast-start
//! policy and asserts each floor. Adding a shipped lab is a JSON row, not a code
//! change. Other boot kinds SKIP until their boot path is wired here.
//!
//! Cross-repo note
//! ===============
//! The flash images live in the SUPERPROJECT (`packages/playground/public/wasm/`),
//! not here, and are deliberately not vendored into core (one artifact, one home
//! — a copy would drift silently and reintroduce exactly the skew this file
//! exists to catch). Each row resolves its image in this order:
//! 1. `$LABWIRED_WASM_DIR/<basename>` — the superproject CI sets this once so
//!    every lab resolves (what the SUPERPROJECT PR CI should export);
//! 2. `$LABWIRED_C3_SHIPPED_FLASH` — legacy single-image override (128x64 only);
//! 3. the sibling superproject checkout, when core is a submodule inside it.
//!
//! If none resolves, the row SKIPS loudly rather than failing, so a standalone
//! core checkout stays green. This is `#[ignore]`d + `event-scheduler`-gated, so
//! it does NOT run in core-ci's default lanes; the SUPERPROJECT PR CI runs it:
//!   cargo test -p labwired-core --release --features event-scheduler \
//!     --test esp32c3_shipped_lab_batch_gate -- --ignored --nocapture
//! with `LABWIRED_WASM_DIR=packages/playground/public/wasm` exported. That is the
//! only environment where the shipped binaries are present for the gate to bite.

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

/// Resolve a SHIPPED flash image by basename. See the module docs for why these
/// live in the superproject and are resolved rather than vendored into core.
///
/// Resolution order:
///   1. `$LABWIRED_WASM_DIR/<basename>` — the superproject CI points this at
///      `packages/playground/public/wasm/` so every lab resolves at once.
///   2. `$LABWIRED_C3_SHIPPED_FLASH` — legacy single-image override; only honored
///      for the 128x64 primary lab it was introduced for.
///   3. the sibling superproject checkout, when core is a submodule inside it.
///
/// Returns `None` (test SKIPS) when the image is absent, so a standalone core
/// checkout stays green.
fn shipped_flash_path(basename: &str) -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("LABWIRED_WASM_DIR") {
        let p = PathBuf::from(dir).join(basename);
        if p.exists() {
            return Some(p);
        }
    }
    if basename == "demo-esp32c3-display-workshop-flash.bin" {
        if let Ok(p) = std::env::var("LABWIRED_C3_SHIPPED_FLASH") {
            let p = PathBuf::from(p);
            assert!(
                p.exists(),
                "LABWIRED_C3_SHIPPED_FLASH points at a missing file: {}",
                p.display()
            );
            return Some(p);
        }
    }
    // core/crates/core → core → <superproject>
    let sibling = root()
        .join("../../../packages/playground/public/wasm")
        .join(basename);
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
fn build_lab(chip_yaml: &str, system_yaml: &str, flash_path: &PathBuf) -> Lab {
    let chip = ChipDescriptor::from_file(root().join("../..").join(chip_yaml))
        .unwrap_or_else(|e| panic!("load chip yaml {chip_yaml}: {e}"));
    let manifest = SystemManifest::from_file(root().join("../..").join(system_yaml))
        .unwrap_or_else(|e| panic!("load system yaml {system_yaml}: {e}"));
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
            // A fresh browser tab mints its first die at FIRST_FACTORY_MAC;
            // pin it so a batch of labs in ONE process each boot the die the
            // tab would have given them rather than the next one in sequence.
            pinned_efuse_mac: Some(labwired_core::system::efuse::FIRST_FACTORY_MAC),
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

/// Run a classic-ESP32 Arduino ELF row through the SHARED boot path
/// (`labwired_core::boot::esp32_arduino`) and measure it the same way the C3
/// rows are measured.
///
/// Two things are deliberate here.
///
/// **`machine.run()`, not `machine.step()`.** Batch width is
/// `cpu_instructions / cpu_batches`, so a harness that single-steps pins the
/// denominator to the instruction count and reports ~1.0 no matter how the
/// engine behaves. The e-reader end-to-end test single-steps (it wants a PC
/// trace), which is why the throughput assertion could not simply be bolted on
/// there — it would have measured the harness rather than the model.
///
/// **No boot code lives in this file.** The bus, the CPU pair, the stack seeds
/// and the thunk install all come from the boot module. A copy here would be a
/// second home for the one sequence whose steps fail silently when missed.
fn run_classic_arduino_lab(
    system_yaml: &str,
    elf_path: &PathBuf,
) -> (labwired_core::StepProfile, usize, u64) {
    use labwired_core::boot::esp32_arduino::{build_arduino_elf_machine, ArduinoElfBootOpts};

    let manifest = SystemManifest::from_file(root().join("../..").join(system_yaml))
        .unwrap_or_else(|e| panic!("load system yaml {system_yaml}: {e}"));
    let elf_bytes = std::fs::read(elf_path)
        .unwrap_or_else(|e| panic!("read shipped ELF {}: {e}", elf_path.display()));
    let image = labwired_loader::load_elf(elf_path)
        .unwrap_or_else(|e| panic!("parse shipped ELF {}: {e}", elf_path.display()));
    let symbol_addrs = labwired_loader::extract_arduino_esp32_thunks(&elf_bytes);

    let mut booted = build_arduino_elf_machine(
        &image,
        symbol_addrs,
        &manifest,
        &ArduinoElfBootOpts::default(),
    )
    .expect("boot classic-ESP32 Arduino ELF");

    booted.machine.reset_step_profile();
    let mut fuel: u64 = 0;
    while fuel < BUDGET {
        let n = 1_000_000u32.min((BUDGET - fuel) as u32);
        // A firmware fault mid-run is not a perf result. Stop and let the
        // liveness floor below report it as the wedge it is.
        if booted.machine.run(Some(n)).is_err() {
            break;
        }
        fuel += u64::from(n);
    }

    let serial_bytes = booted.uart_sink.lock().unwrap().len();
    let total_cycles = booted.machine.total_cycles;
    (
        booted.machine.step_profile().clone(),
        serial_bytes,
        total_cycles,
    )
}

/// One row of the committed per-lab budget (`docs/coverage/lab-perf-budget.json`).
struct LabBudget {
    lab: String,
    boot: String,
    chip_yaml: String,
    system_yaml: String,
    flash_basename: String,
    min_mean_batch: f64,
    min_lit_px: usize,
    min_idle_ff_ratio: f64,
    /// Liveness floor for rows whose display this gate cannot read directly.
    ///
    /// `min_lit_px` is the anti-wedge guard for the C3 OLED rows — "a fast engine
    /// that renders nothing is not a pass". A classic-ESP32 e-paper row has no
    /// SSD1306 to count, so its liveness evidence is the firmware's own serial
    /// output. Some floor above zero is mandatory on every row: the fastest
    /// configurations measured on this chip were fast precisely BECAUSE they were
    /// wedged and emitted nothing.
    min_serial_bytes: usize,
    /// CEILING on legacy walk entries per simulated cycle. `0.0` disables it.
    ///
    /// Mean batch width is the right metric for the batch-COLLAPSE class that
    /// took down the C3 labs, and it is the wrong one for classic ESP32. Measured
    /// on the shipped e-reader: batch width 108.9, which reads as perfectly
    /// healthy — while the same configuration runs at 0.498 M cycles/s, roughly
    /// 7x below what the engine can do. The cost on this chip is not batching at
    /// all, it is per-cycle peripheral walk work, and batch width cannot see it.
    ///
    /// `legacy_tick_entries / total_cycles` can. It is a pure counter ratio, so
    /// it is bit-deterministic like batch width and does not flake on a loaded
    /// box, and it moves in the direction the regression moves. A floor on batch
    /// width alone would have been decorative here — green while the chip that
    /// prompted this gate stayed 7x slow.
    max_legacy_ticks_per_cycle: f64,
    /// Set ONLY on a row this gate is knowingly unable to execute yet.
    ///
    /// Without this field an unwired `boot` kind used to `continue` with a
    /// printed note, which meant the budget file could advertise coverage the
    /// gate never delivered: the row reads as gated in review, the run prints a
    /// line nobody diffs, and the chip is measured by nothing. Classic ESP32 sat
    /// in exactly that shape — uncovered while a 7x throughput regression
    /// shipped. An unwired row must now say so out loud, in the file, where the
    /// reviewer of the NEXT change can see it.
    unenforced_reason: Option<String>,
}

/// True when the caller has declared that the shipped binaries are present and
/// this gate is expected to actually measure something.
///
/// The superproject CI sets `LABWIRED_WASM_DIR`; a standalone core checkout does
/// not. That single fact separates "skipping is correct here" from "skipping is
/// the bug" — so every skip below is fatal when it is set, and tolerated when it
/// is not. Before this, both cases skipped green and the only thing standing
/// between a silent no-op and a real gate was a workflow step in another repo.
fn gate_must_bite() -> bool {
    std::env::var_os("LABWIRED_WASM_DIR").is_some()
}

fn budget_path() -> PathBuf {
    root().join("../../docs/coverage/lab-perf-budget.json")
}

/// Parse the committed budget. The budget file is authoritative — a lab with no
/// row here is not gated, and a row is a *contract* the shipped binary must meet.
fn load_budget() -> Vec<LabBudget> {
    let raw = std::fs::read_to_string(budget_path())
        .unwrap_or_else(|e| panic!("read {}: {e}", budget_path().display()));
    let json: serde_json::Value = serde_json::from_str(&raw).expect("budget json parses");
    json["labs"]
        .as_array()
        .expect("budget.labs is an array")
        .iter()
        .map(|row| LabBudget {
            lab: row["lab"].as_str().expect("lab").to_string(),
            boot: row["boot"].as_str().expect("boot").to_string(),
            chip_yaml: row["chip_yaml"].as_str().expect("chip_yaml").to_string(),
            system_yaml: row["system_yaml"]
                .as_str()
                .expect("system_yaml")
                .to_string(),
            flash_basename: row["flash_basename"]
                .as_str()
                .expect("flash_basename")
                .to_string(),
            min_mean_batch: row["min_mean_batch"].as_f64().expect("min_mean_batch"),
            min_lit_px: row["min_lit_px"].as_u64().expect("min_lit_px") as usize,
            min_idle_ff_ratio: row["min_idle_ff_ratio"]
                .as_f64()
                .expect("min_idle_ff_ratio"),
            max_legacy_ticks_per_cycle: row
                .get("max_legacy_ticks_per_cycle")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0),
            min_serial_bytes: row
                .get("min_serial_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
            unenforced_reason: row
                .get("unenforced_reason")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        })
        .collect()
}

/// The batch-collapse throughput gate for the firmware users actually boot,
/// generalized over every shipped lab in the committed budget.
///
/// Only `boot: "esp32c3-rom"` rows run today — that ROM-boot path is the only
/// one wired here. Other boot kinds (ELF-loaded ARM labs, esp32s3 Xtensa) SKIP
/// with a note until their boot machinery is added; their binaries also live in
/// the superproject, not core. Rows whose flash image is not resolvable SKIP
/// cleanly so a standalone core checkout stays green — the SUPERPROJECT CI sets
/// `LABWIRED_WASM_DIR=packages/playground/public/wasm` for the gate to bite.
#[test]
#[ignore = "runs real shipped bootloaders (~30M steps/lab); run with --release --ignored"]
fn shipped_labs_keep_batching() {
    let budget = load_budget();
    let mut ran = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut unenforced: Vec<String> = Vec::new();

    for b in &budget {
        if b.boot == "esp32-arduino-elf" {
            let Some(elf) = shipped_flash_path(&b.flash_basename) else {
                let msg = format!(
                    "{}: shipped ELF '{}' not found; set LABWIRED_WASM_DIR.",
                    b.lab, b.flash_basename
                );
                if gate_must_bite() {
                    failures.push(msg);
                } else {
                    eprintln!("SKIP {msg}");
                }
                continue;
            };
            eprintln!("{}: shipped ELF {}", b.lab, elf.display());

            let (profile, serial_bytes, total_cycles) =
                run_classic_arduino_lab(&b.system_yaml, &elf);
            let mean_batch = profile.cpu_instructions as f64 / profile.cpu_batches.max(1) as f64;
            let walk_per_cycle = profile.legacy_tick_entries as f64 / total_cycles.max(1) as f64;

            eprintln!(
                "SHIPPED_LAB_GATE {} mean_batch={mean_batch:.3} (min {}) serial_bytes={serial_bytes} \
                 (min {}) walk_per_cycle={walk_per_cycle:.3} (max {}) batches={} interpreted={} \
                 legacy_tick_entries={} total_cycles={total_cycles}",
                b.lab,
                b.min_mean_batch,
                b.min_serial_bytes,
                b.max_legacy_ticks_per_cycle,
                profile.cpu_batches,
                profile.cpu_instructions,
                profile.legacy_tick_entries,
            );

            // Liveness FIRST. On this chip the fastest measured configurations
            // were the broken ones — walk-deleted runs hit 2.5-2.9 M cycles/s
            // while emitting zero bytes. Reporting throughput for a machine that
            // produced no output is how a wedge gets recorded as a win.
            if serial_bytes < b.min_serial_bytes {
                failures.push(format!(
                    "{}: firmware emitted {serial_bytes} serial byte(s) (min {}) — the run \
                     produced no evidence it executed. Do NOT read the batch width below as \
                     a speed result; a wedged machine is fast for the wrong reason.",
                    b.lab, b.min_serial_bytes
                ));
            }
            if mean_batch < b.min_mean_batch {
                failures.push(format!(
                    "{}: batch width collapsed to {mean_batch:.3} (min {}). Classic ESP32 \
                     runs the legacy per-cycle walk at tick interval 1; if this dropped, \
                     something started scheduling an event every ~{mean_batch:.1} guest \
                     instructions.",
                    b.lab, b.min_mean_batch
                ));
            }
            if b.max_legacy_ticks_per_cycle > 0.0 && walk_per_cycle > b.max_legacy_ticks_per_cycle {
                failures.push(format!(
                    "{}: legacy walk work rose to {walk_per_cycle:.3} entries per simulated \
                     cycle (max {}). This is the metric that actually tracks classic-ESP32 \
                     speed — batch width stays healthy while this climbs. Something added \
                     per-cycle walk work, or a peripheral stopped being skippable.",
                    b.lab, b.max_legacy_ticks_per_cycle
                ));
            }
            ran += 1;
            continue;
        }
        if b.boot != "esp32c3-rom" {
            // An unwired boot kind is only acceptable if the row admits it. A row
            // that neither runs nor explains itself is a coverage claim with
            // nothing behind it.
            match &b.unenforced_reason {
                Some(reason) => {
                    eprintln!("UNENFORCED {}: boot kind '{}' — {reason}", b.lab, b.boot);
                    unenforced.push(b.lab.clone());
                }
                None => failures.push(format!(
                    "{}: boot kind '{}' is not wired in this gate, and the row does not \
                     carry `unenforced_reason`. This row measures NOTHING. Either wire \
                     the boot path, or state in the budget file why it cannot be gated \
                     yet — do not let the file imply coverage that does not exist.",
                    b.lab, b.boot
                )),
            }
            continue;
        }
        let Some(flash) = shipped_flash_path(&b.flash_basename) else {
            let msg = format!(
                "{}: shipped flash '{}' not found. Set LABWIRED_WASM_DIR to \
                 <superproject>/packages/playground/public/wasm/, or run with the \
                 superproject checked out around this submodule.",
                b.lab, b.flash_basename
            );
            // Resolving nothing is normal in a standalone core checkout and fatal
            // in the CI that exists to measure. Same condition, opposite meaning.
            if gate_must_bite() {
                failures.push(format!(
                    "{msg}\n  LABWIRED_WASM_DIR is set, so this gate was expected to \
                     measure this lab and silently did not."
                ));
            } else {
                eprintln!("SKIP {msg}");
            }
            continue;
        };
        eprintln!("{}: shipped flash {}", b.lab, flash.display());

        let mut lab = build_lab(&b.chip_yaml, &b.system_yaml, &flash);
        lab.machine.reset_step_profile();
        let mut fuel: u64 = 0;
        while fuel < BUDGET {
            let n = 1_000_000u32.min((BUDGET - fuel) as u32);
            lab.machine.run(Some(n)).expect("run shipped lab");
            fuel += u64::from(n);
        }

        let profile = lab.machine.step_profile();
        let mean_batch = profile.cpu_instructions as f64 / profile.cpu_batches.max(1) as f64;
        let lit = ssd1306_lit_pixels(&lab.machine);
        let idle_ff_ratio = lab.machine.idle_fast_forward_cycles_skipped as f64
            / lab.machine.total_cycles.max(1) as f64;

        eprintln!(
            "SHIPPED_LAB_GATE {} mean_batch={mean_batch:.3} (min {}) lit_px={lit} (min {}) \
             idle_ff_ratio={idle_ff_ratio:.3} (min {}) batches={} interpreted={} total_cycles={}",
            b.lab,
            b.min_mean_batch,
            b.min_lit_px,
            b.min_idle_ff_ratio,
            profile.cpu_batches,
            profile.cpu_instructions,
            lab.machine.total_cycles,
        );

        if lit < b.min_lit_px {
            failures.push(format!(
                "{}: OLED painted only {lit} px (min {}) — a fast engine that renders \
                 nothing is not a pass",
                b.lab, b.min_lit_px
            ));
        }
        if mean_batch < b.min_mean_batch {
            failures.push(format!(
                "{}: batch width collapsed to {mean_batch:.3} (min {}). Some peripheral is \
                 scheduling an event every ~{mean_batch:.1} guest instructions — the shipped \
                 regression class. MEASURE the owner (`EventScheduler::next_event_deadline`) \
                 before assuming it is the UART.",
                b.lab, b.min_mean_batch
            ));
        }
        if idle_ff_ratio < b.min_idle_ff_ratio {
            failures.push(format!(
                "{}: idle fast-forward ratio {idle_ff_ratio:.3} below floor {} — the browser \
                 fast-start policy stopped skipping idle time.",
                b.lab, b.min_idle_ff_ratio
            ));
        }
        ran += 1;
    }

    if !unenforced.is_empty() {
        // Print the hole on every run, at the end, where it cannot be lost in
        // 30M steps of output. `docs/coverage/lab-perf-budget.json` is the place
        // to shrink this list.
        eprintln!(
            "SHIPPED_LAB_GATE coverage: {} enforced, {} UNENFORCED ({}).",
            ran,
            unenforced.len(),
            unenforced.join(", ")
        );
    }

    // Structural complaints (a row that measures nothing, a binary that should
    // have been there) are reported BEFORE the zero-labs branch. They are true
    // regardless of how many labs ran, and an early return here would swallow
    // exactly the "row claims coverage it does not have" failure this change
    // exists to surface.
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));

    if ran == 0 {
        let msg = "no shipped flash image resolved — this gate exercises the firmware \
                   users actually boot and needs the superproject binaries present.";
        // The old code returned green here. That is the failure mode this whole
        // file was written to catch, reproduced in the file itself: every row
        // skipping still reported success, so the gate could measure nothing and
        // nobody would learn about it from the result.
        assert!(
            !gate_must_bite(),
            "{msg}\n  LABWIRED_WASM_DIR is set, so a run that measured zero labs is a \
             failure, not a skip."
        );
        eprintln!("SKIP: {msg}");
    }
}
