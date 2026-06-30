// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! `labwired fuzz` differential fuzzing harness.

use crate::*;

pub(crate) fn run_fuzz(args: FuzzArgs) -> ExitCode {
    use labwired_fuzz::{fuzz, fuzz_collect, Contract, Target, Verdict};

    let contract = Contract {
        input_len: args.input_len_addr,
        input_data: args.input_data_addr,
        verdict: args.verdict_addr,
        done_magic: args.done_magic,
        fault_magic: args.fault_magic,
    };

    // Seeds: parse `--seed-input` hex bytes; empty means the engine self-seeds.
    let mut seeds: Vec<Vec<u8>> = Vec::new();
    for s in &args.seed_input {
        let t = s.trim_start_matches("0x");
        if t.len() % 2 != 0 {
            eprintln!("error: --seed-input `{s}` must be an even number of hex digits");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
        let mut bytes = Vec::with_capacity(t.len() / 2);
        for i in (0..t.len()).step_by(2) {
            match u8::from_str_radix(&t[i..i + 2], 16) {
                Ok(b) => bytes.push(b),
                Err(e) => {
                    eprintln!("error: --seed-input `{s}`: {e}");
                    return ExitCode::from(EXIT_CONFIG_ERROR);
                }
            }
        }
        seeds.push(bytes);
    }

    let target = match Target::from_elf(
        &args.chip,
        &args.system,
        &args.firmware,
        contract,
        args.max_steps,
    ) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {e:#}");
            return ExitCode::from(EXIT_CONFIG_ERROR);
        }
    };

    let engine = if cfg!(feature = "fuzz-libafl") {
        "LibAFL"
    } else {
        "built-in"
    };
    eprintln!(
        "fuzzing {} with the {engine} engine (max_iters={}, seed={:#x}) ...",
        args.firmware.display(),
        args.max_iters,
        args.seed
    );

    // Collect-N mode gathers distinct crashes (feeds HIL-confirm); default mode
    // stops at the first crash.
    let crashes: Vec<Vec<u8>> = if let Some(n) = args.collect {
        fuzz_collect(&target, seeds, args.max_iters, args.seed, n)
    } else {
        match fuzz(&target, seeds, args.max_iters, args.seed) {
            r @ labwired_fuzz::FuzzReport { crash: None, .. } => {
                println!(
                    "no crash in {} iters (corpus {}, {} edges)",
                    r.iterations, r.corpus_size, r.edges_hit
                );
                return ExitCode::SUCCESS;
            }
            labwired_fuzz::FuzzReport {
                crash: Some(c),
                iterations,
                corpus_size,
                edges_hit,
            } => {
                println!(
                    "CRASH in {iterations} iters (corpus {corpus_size}, {edges_hit} edges): {:02X?}",
                    c
                );
                vec![c]
            }
        }
    };

    if crashes.is_empty() {
        println!("no crash found in {} iters", args.max_iters);
        return ExitCode::SUCCESS;
    }

    if args.collect.is_some() {
        println!("found {} distinct crash(es):", crashes.len());
        for c in &crashes {
            println!("  {c:02X?}");
        }
    }

    // Reproduce + report the first crash's verdict for clarity.
    let mut cov = labwired_fuzz::CovMap::new();
    let verdict = target.run(&crashes[0], &mut cov);
    let label = match verdict {
        Verdict::Crash => "crash (fault/panic marker)",
        Verdict::Hang => "hang (step budget exhausted)",
        Verdict::Clean => "clean (non-deterministic?)",
    };
    eprintln!("first crash reproduces as: {label}");

    if let Some(out) = &args.crashes_out {
        match serde_json::to_string_pretty(&crashes) {
            Ok(json) => {
                if let Err(e) = std::fs::write(out, json) {
                    eprintln!("error: write {}: {e}", out.display());
                    return ExitCode::FAILURE;
                }
                eprintln!(
                    "wrote {} crash input(s) to {}",
                    crashes.len(),
                    out.display()
                );
            }
            Err(e) => {
                eprintln!("error: serialize crashes: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    // A crash is a finding — non-zero exit so CI fails the build.
    ExitCode::FAILURE
}
