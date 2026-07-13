// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Chunk H MERGE GATE: the RV32IMC wasm-JIT, wired into `Machine<RiscV>`'s
//! production dispatch, must be **byte-identical** to the interpreter on a
//! REAL ESP32-C3 firmware run.
//!
//! This boots the exact `esp32c3-oled-demo` lab the browser fast-start uses
//! (real 2nd-stage bootloader → SHA verify → app driving an SSD1306 OLED over
//! I²C0, plus the SYSTIMER FreeRTOS tick source) and runs it TWICE from the
//! same reset:
//!   * `riscv_jit_enabled = false` — the interpreter (the oracle), and
//!   * `riscv_jit_enabled = true`  — hot basic blocks compiled to wasm and
//!     retired atomically.
//!
//! After every 1M-instruction chunk (and at the end) it asserts FULL
//! architectural + observable state is identical: every x-register + pc, the
//! CLINT `mtime`/`mtimecmp`, all eight M-mode CSRs and `mip`/`mie`, the
//! reservation, `total_cycles`, the serial (UART) stream, AND the SSD1306
//! framebuffer. It further asserts the JIT path was NON-VACUOUS (compiled
//! blocks actually ran and retired instructions).
//!
//! This exercises ALU + memory + branch heavily (FreeRTOS + display driver)
//! AND peripherals (I²C0, SYSTIMER, UART) through the same `Machine::run`
//! batch/tick/IRQ machinery production uses — so a byte-identical result is a
//! true statement that enabling the JIT changes nothing observable.
//!
//! Heavy (~30M instructions × 2 runs): `#[ignore]`d like the sibling real-C3
//! gates; run as the merge bar with
//! `cargo test -p labwired-core --features jit,event-scheduler --release
//!  --ignored jit_vs_interpreter`.

#![cfg(all(feature = "jit", feature = "event-scheduler"))]

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

/// Build the OLED lab exactly as the browser fast-start (and the sibling
/// `esp32c3_walk_differential` gate) does — real chip/system yaml, vendored
/// mask-ROM blobs, the curated flash image, entering at the 2nd-stage
/// bootloader. Both differential arms use the identical default (scheduler)
/// peripheral configuration; the ONLY difference between the two machines is
/// `config.riscv_jit_enabled`, set by the caller.
fn build_oled_lab(jit_enabled: bool) -> OledLab {
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
                drom,
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

    // Run at the C3's walk-deletable production tick interval
    // (RECOMMENDED_TICK_INTERVAL = 64): i2c0/systimer/etc. are scheduler-driven,
    // so peripherals need not tick every cycle. BOTH arms use this identical
    // interval. This is deliberately MODEST — well below the JIT's 1024-instr
    // block size — to prove Fix 2 engages the JIT: with the batch budget keyed
    // off the tick interval, a 1024-instr block would never fit in a 64-instr
    // batch (`retired + n > max_count`) and the JIT would interpret everything;
    // keying it off the next-scheduled-event distance (walk-deletable → batching
    // is a performance knob, not a correctness boundary) lets full blocks retire
    // atomically between events while staying byte-identical to the interpreter.
    const BATCH_TICK_INTERVAL: u32 = labwired_core::bus::RECOMMENDED_TICK_INTERVAL;
    machine.config.peripheral_tick_interval = BATCH_TICK_INTERVAL;
    machine.bus.config.peripheral_tick_interval = BATCH_TICK_INTERVAL;

    // The sole differential variable.
    machine.config.riscv_jit_enabled = jit_enabled;
    machine.bus.config.riscv_jit_enabled = jit_enabled;

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

/// Flatten the full architectural state that must be identical between the two
/// arms: x0..x31, pc, the CLINT `mtime`/`mtimecmp`, every M-mode CSR incl.
/// `mip`/`mie`, and the LR/SC reservation. The cycle CSRs (0xC00/0x802/0x7E2)
/// are a pure function of `mtime` (× CYCLE_SCALE), so comparing `mtime`
/// proves them identical too.
fn arch_state(cpu: &RiscV) -> Vec<u64> {
    let mut v = Vec::with_capacity(32 + 1 + 2 + 8 + 1);
    v.extend(cpu.x.iter().map(|&w| w as u64));
    v.push(cpu.pc as u64);
    v.push(cpu.mtime);
    v.push(cpu.mtimecmp);
    v.push(cpu.mstatus as u64);
    v.push(cpu.mie as u64);
    v.push(cpu.mip as u64);
    v.push(cpu.mtvec as u64);
    v.push(cpu.mscratch as u64);
    v.push(cpu.mepc as u64);
    v.push(cpu.mcause as u64);
    v.push(cpu.mtval as u64);
    v.push(cpu.reservation.map_or(u64::MAX, |a| a as u64));
    v
}

/// Human-readable name for each `arch_state` index (for a precise divergence
/// message).
fn arch_field_name(i: usize) -> String {
    match i {
        0..=31 => format!("x{i}"),
        32 => "pc".into(),
        33 => "mtime".into(),
        34 => "mtimecmp".into(),
        35 => "mstatus".into(),
        36 => "mie".into(),
        37 => "mip".into(),
        38 => "mtvec".into(),
        39 => "mscratch".into(),
        40 => "mepc".into(),
        41 => "mcause".into(),
        42 => "mtval".into(),
        43 => "reservation".into(),
        _ => format!("field{i}"),
    }
}

fn assert_state_identical(interp: &Machine<RiscV>, jit: &Machine<RiscV>, retired: u64) {
    // total_cycles: the batch/tick/IRQ accounting must match instruction-for-
    // instruction — the strongest single signal that timing stayed exact.
    assert_eq!(
        interp.total_cycles, jit.total_cycles,
        "total_cycles diverged after ~{retired}M instructions: interp={} jit={}",
        interp.total_cycles, jit.total_cycles
    );

    let a = arch_state(&interp.cpu);
    let b = arch_state(&jit.cpu);
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(
            x,
            y,
            "arch state diverged at {} after ~{retired}M instructions: interp={x:#x} jit={y:#x}",
            arch_field_name(i)
        );
    }

    let fb_i = ssd1306_framebuffer(interp);
    let fb_j = ssd1306_framebuffer(jit);
    assert!(
        fb_i == fb_j,
        "SSD1306 framebuffer diverged after ~{retired}M instructions \
         (interp {} lit px, jit {} lit px)",
        lit_pixels(&fb_i),
        lit_pixels(&fb_j)
    );
}

/// THE MERGE GATE. Boot the real C3 OLED lab twice (interpreter vs JIT) and
/// prove byte-identity of the full state along the way and at the paint
/// milestone, plus a non-vacuous JIT.
#[test]
#[ignore = "real C3 bootloader + app, ~30M steps x2; run with --release --ignored"]
fn jit_vs_interpreter_c3_oled_is_byte_identical_and_non_vacuous() {
    // Enough to boot through the 2nd-stage bootloader + app and paint the
    // wordmark; the wasm gate paints well inside this budget.
    const BUDGET: u64 = 30_000_000;
    const CHUNK: u32 = 1_000_000;
    const MIN_LIT: usize = 400;

    let mut interp = build_oled_lab(false);
    let mut jit = build_oled_lab(true);

    let mut steps: u64 = 0;
    while steps < BUDGET {
        let n = CHUNK.min((BUDGET - steps) as u32);
        interp.machine.run(Some(n)).expect("interpreter run");
        jit.machine.run(Some(n)).unwrap_or_else(|e| {
            panic!(
                "jit run failed at pc={:#x} (interp pc={:#x}, retired ~{steps}): {e:?}",
                jit.machine.cpu.pc, interp.machine.cpu.pc
            )
        });
        steps += n as u64;
        assert_state_identical(&interp.machine, &jit.machine, steps / 1_000_000);
    }

    // The serial (UART) stream is an event observable — must be byte-identical.
    let serial_i = interp.serial.lock().unwrap().clone();
    let serial_j = jit.serial.lock().unwrap().clone();
    assert!(
        serial_i == serial_j,
        "UART serial stream diverged (interp {} bytes, jit {} bytes)",
        serial_i.len(),
        serial_j.len()
    );

    // The reference (interpreter) run must actually have painted the OLED, so
    // "identical" is not "both blank".
    let fb = ssd1306_framebuffer(&interp.machine);
    assert!(
        lit_pixels(&fb) >= MIN_LIT,
        "reference run must paint the OLED wordmark ({} lit px < {MIN_LIT})",
        lit_pixels(&fb)
    );

    // NON-VACUOUS: the JIT arm genuinely compiled hot blocks and retired real
    // instructions through them (not pure interpreter fallback).
    let stats = jit
        .machine
        .cpu
        .jit_stats()
        .expect("jit engine was created (JIT ran)");
    assert!(
        stats.compiled > 0,
        "no basic block was ever compiled — the JIT path is vacuous"
    );
    assert!(
        stats.block_runs > 0,
        "no compiled block was ever dispatched — the JIT path is vacuous"
    );
    assert!(
        stats.block_instrs > 0,
        "compiled blocks retired zero instructions — the JIT path is vacuous"
    );
    eprintln!(
        "[jit-gate] byte-identical over {BUDGET} instructions; JIT stats: \
         compiled={} block_runs={} block_instrs={} interpreted={}",
        stats.compiled, stats.block_runs, stats.block_instrs, stats.interpreted
    );
}
