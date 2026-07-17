// THROWAWAY measurement harness — DO NOT COMMIT.
//! Native profile baseline for the ESP32-C3 OLED lab (the browser fast-start
//! assembly, verbatim from `esp32c3_walk_differential`'s `build_oled_lab`).
//!
//! Measurement only: no semantics change, no timing primitive.

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
use labwired_core::{Arch, Bus, Cpu, DebugControl, Machine, SimulationObserver};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

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

/// Verbatim browser fast-start assembly (chip yaml + system yaml + vendored
/// mask ROM + curated flash, entering at the 2nd-stage bootloader).
fn build_oled_lab() -> OledLab {
    let chip = ChipDescriptor::from_file(root().join("../../configs/chips/esp32c3.yaml"))
        .expect("load esp32c3 chip yaml");
    let manifest =
        SystemManifest::from_file(root().join("../../configs/systems/esp32c3-oled-demo.yaml"))
            .expect("load esp32c3-oled-demo system yaml");
    let mut bus = SystemBus::from_config(&chip, &manifest).expect("build oled bus");

    let irom = std::fs::read(root().join("roms/esp32c3/esp32c3_rom.bin")).expect("read C3 IROM");
    let drom = std::fs::read(root().join("roms/esp32c3/esp32c3_drom.bin")).expect("read C3 DROM");
    // LABWIRED_C3_FLASH lets us point the SAME lab at the flash image the
    // DEPLOYED playground lab actually ships (demo-esp32c3-display-workshop-flash.bin,
    // bundled-configs.ts:894) instead of the test fixture — measurement only.
    let flash_path = std::env::var("LABWIRED_C3_FLASH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root().join("../wasm/tests/fixtures/esp32c3-oled-demo-flash.bin"));
    let flash = std::fs::read(&flash_path)
        .unwrap_or_else(|e| panic!("read C3 flash image {}: {e}", flash_path.display()));

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

    // EXACT browser policy: `apply_browser_c3_policy` (crates/wasm/src/lib.rs:1948)
    // = recommended tick interval + idle fast-forward on.
    let rec = std::env::var("LABWIRED_C3_TICK")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| machine.bus.max_safe_tick_interval());
    machine.config.peripheral_tick_interval = rec;
    machine.bus.config.peripheral_tick_interval = rec;
    machine.config.idle_fast_forward_enabled =
        std::env::var("LABWIRED_C3_IDLE_FF").as_deref() != Ok("0");

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

fn budget() -> u64 {
    std::env::var("LABWIRED_C3_OLED_BUDGET")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30_000_000)
}

/// D1/D2/D8/D10: baseline throughput, RTF, idle-vs-interpreted split.
#[test]
#[ignore = "native C3 workload benchmark; run with --release --ignored"]
fn esp32c3_oled_native_baseline() {
    const CHUNK: u32 = 1_000_000;
    let budget = budget();

    let mut lab = build_oled_lab();
    eprintln!(
        "C3_OLED_SETUP tick_interval={} idle_ff={} walk_deleted={} legacy_entries={}",
        lab.machine.config.peripheral_tick_interval,
        lab.machine.config.idle_fast_forward_enabled,
        lab.machine.bus.max_safe_tick_interval() > 1,
        lab.machine.bus.legacy_tick_entry_descriptors().len(),
    );

    lab.machine.reset_step_profile();
    let started = Instant::now();
    let mut fuel: u64 = 0;
    while fuel < budget {
        let n = CHUNK.min((budget - fuel) as u32);
        lab.machine.run(Some(n)).expect("run C3 OLED");
        fuel += u64::from(n);
    }
    let elapsed = started.elapsed();

    let profile = lab.machine.step_profile();
    let total = lab.machine.total_cycles;
    let idle = lab.machine.idle_fast_forward_cycles_skipped;
    let interpreted = profile.cpu_instructions;
    let fb = ssd1306_framebuffer(&lab.machine);
    let serial_len = lab.serial.lock().unwrap().len();

    let secs = elapsed.as_secs_f64();
    let interp_mips = interpreted as f64 / secs / 1e6;
    let guest_cycles_per_sec = total as f64 / secs;
    let rtf = guest_cycles_per_sec / 160e6;

    eprintln!(
        "C3_OLED_PROFILE wall_s={:.4} total_cycles={} idle_ff_cycles={} interpreted={} \
         idle_pct={:.3} interp_mips={:.3} guest_cycles_per_sec={:.0} rtf={:.4} \
         cpu_batches={} mean_batch={:.3} peripheral_ticks={} peripheral_ticked_entries={} \
         bus_tick_entries={} legacy_tick_entries={} lit_px={} serial_bytes={} pc={:#x}",
        secs,
        total,
        idle,
        interpreted,
        idle as f64 / total as f64 * 100.0,
        interp_mips,
        guest_cycles_per_sec,
        rtf,
        profile.cpu_batches,
        interpreted as f64 / profile.cpu_batches.max(1) as f64,
        profile.peripheral_ticks,
        profile.peripheral_ticked_entries,
        profile.bus_tick_entries,
        profile.legacy_tick_entries,
        lit_pixels(&fb),
        serial_len,
        lab.machine.cpu.pc,
    );
}

// NOTE (D4): the batch-width distribution + clamp attribution reported in the
// write-up required TEMPORARY instrumentation of `crates/core/src/machine/plan.rs`
// (a `batch_probe` module recording width/clamp-reason) and a
// `EventScheduler::next_event_deadline_owner` probe in
// `crates/core/src/sched/event_scheduler.rs`. Those src edits were REVERTED after
// measurement; the patch is preserved outside the repo at
// scratchpad/batch-probe-instrumentation.patch. Re-apply it to reproduce.

/// D8/D5: phase decomposition — RTF and idle share per 1M-fuel chunk, so boot
/// (interpretation-heavy) is separated from steady state (idle-dominated).
#[test]
#[ignore = "phase decomposition; run with --release --ignored"]
fn esp32c3_oled_phase_decomposition() {
    const CHUNK: u32 = 1_000_000;
    let budget = budget();
    let mut lab = build_oled_lab();
    lab.machine.reset_step_profile();

    let mut fuel: u64 = 0;
    let mut prev_total = 0u64;
    let mut prev_idle = 0u64;
    let mut prev_interp = 0u64;
    let mut prev_batches = 0u64;
    let mut first_paint: Option<(u64, u64, f64)> = None;
    let started = Instant::now();
    let mut prev_wall = 0.0f64;

    eprintln!("C3_PHASE chunk fuel wall_ms d_total d_idle d_interp idle_pct chunk_mips chunk_rtf mean_batch lit_px");
    while fuel < budget {
        let n = CHUNK.min((budget - fuel) as u32);
        lab.machine.run(Some(n)).expect("run C3 OLED");
        fuel += u64::from(n);
        let wall = started.elapsed().as_secs_f64();
        let d_wall = wall - prev_wall;
        let p = lab.machine.step_profile();
        let d_total = lab.machine.total_cycles - prev_total;
        let d_idle = lab.machine.idle_fast_forward_cycles_skipped - prev_idle;
        let d_interp = p.cpu_instructions - prev_interp;
        let d_batches = p.cpu_batches - prev_batches;
        let lit = lit_pixels(&ssd1306_framebuffer(&lab.machine));
        if first_paint.is_none() && lit > 0 {
            first_paint = Some((fuel, lab.machine.total_cycles, wall));
        }
        eprintln!(
            "C3_PHASE {:>4} {:>10} {:>8.2} {:>9} {:>9} {:>9} {:>7.2} {:>8.3} {:>7.3} {:>8.2} {:>6}",
            fuel / u64::from(CHUNK),
            fuel,
            d_wall * 1000.0,
            d_total,
            d_idle,
            d_interp,
            d_idle as f64 / d_total.max(1) as f64 * 100.0,
            d_interp as f64 / d_wall / 1e6,
            d_total as f64 / d_wall / 160e6,
            d_interp as f64 / d_batches.max(1) as f64,
            lit,
        );
        prev_total = lab.machine.total_cycles;
        prev_idle = lab.machine.idle_fast_forward_cycles_skipped;
        prev_interp = p.cpu_instructions;
        prev_batches = p.cpu_batches;
        prev_wall = wall;
    }
    if let Some((fuel, cyc, wall)) = first_paint {
        eprintln!(
            "C3_FIRST_PAINT fuel={} total_cycles={} wall_s={:.4} guest_ms={:.3} rtf_to_paint={:.3}",
            fuel,
            cyc,
            wall,
            cyc as f64 / 160e6 * 1000.0,
            (cyc as f64 / 160e6) / wall
        );
    }
}

/// Long-running variant for `sample`: no budget, runs until killed.
/// This is STEADY STATE (post-paint, idle-FF dominated).
#[test]
#[ignore = "sampling target; run with --release --ignored and kill after sampling"]
fn esp32c3_oled_sample_target() {
    let mut lab = build_oled_lab();
    let started = Instant::now();
    let mut fuel: u64 = 0;
    loop {
        lab.machine.run(Some(1_000_000)).expect("run C3 OLED");
        fuel += 1_000_000;
        if fuel.is_multiple_of(500_000_000) {
            eprintln!(
                "C3_OLED_SAMPLE_PROGRESS fuel={} total_cycles={} idle_ff={} wall_s={:.2}",
                fuel,
                lab.machine.total_cycles,
                lab.machine.idle_fast_forward_cycles_skipped,
                started.elapsed().as_secs_f64()
            );
        }
    }
}

/// D3 PRIMARY sampling target: loop the BOOT phase (fuel 0..2M), the only
/// interpretation-bound phase of this workload. Rebuild cost shows as
/// `build_oled_lab` frames in the sample and is excluded when reading it.
#[test]
#[ignore = "boot-loop sampling target; run with --release --ignored and kill after sampling"]
fn esp32c3_oled_boot_loop_sample_target() {
    let boot_fuel: u64 = std::env::var("LABWIRED_C3_BOOT_FUEL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2_000_000);
    let started = Instant::now();
    let mut iters = 0u64;
    loop {
        let mut lab = build_oled_lab();
        let mut fuel = 0u64;
        while fuel < boot_fuel {
            let n = 1_000_000u32.min((boot_fuel - fuel) as u32);
            lab.machine.run(Some(n)).expect("run C3 OLED boot");
            fuel += u64::from(n);
        }
        iters += 1;
        if iters.is_multiple_of(50) {
            eprintln!(
                "C3_BOOT_LOOP iters={} wall_s={:.2} lit_px={}",
                iters,
                started.elapsed().as_secs_f64(),
                lit_pixels(&ssd1306_framebuffer(&lab.machine))
            );
        }
    }
}

#[derive(Debug, Default)]
struct PcHistogram {
    counts: Mutex<HashMap<u32, u64>>,
    total: AtomicU64,
}

impl SimulationObserver for PcHistogram {
    fn on_step_start(&self, pc: u32, _opcode: u32) {
        self.total.fetch_add(1, Ordering::Relaxed);
        *self.counts.lock().unwrap().entry(pc).or_insert(0) += 1;
    }
}

/// D9: attribute interpreted guest instructions by PC.
#[test]
#[ignore = "guest PC attribution; run with --release --ignored"]
fn esp32c3_oled_guest_pc_attribution() {
    const CHUNK: u32 = 1_000_000;
    let budget = budget();
    let mut lab = build_oled_lab();
    let hist = Arc::new(PcHistogram::default());
    lab.machine.observers.push(hist.clone());

    lab.machine.reset_step_profile();
    let mut fuel: u64 = 0;
    while fuel < budget {
        let n = CHUNK.min((budget - fuel) as u32);
        lab.machine.run(Some(n)).expect("run C3 OLED");
        fuel += u64::from(n);
    }

    let counts = hist.counts.lock().unwrap();
    let total = hist.total.load(Ordering::Relaxed);
    let mut by_pc: Vec<(u32, u64)> = counts.iter().map(|(k, v)| (*k, *v)).collect();
    by_pc.sort_by_key(|(_, c)| std::cmp::Reverse(*c));

    // Coarse region buckets for the C3 memory map.
    let mut regions: HashMap<&str, u64> = HashMap::new();
    for (pc, c) in by_pc.iter() {
        let region = match *pc {
            0x4000_0000..=0x4005_FFFF => "mask ROM (irom)",
            0x3FF0_0000..=0x3FFF_FFFF => "drom/data",
            0x4037_0000..=0x403F_FFFF => "IRAM (sram, .text/ISR)",
            0x4200_0000..=0x42FF_FFFF => "flash XIP (app .text)",
            0x3C00_0000..=0x3CFF_FFFF => "flash XIP (rodata)",
            _ => "other",
        };
        *regions.entry(region).or_insert(0) += c;
    }
    let mut regions: Vec<(&str, u64)> = regions.into_iter().collect();
    regions.sort_by_key(|(_, c)| std::cmp::Reverse(*c));

    eprintln!(
        "C3_PC_ATTRIBUTION total_interpreted={} distinct_pcs={} idle_ff_cycles={} total_cycles={}",
        total,
        by_pc.len(),
        lab.machine.idle_fast_forward_cycles_skipped,
        lab.machine.total_cycles
    );
    for (region, c) in &regions {
        eprintln!(
            "C3_PC_REGION {:<24} {:>12}  {:>6.2}%",
            region,
            c,
            *c as f64 / total as f64 * 100.0
        );
    }
    eprintln!("C3_PC_TOP40:");
    let mut cum = 0u64;
    for (pc, c) in by_pc.iter().take(40) {
        cum += c;
        eprintln!(
            "C3_PC_HOT pc={:#010x} count={:>10} pct={:>6.3}% cum={:>6.2}%",
            pc,
            c,
            *c as f64 / total as f64 * 100.0,
            cum as f64 / total as f64 * 100.0
        );
    }
    // How concentrated is the workload?
    for topn in [1usize, 10, 50, 100, 500, 1000] {
        let s: u64 = by_pc.iter().take(topn).map(|(_, c)| *c).sum();
        eprintln!(
            "C3_PC_CONCENTRATION top{:<5} = {:>6.2}%",
            topn,
            s as f64 / total as f64 * 100.0
        );
    }
}

/// TEMP DIAGNOSTIC: which `has_active_work()` arm keeps uart0 re-arming?
#[test]
#[ignore = "diagnostic; run with --release --ignored"]
fn esp32c3_uart0_active_work_arms() {
    use labwired_core::peripherals::uart::Uart;
    let budget = budget();
    let mut lab = build_oled_lab();
    let mut fuel: u64 = 0;
    while fuel < budget {
        let n = 1_000_000u32.min((budget - fuel) as u32);
        lab.machine.run(Some(n)).expect("run C3 OLED");
        fuel += u64::from(n);
    }
    let idx = lab
        .machine
        .bus
        .find_peripheral_index_by_name("uart0")
        .expect("uart0 present");
    let base = lab.machine.bus.peripherals[idx].base;
    let streams = lab.machine.bus.peripherals[idx]
        .dev
        .as_any()
        .and_then(|a| a.downcast_ref::<Uart>())
        .map(|u| u.attached_streams.len())
        .expect("uart0 is a generic Uart");
    // Stm32F1 layout: CR1 @ 0x0C, TXEIE = 1<<7, TCIE = 1<<6.
    let cr1 = lab.machine.bus.read_u32(base + 0x0C).unwrap_or(0);
    eprintln!(
        "C3_UART0_ARMS base={:#x} attached_streams={} cr1={:#010x} txeie_set={} tcie_set={}",
        base,
        streams,
        cr1,
        (cr1 & (1 << 7)) != 0,
        (cr1 & (1 << 6)) != 0,
    );
}
