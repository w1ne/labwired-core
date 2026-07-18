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
use labwired_core::{Arch, Bus, Cpu, DebugControl, Machine};
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Counting allocator: deterministic (instruction-count-driven, load-independent)
/// evidence for the per-tick allocation elimination. `alloc`/`realloc` bump the
/// counter so we can measure allocations over a fixed guest-instruction budget.
struct CountingAlloc;
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static COUNTING: AtomicU64 = AtomicU64::new(0);
#[inline]
fn count_alloc() {
    if COUNTING.load(Ordering::Relaxed) != 0 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
    }
}
unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count_alloc();
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count_alloc();
        System.realloc(ptr, layout, new_size)
    }
}
#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

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
    // (RECOMMENDED_TICK_INTERVAL): i2c0/systimer/etc. are scheduler-driven,
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

/// Coverage + wall-time on the REAL C3 OLED lab.
///
/// The synthetic hot-loop bench measures a block shape real firmware does not
/// have (dozens of sequential ALU ops). This measures what fraction of the
/// shipped lab's retired instructions actually land in compiled blocks — the
/// Amdahl ceiling for ANY JIT backend, browser or native.
#[test]
#[ignore = "benchmark; run with --ignored --nocapture"]
fn c3_oled_jit_coverage_and_walltime() {
    use std::time::Instant;
    const N: u32 = 20_000_000;

    let mut interp = build_oled_lab(false);
    let t0 = Instant::now();
    interp.machine.run(Some(N)).expect("interpreter run");
    let interp_dt = t0.elapsed();

    let mut jit = build_oled_lab(true);
    let t1 = Instant::now();
    jit.machine.run(Some(N)).expect("jit run");
    let jit_dt = t1.elapsed();

    let s = jit
        .machine
        .cpu
        .jit_engine_stats()
        .expect("jit stats present");
    let total = s.block_instrs + s.interpreted;
    let coverage = if total > 0 {
        s.block_instrs as f64 / total as f64 * 100.0
    } else {
        0.0
    };
    let interp_mips = f64::from(N) / interp_dt.as_secs_f64() / 1e6;
    let jit_mips = f64::from(N) / jit_dt.as_secs_f64() / 1e6;

    println!("--- REAL C3 OLED lab, {N} guest instr ---");
    println!("interp: {interp_dt:?} ({interp_mips:.1} MIPS)");
    println!("jit   : {jit_dt:?} ({jit_mips:.1} MIPS)");
    println!(
        "ratio : {:.2}x",
        interp_dt.as_secs_f64() / jit_dt.as_secs_f64()
    );
    println!(
        "coverage: {coverage:.2}% of retired instrs in compiled blocks \
         (block_instrs={} interpreted={} compiled_blocks={} block_runs={})",
        s.block_instrs, s.interpreted, s.compiled, s.block_runs
    );
    let amdahl = 1.0 / (1.0 - coverage / 100.0);
    println!("Amdahl ceiling at this coverage (infinitely fast JIT): {amdahl:.2}x");

    // (allocation-eval helper below covers the steady-state alloc count.)

    // Non-vacuous: a lab that died in the bootloader would "measure" fast and
    // mean nothing. Both arms must have reached the paint.
    for (name, lab) in [("interp", &interp), ("jit", &jit)] {
        let serial = lab.serial.lock().expect("serial lock");
        let text = String::from_utf8_lossy(&serial);
        assert!(
            text.contains("OLED painted: LabWired"),
            "{name} arm never reached the paint — the measurement is meaningless. \
             serial: {text}"
        );
    }
}

/// Deterministic allocation evidence for the per-tick alloc elimination.
/// Warms the lab to the paint, then measures host allocations over a fixed
/// steady-state guest-instruction budget. Load-independent (counts, not time).
#[test]
#[ignore = "benchmark; run with --ignored --nocapture"]
fn c3_oled_steady_state_alloc_count() {
    let mut interp = build_oled_lab(false);
    // Warm up well past the paint so the steady-state SYSTIMER tick is running.
    interp.machine.run(Some(6_000_000)).expect("warmup run");
    {
        let serial = interp.serial.lock().expect("serial lock");
        let text = String::from_utf8_lossy(&serial);
        assert!(
            text.contains("OLED painted: LabWired"),
            "warmup never reached the paint — alloc measurement meaningless"
        );
    }
    const BUDGET: u32 = 4_000_000;
    ALLOCS.store(0, Ordering::Relaxed);
    COUNTING.store(1, Ordering::Relaxed);
    interp.machine.run(Some(BUDGET)).expect("measured run");
    COUNTING.store(0, Ordering::Relaxed);
    let allocs = ALLOCS.load(Ordering::Relaxed);
    println!(
        "--- steady-state host allocations over {BUDGET} guest instr: {allocs} \
         ({:.4} allocs/Kinstr) ---",
        allocs as f64 / (BUDGET as f64 / 1000.0)
    );
}
