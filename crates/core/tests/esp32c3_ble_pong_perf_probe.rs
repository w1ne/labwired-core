//! Measurement probe for the two-node BLE Pong lab. **Every test here is
//! `#[ignore]`d and asserts nothing** — it is a harness for deriving numbers,
//! not a gate. Do not add assertions and call it coverage; if a property is
//! worth defending, gate it in `esp32c3_shipped_lab_batch_gate.rs` where the
//! budget file lives.
//!
//! Derived 2026-08-07/08 (release, M-series, `--features event-scheduler`).
//! Output is DETERMINISTIC — three consecutive runs of one binary are
//! byte-identical — so any figure that moves between runs means the TREE
//! changed, not the simulator. See the warning at the bottom.
//!
//! | config                    | wall (2 nodes) | note                        |
//! |---------------------------|----------------|-----------------------------|
//! | 60M cyc, FF on,  1M slice | 1.46 s         | ff_ratio 0.81               |
//! | 60M cyc, FF off, 1M slice | 15.84 s        | **idle FF is worth 10.8x**  |
//! | 96M cyc, FF on,  250k     | ~2.95 s median | main-thread cap             |
//! | 96M cyc, FF on,  16M      | ~3.18 s median | worker cap — **no faster**  |
//!
//! ## Does the SECOND node cost the first one anything? No. (2026-08-09)
//!
//! A browser reading on `nrf52840-ble-lab` — filter the peer out of the lab
//! frame, it stays on the same air but stops advancing, and the survivor goes
//! 2.75 -> 80.7 MIPS — was written up as "a 14.6x per-cycle collapse when two
//! radios advance in step", and pointed the next session at the BLE/air model.
//! On THIS lab that is not what happens. `probe_air_peer_isolation_rounds`
//! times node A's own `run` call, so the peer's CPU time never enters, and
//! measures four configurations interleaved over four rounds:
//!
//! | interleave | pair (MIPS, r0..r3)     | solo (MIPS, r0..r3)     |
//! |------------|-------------------------|-------------------------|
//! | 25 000     | 43.0 29.2 40.0 51.7     | 40.5 45.9 51.0 52.0     |
//! | 100 000    | 52.3 52.0 51.8 52.3     | 52.4 50.7 52.1 52.3     |
//! | 250 000    | 52.5 52.4 51.3 51.7     | 52.4 52.3 52.5 51.6     |
//!
//! Two nodes cost exactly what one costs, at every interleave, and a PRIVATE
//! air per node (`build_node_air`) is indistinguishable from the shared one —
//! so sharing a medium is not a cost either. What the pair does cost is the
//! second node's own wall time: A and B each want ~0.26 s per 96M cycles, so a
//! single thread delivers ~185 Mcyc/s per chip, or 1.15x real time at 160 MHz.
//!
//! ⚠️ **The first configuration in a fresh process reads 2-4x slow.** The first
//! pass at this measured each configuration ONCE, in order, and "found" a 3.4x
//! peer penalty at a 25 000-cycle interleave (1.177 s against 0.345 s). It was
//! cold-start: allocator, page faults, CPU frequency. Interleaved rounds make
//! it vanish. Every probe here now runs a discarded warm-up first, and a single
//! sample of anything on this page is not evidence.
//!
//! Three conclusions that contradict the obvious guesses:
//!
//! 1. The 64x gap between `HEAVY_MAIN_THREAD_MAX_BATCH` (250k) and
//!    `HEAVY_WORKER_MAX_BATCH` (16M) is worth **nothing**. A first reading said
//!    1.21x; measured interleaved over 4 rounds the 16M cap is marginally
//!    SLOWER. The worker keeps the UI thread free; it does not make the engine
//!    faster. Widening the mean batch 3.3x by other means also bought only
//!    ~1.09x, so batch boundaries are simply not where the time goes.
//! 2. Idle FF does not bite until ~6M cycles into boot (0 skipped at 4M).
//!    A browser HUD reading `idle FF 0` on a lab that has only advanced a few
//!    million cycles is reporting health, not a bug.
//! 3. ~~The scheduler, not the CPU, is the cost.~~ **NO LONGER TRUE, and the
//!    reason it changed is the point.** That conclusion came from a `sample`
//!    profile that put `EventScheduler::drain_due_into` (4014) and `::schedule`
//!    (2384) against `RiscV::step` (1094) — 4.7x — because `esp32c3::bt` leaked
//!    live events and blew out the dedup index. The `arm_seq` residency fix
//!    removed the leak. Re-profiled 2026-08-09 on the same lab and workload:
//!
//!    | bucket                | share |
//!    |-----------------------|------:|
//!    | CPU interpret+decode  | 46.8% |
//!    | **I²C model**         | 18.3% |
//!    | IRQ matrix / PMS      |  9.3% |
//!    | bus accessors         |  8.3% |
//!    | scheduler             |  7.9% |
//!    | **BLE (`bt`)**        |  1.6% |
//!
//!    The scheduler is now 7.9% and the BLE model — which this whole file is
//!    named after — is 1.6%. The biggest single avoidable cost is
//!    `Esp32c3I2c::chase` at 15.4%, entered once per I²C wire EDGE (~18 000
//!    scheduler events per 1 KiB OLED frame).
//!
//!    ⚠️ Do NOT rank this by event count. `i2c0` owns 67% of all scheduler arms
//!    but 18% of the wall clock, and deleting the OLED outright buys 1.14x.
//!    Use `labwired_core::profile`, which measures nanoseconds — see
//!    `probe_ble_pong_in_core_profile` below, which cross-checks it against an
//!    external `sample` run of this same probe.
//!
//! ## 2026-08-08: is there a fixed per-call cost of `step_batch` on an
//! ## air-attached chip? NO. Measured, native AND wasm.
//!
//! The rows above stop at a 100 k slice, so they could not see a per-call term
//! at all. `probe_air_step_batch_fixed_cost` sweeps 2 000 → 16 000 000 (a
//! factor of 8 000, bracketing the 2 000-cycle IO-Link wire bound, the 25 000
//! background slice and the 100 000-cycle BLE air bound) and fits
//! `wall/cycle = per_cycle + fixed·(1/slice)`:
//!
//! | build                      | per cycle | fixed per call | 25 k slice |
//! |----------------------------|----------:|---------------:|-----------:|
//! | native, shared air         |  3.088 ns |         159 ns |   0.077 ms |
//! | native, isolated air       |  3.053 ns |         119 ns |   0.076 ms |
//! | wasm, shared air           |  7.813 ns |         722 ns |   0.196 ms |
//! | wasm, isolated air         |  7.843 ns |         304 ns |   0.196 ms |
//!
//! The fixed term is **0.4 % of a 25 000-cycle slice** and Mcyc/s is flat to
//! within 5 % across the whole sweep — and the 5 % runs the WRONG way for a
//! per-call cost in half the rows. Nothing in `Machine::advance` or
//! `WasmSimulator::step_batch` touches the `AirBus` on entry or exit;
//! `attach_lab_air` is a one-time bind. Air traffic is also SPARSE — measured 3
//! BLE PDUs per node per 48 M cycles (`shared_air_frames` below) — so
//! `BleAirBus::receive_from`, O(AIR_DEPTH) under a mutex, is entered a handful
//! of times per second of guest time and costs nothing measurable: shared air
//! vs isolated air differs by 0.4 % per cycle.
//!
//! What the wasm sweep DOES show is that the axis is batch WIDTH, not call
//! width. Same lab, same engine, browser start-up policy NOT applied
//! (`peripheral_tick_interval = 1`, idle FF off — i.e. `mean_batch` 1.00,
//! 250 000 batches per 250 000 cycles): **906–1 116 ns/cycle, ~1.1 Mcyc/s, and
//! still flat in slice width.** With the policy applied: 7.8 ns/cycle,
//! ~127 Mcyc/s. That is a **116x** cliff, and it is entirely per-cycle. A
//! browser reading of ~0.13 MIPS on an air-attached chip is that cliff, not
//! call overhead — which is exactly the failure class
//! `esp32c3_shipped_lab_batch_gate.rs` gates (mean batch width), and the reason
//! it gates width and not wall time.
//!
//! The only per-call work on the browser step path with no native counterpart
//! is `pumpWifiHostNet` (`packages/ui/src/wasm/simulator-bridge.ts`), measured
//! against the real wasm module with empty queues at **541–569 ns/call**. It is
//! now skipped on labs with no `wifi_ap:`, which is every BLE lab.
//!
//! ⚠️ **The table at the top of this file is PRE-`arm_seq`-residency-fix and its
//! wall-clock column is stale.** On this tree `cap_250k`/`cap_16M` at 96 M
//! cycles/node run in **0.50 s** (was ~2.95 s / ~3.18 s), `rtf_pair` 1.192, and
//! the heap reads `max_queued=10 LIVE_HWM [bt=7 systimer=4] ceiling_trips=0`
//! against the `max_queued=792 bt=789 ceiling_trips=3310` recorded below. The
//! RATIOS the three conclusions rest on still hold; the absolute seconds do not.
//!
//! ⚠️ **Never read these numbers off a tree another session is editing.** Three
//! different `max_queued`/`serial_bytes` readings were recorded here before it
//! became clear the cause was uncommitted edits landing and vanishing under the
//! run, not simulator nondeterminism. The clean committed-branch values at 96M
//! are `max_queued=792  bt=789  ceiling_trips=3310  serial_bytes=360
//! mean_batch=99.23`; at 400M, `max_queued=4370  ceiling_trips=22462` (the leak
//! grows with run length, so pick the regime deliberately). Verify with
//! `git status` before quoting anything from this file.
#![cfg(all(feature = "event-scheduler", not(debug_assertions)))]

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

fn bootloader_image(flash: &[u8]) -> ProgramImage {
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

struct Node {
    machine: Machine<RiscV>,
    serial: Arc<Mutex<Vec<u8>>>,
}

fn build_node(flash: &[u8], idle_ff: bool) -> Node {
    build_node_air(flash, idle_ff, None)
}

/// `private_air`: `Some(node_id)` mints this node its OWN `BleAirBus`, so its
/// controller transmits into a medium nobody listens to and hears nothing back.
/// That is the control for "air-attached and TALKING" vs "air-attached and
/// silent" — same firmware, same peripherals, same `bt` block, only the peer's
/// frames removed. `None` leaves the process-global air that
/// `Esp32c3Bt::new()` binds, which is how the two-node probes hear each other.
fn build_node_air(flash: &[u8], idle_ff: bool, private_air: Option<&str>) -> Node {
    let chip = ChipDescriptor::from_file(root().join("../../configs/chips/esp32c3.yaml")).unwrap();
    let manifest =
        SystemManifest::from_file(root().join("../../configs/systems/esp32c3-ble-pong.yaml"))
            .unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    if let Some(node_id) = private_air {
        bus.attach_private_lab_air(node_id);
    }

    let irom = std::fs::read(root().join("roms/esp32c3/esp32c3_rom.bin")).unwrap();
    let drom = std::fs::read(root().join("roms/esp32c3/esp32c3_drom.bin")).unwrap();
    assert!(inject_rom_regions(
        &mut bus,
        &RomImages {
            irom: irom.clone(),
            drom,
        },
    ));
    for (dst, bytes) in c3_rom_data_init_writes(&irom) {
        for (i, b) in bytes.iter().enumerate() {
            let _ = bus.write_u8(dst as u64 + i as u64, *b);
        }
    }

    let serial = Arc::new(Mutex::new(Vec::new()));
    bus.attach_uart_tx_sink(serial.clone(), false);

    let bootloader = bootloader_image(flash);
    let mut machine = build_rom_boot_machine(
        bus,
        flash.to_vec(),
        RomBootOpts {
            pinned_efuse_mac: None,
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
                .unwrap();
        }
    }
    let sp_top = (chip.ram.base + chip.ram.size) as u32;
    machine.cpu.set_sp(sp_top & !0xF);
    machine.cpu.set_pc(bootloader.entry_point as u32);

    let rec = machine.bus.max_safe_tick_interval();
    machine.config.peripheral_tick_interval = rec;
    machine.bus.config.peripheral_tick_interval = rec;
    machine.config.idle_fast_forward_enabled = idle_ff;
    Node { machine, serial }
}

fn probe(label: &str, idle_ff: bool, slice: u32, budget: u64) {
    let flash = std::fs::read(root().join("tests/fixtures/esp32c3-ble-pong-flash.bin")).unwrap();
    let mut a = build_node(&flash, idle_ff);
    let mut b = build_node(&flash, idle_ff);
    a.machine.reset_step_profile();
    b.machine.reset_step_profile();

    let start = std::time::Instant::now();
    let mut fuel = 0u64;
    while fuel < budget {
        let n = slice.min((budget - fuel) as u32);
        for node in [&mut a, &mut b] {
            let _ = node.machine.run(Some(n));
        }
        fuel += u64::from(n);
    }
    let wall = start.elapsed().as_secs_f64();

    for (name, node) in [("A", &a), ("B", &b)] {
        // Batch-width attribution: who armed the wakes that ended the batches.
        let stats = node.machine.sched.stats();
        let at_now = &stats.arms_at_now_per_peripheral;
        let mut owners: Vec<(u64, u64, &str)> = stats
            .arms_per_peripheral
            .iter()
            .enumerate()
            .filter(|(_, n)| **n > 0)
            .map(|(idx, n)| {
                let who = node
                    .machine
                    .bus
                    .peripherals
                    .get(idx)
                    .map(|p| p.name.as_str())
                    .unwrap_or("<out-of-range>");
                (*n, at_now.get(idx).copied().unwrap_or(0), who)
            })
            .collect();
        owners.sort_unstable_by(|x, y| y.0.cmp(&x.0));
        let total_arms: u64 = owners.iter().map(|(n, _, _)| n).sum();
        let top: Vec<String> = owners
            .iter()
            .take(6)
            .map(|(n, z, who)| format!("{who}={n}(at_now={z})"))
            .collect();

        let p = node.machine.step_profile();
        let mean_batch = p.cpu_instructions as f64 / p.cpu_batches.max(1) as f64;
        let total = node.machine.total_cycles.max(1);
        let ff = node.machine.idle_fast_forward_cycles_skipped;
        let console = String::from_utf8_lossy(&node.serial.lock().unwrap().clone()).into_owned();
        eprintln!(
            "PROBE {label} node{name} idle_ff={idle_ff} slice={slice} \
             mean_batch={mean_batch:.2} batches={} interpreted={} total_cycles={total} \
             ff_skipped={ff} ff_ratio={:.4} legacy_tick_entries={} serial_bytes={}",
            p.cpu_batches,
            p.cpu_instructions,
            ff as f64 / total as f64,
            p.legacy_tick_entries,
            console.len(),
        );
        eprintln!(
            "PROBE {label} node{name} ARMS total={total_arms} \
             arms_per_batch={:.2} top=[{}]",
            total_arms as f64 / p.cpu_batches.max(1) as f64,
            top.join(" "),
        );
        eprintln!(
            "PROBE {label} node{name} HEAP max_queued={} max_live_per_periph={} \
             ceiling_trips={} past_clamps={}",
            stats.max_queued_events,
            stats.max_live_events_per_peripheral,
            stats.live_event_ceiling_trips,
            stats.past_schedule_clamps,
        );
        let mut live: Vec<String> = stats
            .max_live_per_peripheral
            .iter()
            .enumerate()
            .filter(|(_, n)| **n > 1)
            .map(|(idx, n)| {
                let who = node
                    .machine
                    .bus
                    .peripherals
                    .get(idx)
                    .map(|p| p.name.as_str())
                    .unwrap_or("<oor>");
                (*n, who)
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|(n, who)| format!("{who}={n}"))
            .collect();
        live.sort();
        eprintln!("PROBE {label} node{name} LIVE_HWM [{}]", live.join(" "));
    }
    let cps = (2.0 * budget as f64) / wall;
    eprintln!(
        "PROBE {label} wall={wall:.2}s two_node_cycles_per_sec={cps:.0} rtf_pair={:.3}",
        (budget as f64 / wall) / 160e6
    );
}

#[test]
#[ignore = "measurement probe, not a gate"]
fn probe_ble_pong_idle_ff() {
    // Same total guest cycles, three batching/FF configurations.
    probe("ff_on_1M", true, 1_000_000, 60_000_000);
    probe("ff_off_1M", false, 1_000_000, 60_000_000);
    probe("ff_on_100k", true, 100_000, 60_000_000);
}

/// A long steady-state window to profile against. Boot is ~6M cycles of the
/// 400M here, so >98% of samples land in the regime that actually matters.
///
/// Run under a sampling profiler:
/// ```text
/// cargo test --release -p labwired-core --features event-scheduler \
///   --test esp32c3_ble_pong_perf_probe -- --ignored probe_ble_pong_profile &
/// sample <pid> 20 -f /tmp/pong.sample
/// ```
#[test]
#[ignore = "measurement probe, not a gate"]
fn probe_ble_pong_profile() {
    probe("profile", true, 1_000_000, 400_000_000);
}

/// Cross-check `labwired_core::profile` against an external `sample` run of
/// the SAME lab. The in-core instrument is only worth having if it agrees with
/// a profiler that shares none of its assumptions.
#[test]
#[ignore = "measurement probe, not a gate"]
fn probe_ble_pong_in_core_profile() {
    let flash = std::fs::read(root().join("tests/fixtures/esp32c3-ble-pong-flash.bin")).unwrap();
    let mut a = build_node(&flash, true);
    let mut b = build_node(&flash, true);
    labwired_core::profile::start();
    let mut fuel = 0u64;
    while fuel < 400_000_000 {
        for node in [&mut a, &mut b] {
            let _ = node.machine.run(Some(1_000_000));
        }
        fuel += 1_000_000;
    }
    labwired_core::profile::stop();
    eprintln!("{}", a.machine.profile_report().render());
}

/// Where in the boot does idle FF first bite? The browser HUD reads
/// `idle FF 0` at ~4M cycles; if native is also 0 there, that reading is
/// evidence of nothing.
#[test]
#[ignore = "measurement probe, not a gate"]
fn probe_ble_pong_ff_onset() {
    for budget in [2_000_000u64, 4_000_000, 8_000_000, 16_000_000, 32_000_000] {
        probe(
            &format!("onset_{}M", budget / 1_000_000),
            true,
            250_000,
            budget,
        );
    }
}

/// A1 as actually shipped: main-thread cap (250k) vs worker cap (16M).
/// Also prints serial so a slice wide enough to break the BLE election shows up
/// as a node that never leaves GUEST.
#[test]
#[ignore = "measurement probe, not a gate"]
fn probe_ble_pong_batch_cap() {
    probe("cap_250k", true, 250_000, 96_000_000);
    probe("cap_16M", true, 16_000_000, 96_000_000);
}

// ──────────────────────────────────────────────────────────────────────────
//  Fixed per-call cost of `step_batch` on an air-attached chip.
//
//  The browser claim under test: a 25 000-cycle slice on a two-C3 BLE lab
//  costs ~196 ms (~0.13 MIPS) while a chip stepped in large batches on the
//  same page runs ~15 MIPS, i.e. `step_batch` carries a large FIXED per-call
//  cost that only air-attached chips pay. The ledger above only ever compared
//  100k / 250k / 1M / 16M — four widths that are all far above the browser's
//  interleave granularity, so it could not see a per-call term at all.
//
//  Model: wall = fixed·calls + per_cycle·cycles, so
//         wall/cycle = per_cycle + fixed·(1/slice)
//  A sweep over 1/slice is therefore a straight line whose SLOPE is the fixed
//  per-call cost and whose INTERCEPT is the honest per-cycle cost. Fitting it
//  is what turns "it feels slow at 25k" into a number.
// ──────────────────────────────────────────────────────────────────────────

/// One (slice, wall) sample: nanoseconds of `Machine::run` per guest cycle.
fn sweep_point(private_air: bool, slice: u32, budget: u64) -> (u64, f64, f64) {
    let flash = std::fs::read(root().join("tests/fixtures/esp32c3-ble-pong-flash.bin")).unwrap();
    let (mut a, mut b) = if private_air {
        (
            build_node_air(&flash, true, Some("solo-a")),
            build_node_air(&flash, true, Some("solo-b")),
        )
    } else {
        (build_node(&flash, true), build_node(&flash, true))
    };
    a.machine.reset_step_profile();
    b.machine.reset_step_profile();

    let mut calls = 0u64;
    let mut fuel = 0u64;
    let mut inside = std::time::Duration::ZERO;
    while fuel < budget {
        let n = slice.min((budget - fuel) as u32);
        for node in [&mut a, &mut b] {
            let t = std::time::Instant::now();
            let _ = node.machine.run(Some(n));
            inside += t.elapsed();
            calls += 1;
        }
        fuel += u64::from(n);
    }
    let cycles = a.machine.total_cycles + b.machine.total_cycles;
    let wall = inside.as_secs_f64();
    let ns_per_cycle = wall * 1e9 / cycles as f64;
    // NON-VACUITY: `air_frames` is the number of BLE PDUs that actually crossed
    // the shared medium. A zero here means the two nodes never talked and every
    // "air-attached" number below would be measuring an unused radio.
    let air_frames = labwired_core::peripherals::ble_air::default_ble_air_bus()
        .trace_snapshot()
        .len();
    eprintln!(
        "SWEEP private_air={private_air} slice={slice} calls={calls} \
         cycles={cycles} wall={wall:.3}s ns/cyc={ns_per_cycle:.4} \
         Mcyc/s={:.2} ff_skipped={} serialA={} serialB={} shared_air_frames={air_frames} \
         max_queued={}",
        cycles as f64 / wall / 1e6,
        a.machine.idle_fast_forward_cycles_skipped + b.machine.idle_fast_forward_cycles_skipped,
        a.serial.lock().unwrap().len(),
        b.serial.lock().unwrap().len(),
        a.machine.sched.stats().max_queued_events,
    );
    (calls, wall, ns_per_cycle)
}

fn fit(label: &str, private_air: bool, budget: u64) {
    // Widths that BRACKET the browser's real interleave granularity (2 000 wire
    // bound, 25 000 background slice, 100 000 BLE air bound) as well as the
    // native regime the ledger already covered.
    const SLICES: [u32; 6] = [2_000, 25_000, 100_000, 250_000, 1_000_000, 16_000_000];
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    for slice in SLICES {
        let (_calls, _wall, ns) = sweep_point(private_air, slice, budget);
        xs.push(1.0 / f64::from(slice));
        ys.push(ns);
    }
    let n = xs.len() as f64;
    let sx: f64 = xs.iter().sum();
    let sy: f64 = ys.iter().sum();
    let sxx: f64 = xs.iter().map(|x| x * x).sum();
    let sxy: f64 = xs.iter().zip(&ys).map(|(x, y)| x * y).sum();
    let slope = (n * sxy - sx * sy) / (n * sxx - sx * sx);
    let intercept = (sy - slope * sx) / n;
    eprintln!(
        "FIT {label} private_air={private_air}: fixed_per_call={:.1} ns  \
         per_cycle={:.4} ns  (fixed cost of a 25k slice = {:.3} ms of {:.3} ms total)",
        slope,
        intercept,
        slope / 1e6,
        (slope + intercept * 25_000.0) / 1e6,
    );
}

/// The measurement the whole investigation turns on.
#[test]
#[ignore = "measurement probe, not a gate"]
fn probe_air_step_batch_fixed_cost() {
    fit("shared_air", false, 48_000_000);
    fit("private_air", true, 48_000_000);
}

// ── Is the second radio a COST, or is it WORK? ────────────────────────────
//
//  The browser reading that started this: filtering the peer out of the lab
//  frame — it stays attached to the same air, it just stops advancing — took
//  the surviving node from 2.75 to 80.7 MIPS. A 14.6x per-cycle collapse that
//  appears only when two radios advance in step.
//
//  That reading cannot distinguish two very different worlds:
//
//    (a) COST. The shared air makes each cycle of the survivor genuinely more
//        expensive — lock traffic, cross-chip event churn, batch truncation.
//        Then there is an engine bug to fix and 14.6x to win.
//    (b) WORK. A node with no peer has nothing to receive, so it sits in WFI
//        and idle fast-forward retires its cycles for free. Connect it and the
//        BLE stack actually runs. Then "MIPS" collapsed because the cycles
//        stopped being free, not because they got slower, and there is nothing
//        to fix here at all.
//
//  Four configurations separate them. Everything is reported for NODE A only,
//  timed around A's own `run` call, so the peer's own CPU time never enters:
//
//    both_shared   A and B both advance, one air        (what ships)
//    both_private  A and B both advance, an air each    (peer runs, no traffic)
//    peer_frozen   only A advances, B built on the same air (the browser test)
//    solo          only A exists
//
//  The discriminator is INSTRUCTIONS per second of wall, not cycles: cycles
//  skipped by idle fast-forward are free, and counting them is exactly how
//  case (b) disguises itself as case (a).
fn isolation(label: &str, step_peer: bool, private_air: bool, slice: u32, budget: u64) {
    let flash = std::fs::read(root().join("tests/fixtures/esp32c3-ble-pong-flash.bin")).unwrap();
    let mut a = build_node_air(&flash, true, private_air.then_some("iso-a"));
    let mut b = build_node_air(&flash, true, private_air.then_some("iso-b"));
    a.machine.reset_step_profile();
    b.machine.reset_step_profile();

    let mut wall_a = std::time::Duration::ZERO;
    let mut fuel = 0u64;
    while fuel < budget {
        let n = slice.min((budget - fuel) as u32);
        let t = std::time::Instant::now();
        let _ = a.machine.run(Some(n));
        wall_a += t.elapsed();
        if step_peer {
            let _ = b.machine.run(Some(n));
        }
        fuel += u64::from(n);
    }

    let wall = wall_a.as_secs_f64();
    let p = a.machine.step_profile();
    let cycles = a.machine.total_cycles.max(1);
    let ff = a.machine.idle_fast_forward_cycles_skipped;
    let stats = a.machine.sched.stats();
    let arms: u64 = stats.arms_per_peripheral.iter().sum();
    // NON-VACUITY: with a shared air and both nodes stepping, frames MUST have
    // crossed. Zero here means the configuration under test never talked and
    // every number on the line is measuring an idle radio.
    let frames = if private_air {
        usize::MAX // per-node airs are not the global bus; not comparable
    } else {
        labwired_core::peripherals::ble_air::default_ble_air_bus()
            .trace_snapshot()
            .len()
    };
    eprintln!(
        "ISO {label:14} step_peer={step_peer} private_air={private_air} \
         wallA={wall:.3}s cyclesA={cycles} instrA={} \
         Mcyc/s={:.2} MIPS={:.2} ff_ratio={:.4} mean_batch={:.1} batches={} \
         arms={arms} max_queued={} serialA={} air_frames={frames}",
        p.cpu_instructions,
        cycles as f64 / wall / 1e6,
        p.cpu_instructions as f64 / wall / 1e6,
        ff as f64 / cycles as f64,
        p.cpu_instructions as f64 / p.cpu_batches.max(1) as f64,
        p.cpu_batches,
        stats.max_queued_events,
        a.serial.lock().unwrap().len(),
    );
}

/// The isolation numbers, but INTERLEAVED over rounds.
///
/// A first pass measured each configuration once, in order, and produced a 3.4x
/// pair/solo gap at a 25 000-cycle interleave. A second pass at half the budget
/// produced no gap at all and a solo curve that was not even monotone in the
/// slice width — which is a measurement telling you it is noise, not a
/// simulator telling you something. Two things were wrong with both passes:
/// one sample per configuration, and configurations run in a fixed order
/// inside one process, so allocator/page-fault warmth and CPU frequency drift
/// are aliased onto the variable under test.
///
/// This runs A/B/A/B for as many rounds as asked and prints every round, so the
/// spread is visible rather than averaged away. Budget stays at 96M — the
/// smaller run never leaves boot (`serialA` ~100 bytes against ~410), and boot
/// is not the regime the lab spends its life in.
#[test]
#[ignore = "measurement probe, not a gate"]
fn probe_air_peer_isolation_rounds() {
    // Discarded: warms the allocator and the CPU so round 0 is data, not a
    // cold start. Its own reading is deliberately not reported.
    isolation("warmup", true, false, 100_000, 96_000_000);
    for slice in [25_000u32, 100_000, 250_000] {
        for round in 0..4 {
            isolation(
                &format!("r{round}_pair_{slice}"),
                true,
                false,
                slice,
                96_000_000,
            );
            isolation(
                &format!("r{round}_solo_{slice}"),
                false,
                false,
                slice,
                96_000_000,
            );
        }
    }
}
