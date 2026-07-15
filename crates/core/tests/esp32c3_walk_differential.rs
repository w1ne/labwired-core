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
//!    sweep (`needs_legacy_walk() == false` on the verified-inert models) plus
//!    the SYSTIMER, I²C0, spi2/apb_saradc, LEDC and — finally — `wifi_mac`
//!    scheduler migrations, the pinner set is EMPTY. `derive_walk_deletable()`
//!    returns true, so the bus flips walk-deleted and `max_safe_tick_interval`
//!    rises above 1 (the campaign payoff), asserted as an exact set so the
//!    report stays honest.

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
use labwired_core::peripherals::esp32c3::apb_saradc::Esp32c3ApbSarAdc;
use labwired_core::peripherals::esp32c3::i2c::Esp32c3I2c;
use labwired_core::peripherals::esp32c3::rtc_timer::Esp32c3RtcTimer;
use labwired_core::peripherals::esp32c3::spi::Esp32c3Spi;
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
/// the SYSTIMER (the FreeRTOS-tick source), `i2c0_legacy` for the I²C0
/// bit-engine (the OLED bus master), and `spi_adc_legacy` for the level-only
/// pair (`spi2` + `apb_saradc`, migrated together), so a walk-on-vs-scheduler
/// differential can isolate each migration in turn.
fn build_oled_lab(
    tick_interval: u32,
    rtc_legacy: bool,
    systimer_legacy: bool,
    i2c0_legacy: bool,
    spi_adc_legacy: bool,
) -> OledLab {
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

    if i2c0_legacy {
        let idx = machine
            .bus
            .find_peripheral_index_by_name("i2c0")
            .expect("oled bus registers i2c0");
        machine.bus.peripherals[idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Esp32c3I2c>()
            .expect("i2c0 is the C3 command-list controller")
            .force_legacy_walk();
    }

    if spi_adc_legacy {
        // Level-only pair: pin both back onto the per-cycle walk so the
        // reference config re-emits their matrix source via `tick()` instead of
        // the scheduler export (`matrix_irq_sources`).
        let spi_idx = machine
            .bus
            .find_peripheral_index_by_name("spi2")
            .expect("oled bus registers spi2");
        machine.bus.peripherals[spi_idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Esp32c3Spi>()
            .expect("spi2 is the C3 GP-SPI2 controller")
            .force_legacy_walk();
        let adc_idx = machine
            .bus
            .find_peripheral_index_by_name("apb_saradc")
            .expect("oled bus registers apb_saradc");
        machine.bus.peripherals[adc_idx]
            .dev
            .as_any_mut()
            .unwrap()
            .downcast_mut::<Esp32c3ApbSarAdc>()
            .expect("apb_saradc is the C3 SAR ADC controller")
            .force_legacy_walk();
    }

    // Any `*_legacy` reference above pinned a peripheral back onto the walk with
    // `force_legacy_walk` AFTER the rom-boot path derived walk-deletion over the
    // (then all-scheduler) peripheral set. Now that `wifi_mac` is migrated the
    // rom-boot derivation reads walk-DELETED, so a pinned-back peripheral would
    // be starved of ticks unless we recompute the flag over the live set (the
    // same recompute the in-crate routing gates do after `force_legacy_walk`).
    machine.bus.recompute_walk_deletable();

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
    let (walk_cycles, walk_fb, walk_serial) =
        run_lab(build_oled_lab(1, true, false, false, false), PAINT_BUDGET);
    let (sched_cycles, sched_fb, sched_serial) =
        run_lab(build_oled_lab(1, false, false, false, false), PAINT_BUDGET);

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
    let (_, fb_1, _) = run_lab(build_oled_lab(1, false, false, false, false), PAINT_BUDGET);
    let (_, fb_64, serial_64) =
        run_lab(build_oled_lab(64, false, false, false, false), PAINT_BUDGET);

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
        let (walk_cycles, walk_fb, walk_serial) = run_lab(
            build_oled_lab(interval, false, true, false, false),
            PAINT_BUDGET,
        );
        let (sched_cycles, sched_fb, sched_serial) = run_lab(
            build_oled_lab(interval, false, false, false, false),
            PAINT_BUDGET,
        );

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

/// I²C0 walk-free gate (the fidelity contract for THIS batch): I²C0 is the OLED
/// bus master — the SSD1306 GDDRAM is painted entirely through its bit-level
/// command-list engine, so the framebuffer is the direct downstream observable
/// of every module tick. Run the REAL demo with I²C0 on the legacy per-cycle
/// walk (reference) vs scheduler-driven (self-perpetuating module-tick events
/// routed through the C3 interrupt matrix), RTC + SYSTIMER scheduler in both so
/// I²C0 is the only variable.
///
/// At interval 1 every observable must be BYTE-IDENTICAL: the module ticks fire
/// at the same cycles, the slave sees the same byte boundaries and the same
/// SDA/SCL waveform, so serial + total_cycles + framebuffer cannot differ. At
/// interval 64 the framebuffer must ALSO be byte-identical: I²C0's bounded-stale
/// `&self` register reads (SR.BUS_BUSY / INT_RAW polled between module-tick
/// events) never change what the engine actually clocks onto the wire, so the
/// painted OLED output is interval-independent. This is the cycle-exact identity
/// that licenses un-pinning I²C0 from the walk.
#[test]
#[ignore = "runs the real C3 bootloader + app (~4x30M steps); run with --release --ignored"]
fn oled_lab_i2c0_walk_on_vs_scheduler_is_byte_identical() {
    // interval 1: full byte-identity (serial + total_cycles + framebuffer).
    let (walk_cycles, walk_fb, walk_serial) =
        run_lab(build_oled_lab(1, false, false, true, false), PAINT_BUDGET);
    let (sched_cycles, sched_fb, sched_serial) =
        run_lab(build_oled_lab(1, false, false, false, false), PAINT_BUDGET);

    assert!(
        lit_pixels(&walk_fb) >= MIN_LIT,
        "reference (I²C0 walk) must paint the OLED at interval 1 (lit={}); serial:\n{}",
        lit_pixels(&walk_fb),
        String::from_utf8_lossy(&walk_serial)
    );
    assert_eq!(
        walk_cycles, sched_cycles,
        "total_cycles must be byte-identical (I²C0 walk vs scheduler) at interval 1"
    );
    assert!(
        walk_serial == sched_serial,
        "serial stream must be byte-identical (I²C0 walk vs scheduler) at interval 1\n\
         --- walk ---\n{}\n--- sched ---\n{}",
        String::from_utf8_lossy(&walk_serial),
        String::from_utf8_lossy(&sched_serial)
    );
    assert!(
        walk_fb == sched_fb,
        "SSD1306 framebuffer must be byte-identical (I²C0 walk vs scheduler) at interval 1"
    );

    // interval 64: bounded-stale reads must leave the painted OLED unchanged.
    let (_, walk_fb_64, _) = run_lab(build_oled_lab(64, false, false, true, false), PAINT_BUDGET);
    let (_, sched_fb_64, _) = run_lab(build_oled_lab(64, false, false, false, false), PAINT_BUDGET);
    assert!(
        walk_fb_64 == sched_fb_64,
        "SSD1306 framebuffer must be byte-identical (I²C0 walk vs scheduler) at interval 64 \
         (bounded-stale reads must not change the painted OLED)"
    );
    assert!(
        sched_fb_64 == sched_fb,
        "I²C0 scheduler framebuffer must be interval-independent (interval 64 == interval 1)"
    );
}

/// spi2 + apb_saradc walk-free gate: the level-only pair migrates together. In
/// THIS lab neither peripheral fires (the OLED demo drives the display over
/// I²C0, never GP-SPI2 or the SAR ADC), so pinning them to the legacy walk
/// (reference) vs scheduler-driven (test) must leave EVERY observable
/// byte-identical — this proves the migration does not perturb the demo. The
/// direct IRQ-delivery identity (where they actually fire) is proven at the bus
/// level by `c3_level_peripheral_matrix_routing` in `bus::tick`; this end-to-end
/// gate proves the quiescent case. RTC + SYSTIMER + I²C0 are scheduler-driven in
/// both configs so the pair is the only variable.
#[test]
#[ignore = "runs the real C3 bootloader + app (~4x30M steps); run with --release --ignored"]
fn oled_lab_spi2_apb_saradc_walk_on_vs_scheduler_is_byte_identical() {
    // interval 1: full byte-identity (serial + total_cycles + framebuffer).
    let (walk_cycles, walk_fb, walk_serial) =
        run_lab(build_oled_lab(1, false, false, false, true), PAINT_BUDGET);
    let (sched_cycles, sched_fb, sched_serial) =
        run_lab(build_oled_lab(1, false, false, false, false), PAINT_BUDGET);

    assert!(
        lit_pixels(&walk_fb) >= MIN_LIT,
        "reference (spi2/apb_saradc walk) must paint the OLED at interval 1 (lit={}); serial:\n{}",
        lit_pixels(&walk_fb),
        String::from_utf8_lossy(&walk_serial)
    );
    assert_eq!(
        walk_cycles, sched_cycles,
        "total_cycles must be byte-identical (spi2/apb_saradc walk vs scheduler) at interval 1"
    );
    assert!(
        walk_serial == sched_serial,
        "serial stream must be byte-identical (spi2/apb_saradc walk vs scheduler) at interval 1"
    );
    assert!(
        walk_fb == sched_fb,
        "SSD1306 framebuffer must be byte-identical (spi2/apb_saradc walk vs scheduler) at interval 1"
    );

    // interval 64: the quiescent pair must still not perturb the painted OLED.
    let (_, walk_fb_64, _) = run_lab(build_oled_lab(64, false, false, false, true), PAINT_BUDGET);
    let (_, sched_fb_64, _) = run_lab(build_oled_lab(64, false, false, false, false), PAINT_BUDGET);
    assert!(
        walk_fb_64 == sched_fb_64,
        "SSD1306 framebuffer must be byte-identical (spi2/apb_saradc walk vs scheduler) at interval 64"
    );
    assert!(
        sched_fb_64 == sched_fb,
        "spi2/apb_saradc scheduler framebuffer must be interval-independent (interval 64 == interval 1)"
    );
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
///   cargo test -p labwired-core --release --features event-scheduler \
///     --test esp32c3_walk_differential oled_lab_native_mips_probe -- --ignored --nocapture
/// Env knobs:
/// - `LABWIRED_MIPS_INTERVAL` — peripheral tick interval (default 1; browser uses 64
///   when walk-free via `recommended_tick_interval`)
/// - `LABWIRED_MIPS_SYSTIMER_LEGACY=1` — pin SYSTIMER on the legacy walk
/// - `LABWIRED_MIPS_IDLE_FF=1` — enable WFI idle fast-forward
/// - `LABWIRED_MIPS_JIT=1` — enable RV32IMC wasm-JIT (requires `--features jit`)
/// - `LABWIRED_MIPS_STEPS` — instruction budget (default 50_000_000)
#[test]
#[ignore = "wall-clock throughput probe; run 3x with --release --nocapture"]
fn oled_lab_native_mips_probe() {
    let interval: u32 = std::env::var("LABWIRED_MIPS_INTERVAL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let systimer_legacy = std::env::var("LABWIRED_MIPS_SYSTIMER_LEGACY").as_deref() == Ok("1");
    let idle_ff = std::env::var("LABWIRED_MIPS_IDLE_FF").as_deref() == Ok("1");
    let jit = std::env::var("LABWIRED_MIPS_JIT").as_deref() == Ok("1");
    let steps_budget: u64 = std::env::var("LABWIRED_MIPS_STEPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(50_000_000);
    let mut lab = build_oled_lab(interval, false, systimer_legacy, false, false);
    lab.machine.config.idle_fast_forward_enabled = idle_ff;
    lab.machine.config.riscv_jit_enabled = jit;
    lab.machine.bus.config.riscv_jit_enabled = jit;
    let start = std::time::Instant::now();
    let mut steps = 0u64;
    while steps < steps_budget {
        let chunk = 1_000_000u32.min((steps_budget - steps) as u32);
        lab.machine.run(Some(chunk)).expect("run oled lab");
        steps += chunk as u64;
    }
    let secs = start.elapsed().as_secs_f64();
    let profile = lab.machine.step_profile();
    let idle_skipped = lab.machine.idle_fast_forward_cycles_skipped;
    eprintln!(
        "oled-lab native: {steps_budget} insn budget, interval {interval}, systimer {}, idle_ff={idle_ff}, jit={jit}: \
         {:.2}s = {:.2} MIPS (total_cycles {}, idle_ff_skipped {}, cpu_instr {}, legacy_tick_entries {})",
        if systimer_legacy { "walk" } else { "scheduler" },
        secs,
        lab.machine.total_cycles as f64 / secs / 1.0e6,
        lab.machine.total_cycles,
        idle_skipped,
        profile.cpu_instructions,
        profile.legacy_tick_entries,
    );
    #[cfg(feature = "jit")]
    if let Some(s) = lab.machine.cpu.jit_stats() {
        eprintln!(
            "  jit stats: compiled={} block_runs={} block_instrs={} interpreted={}",
            s.compiled, s.block_runs, s.block_instrs, s.interpreted
        );
    }
}

/// Endgame ledger: after the RTC migration + the C3/ESP32 Class-A inert
/// sweep, which peripherals still pin the per-cycle walk on the REAL OLED
/// rom-boot bus? Locks in that `rtc_cntl_timer` is walk-independent and that
/// the remaining pinners are EXACTLY the verified real workers — models whose
/// tick genuinely mutates state or asserts a level IRQ from the walk:
///
///   (`systimer` — the free-running counter + FreeRTOS tick alarm —, `i2c0`
///   — the OLED bit-level wire engine —, the level-only pair `spi2` +
///   `apb_saradc`, and now `ledc` are all scheduler-driven and no longer here.
///   The level-only pair export their level via `matrix_irq_sources` with no
///   scheduled events, since their `int_raw` is write-armed by a transaction /
///   conversion. `ledc` is a genuine timer port: its four low-speed up-counters
///   advance lazily off the bus-published `CycleClock` and each `LSTIMERx_OVF`
///   rides a scheduled event that materialises the latch at its exact cycle and
///   re-arms — the SYSTIMER/STM32 TIMx pattern.)
///   - (`wifi_mac` was the LAST pinner and is now migrated too: its MAC
///     interrupt LEVEL is exported through `matrix_irq_sources` + the write-choke
///     re-derivation, and its descriptor-ring PUMP rides the write-armed,
///     self-perpetuating bus-tick path — so `needs_bus_tick()` is false while
///     WiFi is idle and `uses_scheduler()` is true. The OLED demo never enables
///     WiFi, so the pump arms nothing and the bus is fully walk-deletable.)
///
/// Every model on the bus now proves walk-independence itself
/// (`uses_scheduler()` or a verified `needs_legacy_walk() == false`), so
/// `derive_walk_deletable()` returns TRUE, `EXPECTED_PINNERS` is empty, and
/// `max_safe_tick_interval` rises above 1 — the payoff of the campaign. This is
/// the UNLOCK gate: it must stay empty (any model that starts pinning again is a
/// regression).
#[test]
#[ignore = "builds the full rom-boot bus (needs ROM blobs + flash fixture); run with --ignored"]
fn oled_lab_walk_pinners_after_rtc_migration() {
    /// The C3 walk-free campaign is COMPLETE: no verified real worker still
    /// pins the per-cycle walk on the OLED rom-boot bus. A model newly marked
    /// `needs_legacy_walk() == true` (un-migrated) would re-populate this set —
    /// a regression.
    const EXPECTED_PINNERS: &[&str] = &[];

    let lab = build_oled_lab(1, false, false, false, false);
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

    // THE UNLOCK: every worker is migrated, so the OLED bus flips
    // walk-deletable and the tick interval is free to rise. `derive_walk_deletable()`
    // is true iff every peripheral satisfies `uses_scheduler() || !needs_legacy_walk()`
    // — i.e. iff this pinner set is empty — so the empty set IS the derivation
    // going true, and `max_safe_tick_interval() > 1` is its runtime payoff.
    assert!(
        pinners.is_empty(),
        "OLED rom-boot bus must now derive walk-deletion (no real workers remain), \
         but these still pin: {pinners:?}"
    );
    assert!(
        bus.max_safe_tick_interval() > 1,
        "bus must recommend an interval > 1 now that every pinner is migrated (got {})",
        bus.max_safe_tick_interval()
    );
}
