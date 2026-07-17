// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT
//
// ESP32-WROOM-32 HW-oracle replay tool (chip-model verification, issue #105).
//
// Reads a capture directory produced by `scripts/hw-oracle/esp32_capture.sh`,
// loads the SAME firmware ELF into our `configure_xtensa_esp32` sim, runs it
// for a comparable number of cycles, samples PC at the same step indices,
// re-reads the same checkpoint memory regions, and emits a JSON diff with
// the first divergence point.
//
// This tool requires NO hardware: it operates entirely on captured artifacts
// plus the firmware ELF. Operators run it after `esp32_capture.sh` (which
// does need hardware) or against a sample capture committed to the repo.
//
// Output (printed to stdout, also written to <capture>/diff.json):
//
//   {
//     "schema":          "labwired-hw-oracle/esp32-wroom/diff/v1",
//     "capture_dir":     "...",
//     "elf":             "...",
//     "pc_samples":      256,
//     "pc_first_diverge": { "step": 17, "hw_pc": "0x...", "sim_pc": "0x..." },
//     "mem_pre_match":   true,
//     "mem_post_mismatches": [
//       { "addr": "0x40080000", "hw": "0xdeadbeef", "sim": "0x00000000" },
//       ...
//     ],
//     "summary":         "ok" | "diverged"
//   }
//
// Exit codes:
//   0  Capture loaded and diff produced (regardless of whether trace matched).
//   1  Capture dir malformed / firmware ELF unreadable.

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use labwired_core::bus::SystemBus;
use labwired_core::system::xtensa::configure_xtensa_esp32;
use labwired_core::{Bus, Cpu, Machine};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Capture-side manifest (subset; everything else is ignored).
#[derive(Debug, Deserialize)]
struct OracleManifest {
    #[serde(default)]
    status: String,
    #[serde(default)]
    elf: String,
    /// Informational; not consumed by the diff logic but useful in `Debug`
    /// dumps when a capture/sim disagreement is investigated by hand.
    #[serde(default)]
    #[allow(dead_code)]
    chip: String,
    #[serde(default)]
    pc_samples: u32,
    #[serde(default)]
    pc_sample_interval_ms: u32,
    #[serde(default)]
    checkpoints: Vec<Checkpoint>,
}

#[derive(Debug, Deserialize)]
struct Checkpoint {
    /// Human-readable region name, surfaced only via `Debug`; the diff keys
    /// off `addr` so unrecognised labels still round-trip.
    #[allow(dead_code)]
    label: String,
    /// Hex string, e.g. "0x40080000".
    addr: String,
    words: u32,
}

#[derive(Debug, Serialize)]
struct DiffReport {
    schema: &'static str,
    capture_dir: String,
    elf: String,
    pc_samples: usize,
    pc_first_diverge: Option<PcDiverge>,
    mem_pre_match: bool,
    mem_post_mismatches: Vec<MemMismatch>,
    summary: &'static str,
}

#[derive(Debug, Serialize)]
struct PcDiverge {
    step: usize,
    hw_pc: String,
    sim_pc: String,
}

#[derive(Debug, Serialize)]
struct MemMismatch {
    addr: String,
    hw: String,
    sim: String,
}

#[derive(Parser)]
#[command(
    name = "esp32_replay_in_sim",
    about = "Replay a captured ESP32-WROOM-32 firmware run inside the sim and diff."
)]
struct Args {
    /// Path to the capture directory written by `esp32_capture.sh`.
    #[arg(long)]
    capture: PathBuf,

    /// Override the firmware ELF path (defaults to the one named in oracle.json).
    #[arg(long)]
    elf: Option<PathBuf>,

    /// Cycles to step before sampling each PC point. Defaults to a budget
    /// derived from `pc_sample_interval_ms` assuming 240 MHz CPU clock so
    /// that sim and HW sample the same nominal "wall-clock window."
    #[arg(long)]
    cycles_per_sample: Option<u64>,

    /// Maximum total sim steps (safety cap). Defaults to
    /// `cycles_per_sample * pc_samples * 4`.
    #[arg(long)]
    max_steps: Option<u64>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let report = run(&args)?;
    let pretty = serde_json::to_string_pretty(&report)?;
    println!("{pretty}");
    let out = args.capture.join("diff.json");
    fs::write(&out, &pretty).with_context(|| format!("write diff to {}", out.display()))?;
    eprintln!("[esp32_replay_in_sim] diff written to {}", out.display());
    Ok(())
}

fn run(args: &Args) -> Result<DiffReport> {
    let manifest_path = args.capture.join("oracle.json");
    let manifest_raw = fs::read_to_string(&manifest_path)
        .with_context(|| format!("read manifest {}", manifest_path.display()))?;
    let manifest: OracleManifest = serde_json::from_str(&manifest_raw)
        .with_context(|| format!("parse manifest {}", manifest_path.display()))?;

    // If the HW capture exited early ("no_hardware"), still let the operator
    // run the sim side so they can sanity-check the replay pipeline. We just
    // skip the diff comparison.
    if manifest.status == "no_hardware" {
        eprintln!(
            "[esp32_replay_in_sim] note: capture status='no_hardware' — running sim only, \
             no comparison possible. Connect an ESP32 and re-capture for a real diff."
        );
    }

    let elf_path = match &args.elf {
        Some(p) => p.clone(),
        None => PathBuf::from(&manifest.elf),
    };
    if !elf_path.is_file() {
        bail!(
            "firmware ELF not readable: {} (override with --elf)",
            elf_path.display()
        );
    }

    // ── Sim setup ────────────────────────────────────────────────────────────
    let mut bus = SystemBus::new();
    let cpu = configure_xtensa_esp32(&mut bus);
    bus.refresh_peripheral_index();
    let mut machine = Machine::new(cpu, bus);

    let image = labwired_loader::load_elf(&elf_path)
        .with_context(|| format!("load ELF {}", elf_path.display()))?;
    machine
        .load_firmware(&image)
        .map_err(|e| anyhow!("load_firmware: {e:?}"))?;
    machine.cpu.set_pc(image.entry_point as u32);

    // ── Cycle budgeting ──────────────────────────────────────────────────────
    // Default budget: assume 240 MHz CPU. cycles_per_ms = 240_000.
    // cycles_per_sample = sample_interval_ms * 240_000.
    let pc_samples = manifest.pc_samples.max(1) as usize;
    let cycles_per_sample = args.cycles_per_sample.unwrap_or_else(|| {
        let ms = manifest.pc_sample_interval_ms.max(1) as u64;
        ms * 240_000
    });
    let max_steps = args
        .max_steps
        .unwrap_or((cycles_per_sample * pc_samples as u64).saturating_mul(4));

    eprintln!(
        "[esp32_replay_in_sim] sim plan: {pc_samples} samples × {cycles_per_sample} cycles \
         (max_steps={max_steps})"
    );

    // ── HW-side artifacts ────────────────────────────────────────────────────
    let hw_pc_trace = read_pc_trace(&args.capture.join("pc_trace.tsv"))?;
    let hw_mem_pre = read_mem_snapshot(&args.capture.join("mem_pre.json"))?;
    let hw_mem_post = read_mem_snapshot(&args.capture.join("mem_post.json"))?;

    // ── Sim PC trace ─────────────────────────────────────────────────────────
    let mut sim_pc_trace: Vec<u32> = Vec::with_capacity(pc_samples);
    let mut sim_mem_pre: BTreeMap<u32, u32> = BTreeMap::new();
    let mut sim_mem_post: BTreeMap<u32, u32> = BTreeMap::new();

    // Pre-snapshot: grab the same checkpoint windows BEFORE we step.
    capture_checkpoints(&machine, &manifest.checkpoints, &mut sim_mem_pre);

    let mut steps_taken: u64 = 0;
    for sample in 0..pc_samples {
        let target = (sample as u64 + 1) * cycles_per_sample;
        let target = target.min(max_steps);
        while steps_taken < target {
            if let Err(e) = machine.step() {
                eprintln!(
                    "[esp32_replay_in_sim] sim stopped at step {steps_taken}: \
                     pc=0x{:08x} err={e:?}",
                    machine.cpu.get_pc()
                );
                // Pad the remaining samples with the halt PC so the diff
                // tells the operator exactly when sim divergence started.
                let halt_pc = machine.cpu.get_pc();
                while sim_pc_trace.len() < pc_samples {
                    sim_pc_trace.push(halt_pc);
                }
                break;
            }
            steps_taken += 1;
            if steps_taken >= max_steps {
                break;
            }
        }
        if sim_pc_trace.len() >= pc_samples {
            break;
        }
        sim_pc_trace.push(machine.cpu.get_pc());
        if steps_taken >= max_steps {
            // Cap reached; fill rest with the last observed PC.
            while sim_pc_trace.len() < pc_samples {
                sim_pc_trace.push(machine.cpu.get_pc());
            }
            break;
        }
    }

    capture_checkpoints(&machine, &manifest.checkpoints, &mut sim_mem_post);

    // ── Diff ────────────────────────────────────────────────────────────────
    let mut pc_first_diverge = None;
    if manifest.status != "no_hardware" {
        for (step, &hw_pc) in hw_pc_trace.iter().enumerate() {
            let Some(&sim_pc) = sim_pc_trace.get(step) else {
                break;
            };
            if hw_pc != sim_pc {
                pc_first_diverge = Some(PcDiverge {
                    step,
                    hw_pc: format!("0x{hw_pc:08x}"),
                    sim_pc: format!("0x{sim_pc:08x}"),
                });
                break;
            }
        }
    }

    let mem_pre_match =
        manifest.status == "no_hardware" || mem_maps_equal(&hw_mem_pre, &sim_mem_pre);

    let mem_post_mismatches = if manifest.status == "no_hardware" {
        Vec::new()
    } else {
        diff_mem_maps(&hw_mem_post, &sim_mem_post)
    };

    let summary = if pc_first_diverge.is_none()
        && mem_pre_match
        && mem_post_mismatches.is_empty()
        && manifest.status != "no_hardware"
    {
        "ok"
    } else if manifest.status == "no_hardware" {
        "sim_only"
    } else {
        "diverged"
    };

    Ok(DiffReport {
        schema: "labwired-hw-oracle/esp32-wroom/diff/v1",
        capture_dir: args.capture.display().to_string(),
        elf: elf_path.display().to_string(),
        pc_samples,
        pc_first_diverge,
        mem_pre_match,
        mem_post_mismatches,
        summary,
    })
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn read_pc_trace(path: &Path) -> Result<Vec<u32>> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut out = Vec::new();
    for (lineno, line) in raw.lines().enumerate() {
        if lineno == 0 || line.is_empty() {
            // Skip header / blank.
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 3 {
            continue;
        }
        let pc_hex = cols[2].trim_start_matches("0x");
        let pc = u32::from_str_radix(pc_hex, 16)
            .with_context(|| format!("parse pc '{}' on line {lineno}", cols[2]))?;
        out.push(pc);
    }
    Ok(out)
}

fn read_mem_snapshot(path: &Path) -> Result<BTreeMap<u32, u32>> {
    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let parsed: BTreeMap<String, String> = serde_json::from_str(&raw)
        .with_context(|| format!("parse mem snapshot {}", path.display()))?;
    let mut out = BTreeMap::new();
    for (k, v) in parsed {
        let addr = u32::from_str_radix(k.trim_start_matches("0x"), 16)
            .with_context(|| format!("parse mem snapshot addr '{k}'"))?;
        let word = u32::from_str_radix(v.trim_start_matches("0x"), 16)
            .with_context(|| format!("parse mem snapshot word '{v}'"))?;
        out.insert(addr, word);
    }
    Ok(out)
}

fn capture_checkpoints<C: Cpu>(
    machine: &Machine<C>,
    checkpoints: &[Checkpoint],
    out: &mut BTreeMap<u32, u32>,
) {
    for cp in checkpoints {
        let base = match u32::from_str_radix(cp.addr.trim_start_matches("0x"), 16) {
            Ok(a) => a,
            Err(_) => {
                eprintln!("[esp32_replay_in_sim] bad checkpoint addr {}", cp.addr);
                continue;
            }
        };
        for i in 0..cp.words {
            let addr = base.wrapping_add(i * 4);
            // Best-effort read — unmapped regions just produce 0, which is
            // semantically "sim has no model" and shows up as a diff entry
            // for the operator to investigate.
            let word = machine.bus.read_u32(addr as u64).unwrap_or(0);
            out.insert(addr, word);
        }
    }
}

fn mem_maps_equal(a: &BTreeMap<u32, u32>, b: &BTreeMap<u32, u32>) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for (k, v) in a {
        if b.get(k) != Some(v) {
            return false;
        }
    }
    true
}

fn diff_mem_maps(hw: &BTreeMap<u32, u32>, sim: &BTreeMap<u32, u32>) -> Vec<MemMismatch> {
    let mut out = Vec::new();
    for (&addr, &hw_word) in hw {
        let sim_word = sim.get(&addr).copied().unwrap_or(0);
        if hw_word != sim_word {
            out.push(MemMismatch {
                addr: format!("0x{addr:08x}"),
                hw: format!("0x{hw_word:08x}"),
                sim: format!("0x{sim_word:08x}"),
            });
        }
    }
    out
}
