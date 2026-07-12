// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Differential + structural gates for `perf/c3-walk-free` (the ESP32-C3
//! walk-independence campaign) in the #511/#512 style:
//!
//! 1. `oled_lab_walk_on_vs_scheduler_rtc_is_byte_identical_at_interval_1` —
//!    the REAL `esp32c3-oled-demo` lab (flash fast-start through the real
//!    2nd-stage bootloader, the exact assembly the browser uses), run with the
//!    RTC main timer on the legacy per-cycle walk (reference) vs
//!    scheduler-driven (lazy counter via the bus-published `CycleClock`), at
//!    tick interval 1: serial stream, total cycles and the SSD1306 GDDRAM
//!    framebuffer must be BYTE-IDENTICAL after the same instruction budget.
//!
//! 2. `oled_lab_framebuffer_is_byte_identical_across_tick_intervals` — the
//!    same lab at tick interval 64 (scheduler RTC): raw counter reads are
//!    quantised to the tick grid (bounded staleness, < one interval — the
//!    write-path bound), but the OLED output is an event observable and must
//!    not change: the painted framebuffer is byte-identical to interval 1.
//!
//! 3. `esp32c3_irq_choke_reaggregates_on_write` — Task C: FROM_CPU IPI and
//!    INTC config writes re-aggregate `riscv_irq_lines` AT THE WRITE (through
//!    `sync_esp32c3_irq_cache_write`), with no peripheral tick in between —
//!    the property that lets a future walk-free C3 bus take the trivial
//!    per-cycle tick path.
//!
//! 4. `oled_lab_walk_pinners_after_rtc_migration` — the endgame ledger: on
//!    the real OLED rom-boot bus, `rtc_cntl_timer` no longer pins the walk
//!    (`uses_scheduler() == true`), and after the C3/ESP32 Class-A inert
//!    sweep (`needs_legacy_walk() == false` on the verified-inert models) the
//!    remaining pinners are EXACTLY the verified real workers
//!    (i2c0 / ledc / spi2 / apb_saradc / wifi_mac), asserted as an
//!    exact set so the campaign report stays honest. SYSTIMER is now
//!    scheduler-driven (walk-free C3 SYSTIMER batch) and no longer pins.

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
use labwired_core::peripherals::esp32c3::rtc_timer::Esp32c3RtcTimer;
use labwired_core::{Arch, Bus, Cpu, DebugControl, Machine};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

// ───────────────────────── flash fast-start assembly ─────────────────────────
// Mirrors `WasmSimulator::new_from_config_riscv_flash_fastboot` (the browser
// entry): real chip yaml + real oled-demo system yaml + vendored mask-ROM
// blobs + the curated merged flash image, skipping only the ~150M-step mask
// ROM replay by entering at the 2nd-stage bootloader (still the real boot
// chain: bootloader → SHA verify → app).

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
    serial: Arc<Mutex<Vec<u8>>>,
}

/// Build the OLED lab exactly as the browser fast-start does. `rtc_legacy`
/// selects the reference config (RTC main timer pinned back onto the
/// per-cycle walk via `force_legacy_walk`); `systimer_legacy` does the same for
/// the SYSTIMER (the FreeRTOS-tick source), so a walk-on-vs-scheduler
/// differential can isolate the SYSTIMER migration.
fn build_oled_lab(tick_interval: u32, rtc_legacy: bool, systimer_legacy: bool) -> OledLab {
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
    // Fast-start skips the mask-ROM reset code; copy the ROM `.data` records
    // the bootloader's ROM-helper calls depend on (same as the wasm path).
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

    // Load the bootloader segments without a CPU reset and enter at its entry
    // point (the fast-start seam).
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

    if rtc_legacy {
        let idx = machine
            .bus
            .find_peripheral_index_by_name("rtc_cntl_timer")
            .expect("rom-boot bus registers rtc_cntl_timer");
        machine.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Esp32c3RtcTimer>()
            .unwrap()
            .force_legacy_walk();
    }

    if systimer_legacy {
        // Find the real scheduler `Systimer` by type (the rom-boot bus also
        // carries a declarative `systimer` stub at the same base, registered
        // first; the real model is appended by `esp32c3_rom` and wins MMIO).
        let systimer = machine
            .bus
            .peripherals
            .iter_mut()
            .find_map(|p| {
                p.dev.as_any_mut().and_then(|a| {
                    a.downcast_mut::<labwired_core::peripherals::esp32s3::systimer::Systimer>()
                })
            })
            .expect("rom-boot bus registers a real Systimer");
        systimer.force_legacy_walk();
    }

    machine.config.peripheral_tick_interval = tick_interval;
    machine.bus.config.peripheral_tick_interval = tick_interval;

    OledLab { machine, serial }
}

fn ssd1306_framebuffer(machine: &Machine<RiscV>) -> Vec<u8> {
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
        .to_vec()
}

fn lit_pixels(fb: &[u8]) -> usize {
    fb.iter().map(|b| b.count_ones() as usize).sum()
}

/// Run exactly `budget` instructions (in chunks) and return the end state.
fn run_lab(mut lab: OledLab, budget: u64) -> (u64, Vec<u8>, Vec<u8>) {
    const CHUNK: u32 = 1_000_000;
    let mut steps = 0u64;
    while steps < budget {
        let n = CHUNK.min((budget - steps) as u32);
        lab.machine.run(Some(n)).expect("run oled lab");
        steps += n as u64;
    }
    let fb = ssd1306_framebuffer(&lab.machine);
    let serial = lab.serial.lock().unwrap().clone();
    (lab.machine.total_cycles, fb, serial)
}

/// Instruction budget that comfortably covers bootloader + app + first OLED
/// paint on the fast-start path (the wasm gate paints well inside 40M).
const PAINT_BUDGET: u64 = 30_000_000;
/// "LabWired" wordmark + frame + bar → well over this many lit pixels.
const MIN_LIT: usize = 400;

/// Gate 1 (interval 1): walk-on RTC vs scheduler RTC — event observables must
/// be byte-identical: same serial bytes, same total cycles, same framebuffer.
#[test]
#[ignore = "runs the real C3 bootloader + app (~2x30M steps); run with --release --ignored"]
fn oled_lab_walk_on_vs_scheduler_rtc_is_byte_identical_at_interval_1() {
    let (walk_cycles, walk_fb, walk_serial) = run_lab(build_oled_lab(1, true, false), PAINT_BUDGET);
    let (sched_cycles, sched_fb, sched_serial) =
        run_lab(build_oled_lab(1, false, false), PAINT_BUDGET);

    assert!(
        lit_pixels(&walk_fb) >= MIN_LIT,
        "reference run must paint the OLED (lit={}); serial:\n{}",
        lit_pixels(&walk_fb),
        String::from_utf8_lossy(&walk_serial)
    );
    assert_eq!(
        walk_cycles, sched_cycles,
        "total_cycles must be byte-identical at interval 1"
    );
    assert!(
        walk_serial == sched_serial,
        "serial stream must be byte-identical at interval 1\n--- walk ---\n{}\n--- sched ---\n{}",
        String::from_utf8_lossy(&walk_serial),
        String::from_utf8_lossy(&sched_serial)
    );
    assert!(
        walk_fb == sched_fb,
        "SSD1306 framebuffer must be byte-identical at interval 1"
    );
}

/// Gate 2 (interval 64, scheduler RTC): raw counter reads are quantised to
/// the tick grid (documented bounded staleness, < one interval — identical to
/// the write-path bound and to what the legacy walk itself does at interval
/// 64), but the SSD1306 output must not change: byte-identical framebuffer.
#[test]
#[ignore = "runs the real C3 bootloader + app (~2x30M steps); run with --release --ignored"]
fn oled_lab_framebuffer_is_byte_identical_across_tick_intervals() {
    let (_, fb_1, _) = run_lab(build_oled_lab(1, false, false), PAINT_BUDGET);
    let (_, fb_64, serial_64) = run_lab(build_oled_lab(64, false, false), PAINT_BUDGET);

    assert!(
        lit_pixels(&fb_1) >= MIN_LIT,
        "interval-1 run must paint the OLED"
    );
    assert!(
        fb_1 == fb_64,
        "SSD1306 framebuffer must be byte-identical across tick intervals \
         (lit@1={}, lit@64={}); serial@64:\n{}",
        lit_pixels(&fb_1),
        lit_pixels(&fb_64),
        String::from_utf8_lossy(&serial_64)
    );
}

/// SYSTIMER walk-free gate (the fidelity contract for THIS batch): the
/// SYSTIMER is the FreeRTOS-tick source, so the OLED demo's serial log, total
/// cycles and painted framebuffer are all downstream of its alarm delivery.
/// Run the REAL demo with the SYSTIMER on the legacy per-cycle walk
/// (reference) vs scheduler-driven (lazy counter + scheduled alarms routed
/// through the C3 interrupt matrix), at BOTH interval 1 and interval 64 — RTC
/// scheduler in both so the SYSTIMER is the only variable. Every observable
/// must be BYTE-IDENTICAL at each interval: the alarm fires at the same cycle
/// and the tick ISR is entered at the same instruction boundary, so nothing
/// downstream can differ. This is the cycle-exact identity that licenses
/// un-pinning the SYSTIMER from the walk.
#[test]
#[ignore = "runs the real C3 bootloader + app (~4x30M steps); run with --release --ignored"]
fn oled_lab_systimer_walk_on_vs_scheduler_is_byte_identical() {
    for interval in [1u32, 64] {
        // reference: SYSTIMER on the legacy walk; test: SYSTIMER scheduler.
        let (walk_cycles, walk_fb, walk_serial) =
            run_lab(build_oled_lab(interval, false, true), PAINT_BUDGET);
        let (sched_cycles, sched_fb, sched_serial) =
            run_lab(build_oled_lab(interval, false, false), PAINT_BUDGET);

        assert!(
            lit_pixels(&walk_fb) >= MIN_LIT,
            "reference (SYSTIMER walk) must paint the OLED at interval {interval} (lit={}); \
             serial:\n{}",
            lit_pixels(&walk_fb),
            String::from_utf8_lossy(&walk_serial)
        );
        assert_eq!(
            walk_cycles, sched_cycles,
            "total_cycles must be byte-identical (SYSTIMER walk vs scheduler) at interval {interval}"
        );
        assert!(
            walk_serial == sched_serial,
            "serial stream must be byte-identical (SYSTIMER walk vs scheduler) at interval \
             {interval}\n--- walk ---\n{}\n--- sched ---\n{}",
            String::from_utf8_lossy(&walk_serial),
            String::from_utf8_lossy(&sched_serial)
        );
        assert!(
            walk_fb == sched_fb,
            "SSD1306 framebuffer must be byte-identical (SYSTIMER walk vs scheduler) at \
             interval {interval}"
        );
    }
}

/// Task C gate: FROM_CPU IPI + INTC config writes re-aggregate the routed
/// RISC-V line mask AT THE WRITE — no peripheral tick in between. This is the
/// write-choke path (`sync_esp32c3_irq_cache_write` →
/// `recompute_esp32c3_irq_lines`) that makes C3 IRQ routing compatible with a
/// batched tick interval and (on a walk-free bus) the trivial per-cycle tick.
#[test]
fn esp32c3_irq_choke_reaggregates_on_write() {
    const INTMATRIX: u64 = 0x600C_2000;
    const FROM_CPU_0: u64 = 0x600C_0028; // routes as matrix source 50
    const SRC: u64 = 50;
    const LINE: u32 = 5;

    let chip = ChipDescriptor::from_file(root().join("../../configs/chips/esp32c3.yaml"))
        .expect("load esp32c3 chip yaml");
    let manifest =
        SystemManifest::from_file(root().join("../../configs/systems/esp32c3-devkit.yaml"))
            .expect("load esp32c3-devkit system yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build c3 bus");
    bus.esp32c3_irq_routing = true;

    // Route source 50 (FROM_CPU_0) → CPU line 5, priority 1, threshold 1,
    // line enabled — all through ordinary MMIO writes.
    bus.write_u32(INTMATRIX + SRC * 4, LINE).unwrap();
    bus.write_u32(INTMATRIX + 0x114 + (LINE as u64) * 4, 1)
        .unwrap();
    bus.write_u32(INTMATRIX + 0x194, 1).unwrap();
    bus.write_u32(INTMATRIX + 0x104, 1 << LINE).unwrap();
    assert_eq!(
        bus.riscv_irq_lines, 0,
        "no source asserted yet → no routed lines"
    );

    // IPI set: the line must assert at the write, with NO tick in between.
    bus.write_u32(FROM_CPU_0, 1).unwrap();
    assert_eq!(
        bus.riscv_irq_lines,
        1 << LINE,
        "FROM_CPU write must re-aggregate the routed line mask immediately"
    );

    // Masking via the INTC threshold (the FreeRTOS critical-section mechanism)
    // must also take effect at the write.
    bus.write_u32(INTMATRIX + 0x194, 2).unwrap();
    assert_eq!(
        bus.riscv_irq_lines, 0,
        "raising CPU_INT_THRESH above the line priority must mask at the write"
    );
    bus.write_u32(INTMATRIX + 0x194, 1).unwrap();
    assert_eq!(bus.riscv_irq_lines, 1 << LINE, "unmask re-asserts");

    // IPI clear: the line must de-assert at the write.
    bus.write_u32(FROM_CPU_0, 0).unwrap();
    assert_eq!(
        bus.riscv_irq_lines, 0,
        "FROM_CPU clear must de-assert the routed line at the write"
    );
}

/// Native throughput probe for the campaign report: the OLED lab (browser
/// fast-start assembly) run for 50M instructions through `Machine::run`,
/// wall-clocked. Run 3x and take the median:
///   cargo test -p labwired-core --release --features jit,event-scheduler \
///     --test esp32c3_walk_differential mips_probe -- --ignored --nocapture
/// `LABWIRED_MIPS_INTERVAL` overrides the tick interval (default 1 — the
/// interval the deploy currently pins). `LABWIRED_MIPS_SYSTIMER_LEGACY=1` pins
/// the SYSTIMER back onto the per-cycle walk (the pre-migration "before"
/// baseline) so the batch's shrunk-walk effect can be wall-clocked.
#[test]
#[ignore = "wall-clock throughput probe; run 3x with --release --nocapture"]
fn oled_lab_native_mips_probe() {
    let interval: u32 = std::env::var("LABWIRED_MIPS_INTERVAL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let systimer_legacy = std::env::var("LABWIRED_MIPS_SYSTIMER_LEGACY").as_deref() == Ok("1");
    const STEPS: u64 = 50_000_000;
    let mut lab = build_oled_lab(interval, false, systimer_legacy);
    let start = std::time::Instant::now();
    let mut steps = 0u64;
    while steps < STEPS {
        lab.machine.run(Some(1_000_000)).expect("run oled lab");
        steps += 1_000_000;
    }
    let secs = start.elapsed().as_secs_f64();
    eprintln!(
        "oled-lab native: {STEPS} instructions, interval {interval}, systimer {}: \
         {:.2}s = {:.2} MIPS (total_cycles {})",
        if systimer_legacy { "walk" } else { "scheduler" },
        secs,
        STEPS as f64 / secs / 1.0e6,
        lab.machine.total_cycles
    );
}

/// Endgame ledger: after the RTC migration + the C3/ESP32 Class-A inert
/// sweep, which peripherals still pin the per-cycle walk on the REAL OLED
/// rom-boot bus? Locks in that `rtc_cntl_timer` is walk-independent and that
/// the remaining pinners are EXACTLY the verified real workers — models whose
/// tick genuinely mutates state or asserts a level IRQ from the walk:
///
///   (`systimer` — the free-running counter + FreeRTOS tick alarm — is now
///   scheduler-driven and no longer here.)
///   - `i2c0` — bit-level wire engine advances mid-transfer in
///     `tick_elapsed` (num/den module-clock fraction) + level IRQ;
///   - `ledc` — the four low-speed timers run as live up-counters clocked by
///     elapsed cycles (OVF latch) + level IRQ;
///   - `spi2` — `tick()` re-asserts the level interrupt source while
///     `int_raw & int_ena != 0` (reachable whenever firmware enables a
///     TRANS_DONE/DMA int), so deleting the walk would starve the IRQ;
///   - `apb_saradc` — same level-IRQ re-assert pattern from `tick()`
///     (this is the chip-yaml `esp32c3_apb_saradc` controller; the rom-boot
///     `apb_saradc` cal window at the same name is inert and no longer pins);
///   - `wifi_mac` — `tick_with_bus` pumps TX/RX descriptor rings + `tick()`
///     re-asserts the MAC level interrupt while events are pending.
///
/// Every other model on the bus now proves walk-independence itself
/// (`uses_scheduler()` or a verified `needs_legacy_walk() == false`).
/// `derive_walk_deletable()` stays false and `max_safe_tick_interval` stays 1
/// until this set empties — asserted so the report stays honest (zero
/// behavior change is the point of the Class-A sweep).
#[test]
#[ignore = "builds the full rom-boot bus (needs ROM blobs + flash fixture); run with --ignored"]
fn oled_lab_walk_pinners_after_rtc_migration() {
    /// The verified real workers still awaiting scheduler migration. A model
    /// newly marked `needs_legacy_walk() == false` or migrated to the
    /// scheduler must shrink this set; a model that starts pinning again is a
    /// regression.
    const EXPECTED_PINNERS: &[&str] = &["apb_saradc", "i2c0", "ledc", "spi2", "wifi_mac"];

    let lab = build_oled_lab(1, false, false);
    let bus = &lab.machine.bus;

    let mut pinners: Vec<&str> = Vec::new();
    for p in &bus.peripherals {
        let walk_independent = p.dev.uses_scheduler() || !p.dev.needs_legacy_walk();
        if !walk_independent {
            pinners.push(p.name.as_str());
        }
    }
    eprintln!("walk pinners on the esp32c3-oled-demo rom-boot bus:");
    for p in &pinners {
        eprintln!("  - {p}");
    }
    eprintln!("max_safe_tick_interval = {}", bus.max_safe_tick_interval());

    assert!(
        !pinners.contains(&"rtc_cntl_timer"),
        "rtc_cntl_timer must no longer pin the walk; pinners: {pinners:?}"
    );

    let mut got = pinners.clone();
    got.sort_unstable();
    let mut expected: Vec<&str> = EXPECTED_PINNERS.to_vec();
    expected.sort_unstable();
    assert_eq!(
        got,
        expected,
        "walk-pinning set (needs_legacy_walk && !uses_scheduler) drifted from the \
         verified real-worker ledger on the esp32c3-oled-demo rom-boot bus.\n  \
         got ({}):      {:?}\n  expected ({}): {:?}\n\
         A model newly (un)marked `needs_legacy_walk()` or migrated to the scheduler \
         must update EXPECTED_PINNERS to match the campaign's remaining surface.",
        got.len(),
        got,
        expected.len(),
        expected,
    );

    // Zero behavior change: the real workers above still force the walk, so
    // the OLED bus must NOT flip walk-deletable and must stay on interval 1.
    // `derive_walk_deletable()` (crate-private) is true iff every peripheral
    // satisfies `uses_scheduler() || !needs_legacy_walk()` — i.e. iff this
    // pinner set is empty — so a non-empty set IS the derivation staying
    // false, and `max_safe_tick_interval() == 1` is its runtime effect.
    assert!(
        !pinners.is_empty(),
        "OLED rom-boot bus must not derive walk-deletion while real workers remain"
    );
    assert_eq!(
        bus.max_safe_tick_interval(),
        1,
        "bus cannot recommend interval > 1 until every pinner is migrated"
    );
}
