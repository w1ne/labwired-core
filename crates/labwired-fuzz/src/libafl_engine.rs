// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! LibAFL-backed fuzzing engine (Phase 2).
//!
//! Swaps the crate's built-in coverage-guided loop for a real [LibAFL] fuzzer:
//! havoc/splice mutators, a map-feedback queue scheduler, and crash objectives.
//! The simulator is wrapped as an in-process executor — `Target::run` fills an
//! AFL-style edge bitmap that a `StdMapObserver` watches, and a sim CPU fault
//! (or the firmware FAULT marker) is reported as `ExitKind::Crash`, which
//! `CrashFeedback` routes into the solutions corpus.
//!
//! Gated behind the `libafl` feature so the default workspace/CI build stays
//! light. When enabled, [`crate::fuzz`] / [`crate::fuzz_collect`] delegate here.
//!
//! [LibAFL]: https://github.com/AFLplusplus/LibAFL

use crate::{CovMap, Target, Verdict, MAP_SIZE};
use core::ptr::addr_of_mut;

use libafl::corpus::{Corpus, InMemoryCorpus};
use libafl::events::SimpleEventManager;
use libafl::executors::{ExitKind, InProcessExecutor};
use libafl::feedbacks::{CrashFeedback, MaxMapFeedback};
use libafl::inputs::{BytesInput, HasTargetBytes};
use libafl::monitors::SimpleMonitor;
use libafl::mutators::{havoc_mutations, HavocScheduledMutator};
use libafl::observers::StdMapObserver;
use libafl::schedulers::QueueScheduler;
use libafl::stages::StdMutationalStage;
use libafl::state::{HasCorpus, HasSolutions, StdState};
use libafl::{Evaluator, Fuzzer, StdFuzzer};
use libafl_bolts::rands::StdRand;
use libafl_bolts::tuples::tuple_list;
use libafl_bolts::AsSlice;

/// Shared edge-coverage bitmap the observer watches. Single-threaded fuzzing,
/// so a process-global map is fine (LibAFL's `StdMapObserver` wants a stable
/// backing buffer that outlives the executor).
static mut COVERAGE: [u8; MAP_SIZE] = [0u8; MAP_SIZE];

/// Run LibAFL against `target` until `want` distinct crashes land in the
/// solutions corpus or `max_iters` mutational rounds elapse. Returns the
/// crashing inputs. Deterministic for a fixed `seed`.
fn run(
    target: &Target,
    seeds: Vec<Vec<u8>>,
    max_iters: usize,
    seed: u64,
    want: usize,
) -> Vec<Vec<u8>> {
    // SAFETY: single-threaded; the observer + harness are the only users and
    // the executor below borrows neither across threads.
    let observer = unsafe {
        StdMapObserver::from_mut_ptr("edges", addr_of_mut!(COVERAGE) as *mut u8, MAP_SIZE)
    };

    let mut feedback = MaxMapFeedback::new(&observer);
    let mut objective = CrashFeedback::new();

    // The harness: run one input in the sim, publish its edge map, classify.
    let mut harness = |input: &BytesInput| {
        let bytes = input.target_bytes();
        let mut local = CovMap::new();
        let verdict = target.run(bytes.as_slice(), &mut local);
        // SAFETY: single-threaded; overwrite (not accumulate) this run's map.
        // Write through a raw pointer to avoid a reference to the mutable static.
        unsafe {
            core::slice::from_raw_parts_mut(addr_of_mut!(COVERAGE) as *mut u8, MAP_SIZE)
                .copy_from_slice(&local.0);
        }
        match verdict {
            Verdict::Crash => ExitKind::Crash,
            _ => ExitKind::Ok,
        }
    };

    let mut state = StdState::new(
        StdRand::with_seed(seed),
        InMemoryCorpus::<BytesInput>::new(),
        InMemoryCorpus::<BytesInput>::new(),
        &mut feedback,
        &mut objective,
    )
    .expect("libafl state");

    let monitor = SimpleMonitor::new(|_s| {});
    let mut mgr = SimpleEventManager::new(monitor);
    let scheduler = QueueScheduler::new();
    let mut fuzzer = StdFuzzer::new(scheduler, feedback, objective);

    let mut executor = InProcessExecutor::new(
        &mut harness,
        tuple_list!(observer),
        &mut fuzzer,
        &mut state,
        &mut mgr,
    )
    .expect("libafl executor");

    // Prime the corpus with the seeds (fall back to a single zero byte).
    let seed_inputs: Vec<BytesInput> = if seeds.is_empty() {
        vec![BytesInput::new(vec![0u8])]
    } else {
        seeds.into_iter().map(BytesInput::new).collect()
    };
    for inp in seed_inputs {
        let _ = fuzzer.evaluate_input(&mut state, &mut executor, &mut mgr, &inp);
    }
    // A scheduler needs at least one corpus entry even if no seed was novel.
    if state.corpus().count() == 0 {
        let _ = fuzzer.add_input(
            &mut state,
            &mut executor,
            &mut mgr,
            BytesInput::new(vec![0u8]),
        );
    }

    let mutator = HavocScheduledMutator::new(havoc_mutations());
    let mut stages = tuple_list!(StdMutationalStage::new(mutator));

    for _ in 0..max_iters {
        if state.solutions().count() >= want {
            break;
        }
        if fuzzer
            .fuzz_one(&mut stages, &mut executor, &mut state, &mut mgr)
            .is_err()
        {
            break;
        }
    }

    // Extract the crashing inputs from the solutions corpus.
    let mut out = Vec::new();
    let solutions = state.solutions();
    for id in solutions.ids() {
        if let Ok(tc) = solutions.get(id) {
            if let Some(input) = tc.borrow().input() {
                out.push(input.target_bytes().as_slice().to_vec());
            }
        }
    }
    out
}

/// Coverage-guided fuzz with LibAFL; return the first crashing input found.
pub fn fuzz_libafl(
    target: &Target,
    seeds: Vec<Vec<u8>>,
    max_iters: usize,
    seed: u64,
) -> Option<Vec<u8>> {
    run(target, seeds, max_iters, seed, 1).into_iter().next()
}

/// Coverage-guided fuzz with LibAFL; collect up to `max_crashes` distinct
/// crashing inputs.
pub fn fuzz_collect_libafl(
    target: &Target,
    seeds: Vec<Vec<u8>>,
    max_iters: usize,
    seed: u64,
    max_crashes: usize,
) -> Vec<Vec<u8>> {
    run(target, seeds, max_iters, seed, max_crashes)
}
