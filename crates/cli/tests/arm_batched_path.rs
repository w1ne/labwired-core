// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! `labwired run --batched` on ARM: the batched orchestration the browser runs.
//!
//! The browser drives every simulated instruction through
//! `Machine::advance(AdvanceRequest::run(..))` (`Sim::step_batch` in
//! crates/wasm), while `labwired run` on ARM drove `Machine::step()`, one
//! instruction per call. That divergence is why the per-board throughput gate
//! (`scripts/perf/board_perf.py`) reported +0.2-0.4% for #830, a change worth
//! 9-16x on the batched path: the gate never entered it.
//!
//! What these tests hold down:
//!   1. `--batched` changes which loop runs, and says so, so a measurement can
//!      prove which path it measured instead of assuming.
//!   2. The default run is untouched — no flag, no marker, no behaviour change.
//!   3. The two loops produce the same firmware-visible output. A faster path
//!      that simulates something else is not a faster path.

use std::path::{Path, PathBuf};
use std::process::Command;

fn labwired_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_labwired"))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// One TIER1 ARM board with a committed fixture ELF. These are skipped rather
/// than failed when the blobs are absent (fresh clone), same as the TIER1
/// matrix harness does.
const BOARDS: &[(&str, &str)] = &[
    ("stm32l476", "configs/chips/stm32l476.yaml"),
    ("stm32f103", "configs/chips/stm32f103.yaml"),
    ("stm32f407", "configs/chips/stm32f407.yaml"),
    ("stm32g474re", "configs/chips/stm32g474re.yaml"),
    ("stm32h563", "configs/chips/stm32h563.yaml"),
    ("stm32l073", "configs/chips/stm32l073.yaml"),
    ("stm32wb55", "configs/chips/stm32wb55.yaml"),
    ("nrf52832", "configs/chips/nrf52832.yaml"),
    ("nrf52840", "configs/chips/nrf52840.yaml"),
    ("rp2040", "configs/chips/rp2040.yaml"),
];

const STEPS: u64 = 300_000;

struct RunOut {
    stdout: String,
    stderr: String,
}

fn run(chip: &Path, elf: &Path, batched: bool) -> RunOut {
    run_with_env(chip, elf, batched, &[])
}

fn run_with_env(chip: &Path, elf: &Path, batched: bool, env: &[(&str, &str)]) -> RunOut {
    let mut cmd = Command::new(labwired_bin());
    cmd.arg("run")
        .arg("--chip")
        .arg(chip)
        .arg("--firmware")
        .arg(elf)
        .arg("--max-steps")
        .arg(STEPS.to_string());
    if batched {
        cmd.arg("--batched");
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn labwired");
    RunOut {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

/// `[batched] instructions=N batches=M steps_per_batch=X ...` -> (N, X).
fn parse_marker(stderr: &str) -> (u64, f64) {
    (
        marker_field(stderr, "instructions=").parse().unwrap(),
        marker_field(stderr, "steps_per_batch=").parse().unwrap(),
    )
}

/// One `key=value` field of the `[batched]` line, by key.
fn marker_field(stderr: &str, key: &str) -> String {
    let line = stderr
        .lines()
        .find(|l| l.starts_with("[batched] "))
        .unwrap_or_else(|| panic!("no [batched] proof line in:\n{stderr}"));
    line.split_whitespace()
        .find_map(|f| f.strip_prefix(key))
        .unwrap_or_else(|| panic!("no {key} field in {line:?}"))
        .to_string()
}

fn fixtures() -> Vec<(&'static str, PathBuf, PathBuf)> {
    let root = workspace_root();
    BOARDS
        .iter()
        .map(|(board, chip)| {
            (
                *board,
                root.join(chip),
                root.join(format!("tests/fixtures/tier1/{board}.elf")),
            )
        })
        .filter(|(_, chip, elf)| chip.exists() && elf.exists())
        .collect()
}

#[test]
fn batched_run_reports_the_loop_it_took_and_retires_every_requested_step() {
    let boards = fixtures();
    if boards.is_empty() {
        eprintln!("SKIP: no TIER1 ARM fixtures present");
        return;
    }
    for (board, chip, elf) in boards {
        let out = run(&chip, &elf, true);
        let (instructions, per_batch) = parse_marker(&out.stderr);
        // The perf gate's slope divides by the REQUESTED step delta. A run that
        // retired fewer instructions than it was asked for would silently
        // inflate Ir/step, so the count is part of the contract, not a stat.
        assert_eq!(
            instructions, STEPS,
            "{board}: batched run retired {instructions} of {STEPS} requested steps"
        );
        assert!(
            per_batch >= 1.0,
            "{board}: nonsensical batch width {per_batch}"
        );
    }
}

#[test]
fn default_run_is_untouched_by_the_flags_existence() {
    let boards = fixtures();
    if boards.is_empty() {
        eprintln!("SKIP: no TIER1 ARM fixtures present");
        return;
    }
    for (board, chip, elf) in boards {
        let out = run(&chip, &elf, false);
        assert!(
            !out.stderr.contains("[batched]"),
            "{board}: default run printed the batched marker; every existing \
             consumer of this stderr just changed"
        );
    }
}

/// Boards whose TIER1 transcript is NOT the same on the batched path.
///
/// EMPTY, and the emptiness is the point — every ARM board in `BOARDS` now
/// produces byte-identical firmware output on both loops.
///
/// It held `nrf52832`, `nrf52840` and `rp2040`, all with the same symptom:
/// `TIER1 timer FAIL code=timer-not-advancing`. Firmware polled a free-running
/// counter (nRF52 TIMER, RP2040 TIMER) in a tight loop and saw it frozen,
/// because `CortexM::step_batch` left `bus.current_cycle` — and the
/// `CycleClock` published in lock-step with it — pinned to the BATCH-START
/// cycle for the whole window, so every lazily-advanced peripheral read the
/// same instant for `peripheral_tick_interval` instructions. Fixed in #842 by
/// republishing the live cycle per retired instruction, which `RiscV::step_batch`
/// had done all along; ARM simply never got that block, which is why no RISC-V
/// board was ever on this list.
///
/// The unit-level twin now lives in
/// `crates/core/tests/nrf52_timer_walk_differential.rs`
/// (`timer0_capture_poll_advances_inside_batch_at_tick512`), which polls the
/// counter from INSIDE a 512-instruction batch — the half that gate was
/// missing when this list was written.
///
/// Keep the list and its two-directional check. This is the seam where a CPU
/// batch loop and a lazily-advanced peripheral disagree about what time it is,
/// and it has gone wrong once already.
const KNOWN_DIVERGENT: &[&str] = &[];

#[test]
fn batching_does_not_change_what_the_firmware_does() {
    let boards = fixtures();
    if boards.is_empty() {
        eprintln!("SKIP: no TIER1 ARM fixtures present");
        return;
    }
    let mut newly_divergent = Vec::new();
    let mut silently_fixed = Vec::new();
    for (board, chip, elf) in boards {
        let stepped = run(&chip, &elf, false);
        let batched = run(&chip, &elf, true);
        // UART bytes echoed to stdout are the firmware's own output, produced by
        // the modelled peripheral in both cases. Byte-identical or the batched
        // path is simulating a different machine.
        let same = stepped.stdout == batched.stdout;
        let known = KNOWN_DIVERGENT.contains(&board);
        if !same && !known {
            newly_divergent.push(format!(
                "{board}:\n  stepped: {:?}\n  batched: {:?}",
                stepped.stdout, batched.stdout
            ));
        }
        // Checked in both directions, so the list cannot rot into a permanent
        // excuse: fixing a board forces its entry to be deleted here.
        if same && known {
            silently_fixed.push(board.to_string());
        }
    }
    assert!(
        newly_divergent.is_empty(),
        "the batched path (what the browser runs) now simulates something else \
         on boards that used to agree:\n{}",
        newly_divergent.join("\n")
    );
    assert!(
        silently_fixed.is_empty(),
        "these boards agree on both paths now — remove them from \
         KNOWN_DIVERGENT so the next regression is caught: {silently_fixed:?}"
    );
}

/// The width is only expected to open up in the feature set the browser ships
/// (`crates/wasm` enables `event-scheduler`). Without it
/// `SystemBus::max_safe_tick_interval` returns 1 by construction and the
/// batched loop correctly runs one instruction per dispatch — which is why the
/// perf gate builds the CLI with that feature before measuring.
#[cfg(feature = "event-scheduler")]
#[test]
fn batching_actually_widens_the_cpu_quantum_in_the_browser_feature_set() {
    // stm32l476 is walk-deletable with nothing non-relaxable on the bus, so its
    // max-safe tick interval is RECOMMENDED_TICK_INTERVAL and its batches must
    // be hundreds of instructions wide. A regression that reintroduces a clamp
    // of the kind #830 removed lands here as 1.00.
    let root = workspace_root();
    let chip = root.join("configs/chips/stm32l476.yaml");
    let elf = root.join("tests/fixtures/tier1/stm32l476.elf");
    if !chip.exists() || !elf.exists() {
        eprintln!("SKIP: stm32l476 TIER1 fixture not present");
        return;
    }
    let (_, per_batch) = parse_marker(&run(&chip, &elf, true).stderr);
    assert!(
        per_batch > 100.0,
        "stm32l476 batched at {per_batch} instructions per dispatch — the CPU \
         quantum is clamped again (see plan_cpu_window)"
    );
}

/// The marker reports what the JIT COULD do, not what the environment asked for.
///
/// `LABWIRED_CORTEX_M_JIT=1` sets a config flag on every build. In a binary
/// without `jit-core` there is no JIT arm in `CortexM::step_batch` to read it,
/// so the run is a pure interpreter run wearing a JIT label — and two such runs,
/// one taken against a `jit-core` binary and one against an `event-scheduler`
/// binary at the same path, differ by 20x for reasons that have nothing to do
/// with the label. That measurement was made, believed, and filed as a JIT win.
/// The `jit=` field is what makes it non-repeatable.
#[test]
fn batched_marker_reports_what_the_jit_could_do_not_what_was_requested() {
    let boards = fixtures();
    if boards.is_empty() {
        eprintln!("SKIP: no TIER1 ARM fixtures present");
        return;
    }
    let (board, chip, elf) = &boards[0];

    let off = run_with_env(chip, elf, true, &[("LABWIRED_CORTEX_M_JIT", "0")]);
    assert_eq!(
        marker_field(&off.stderr, "jit="),
        "off",
        "{board}: LABWIRED_CORTEX_M_JIT=0 must read back as off"
    );

    let on = run_with_env(chip, elf, true, &[("LABWIRED_CORTEX_M_JIT", "1")]);
    let state = marker_field(&on.stderr, "jit=");
    if cfg!(feature = "jit-core") {
        // Compiled in: the honest answers are "on" or one of the idle reasons —
        // never "unavailable", and never silence.
        assert!(
            ["on", "idle(quantum=1)", "idle(cycle-accurate-bus)"].contains(&state.as_str()),
            "{board}: unexpected jit state {state:?} in a jit-core build"
        );
    } else {
        assert_eq!(
            state, "unavailable(no-jit-core)",
            "{board}: a binary with no JIT must say so rather than let \
             LABWIRED_CORTEX_M_JIT=1 imply one ran"
        );
    }
}

/// An interval of 1 must name its cause: this bus, or this binary.
///
/// `SystemBus::max_safe_tick_interval` can only return `RECOMMENDED_TICK_INTERVAL`
/// from inside its `#[cfg(feature = "event-scheduler")]` arm. Without the
/// feature it answers 1 for every board on every chip, the batched loop runs one
/// instruction per dispatch, and `steps_per_batch=1.00` looks exactly like a
/// board that genuinely needs cycle accuracy — 20x apart in throughput, one
/// character apart in the output.
///
/// Asserted against the RUN rather than against `cfg!(feature = "event-scheduler")`
/// on this test: a `cfg!` here would be one more conditional-compilation site on
/// a surface `event_scheduler_cfg_ratchet` exists to shrink, and it would prove
/// less. The properties below are the ones that make the field worth printing,
/// and they hold in either build.
#[test]
fn batched_marker_names_whether_the_build_or_the_bus_capped_the_window() {
    let boards = fixtures();
    if boards.is_empty() {
        eprintln!("SKIP: no TIER1 ARM fixtures present");
        return;
    }
    let mut build_capped = Vec::new();
    let mut bus_capped = Vec::new();
    for (board, chip, elf) in boards {
        let out = run(&chip, &elf, true);
        let cap = marker_field(&out.stderr, "tick_cap=");
        let interval: u32 = marker_field(&out.stderr, "tick_interval=").parse().unwrap();
        let (_, per_batch) = parse_marker(&out.stderr);
        match cap.as_str() {
            "bus" => bus_capped.push(board),
            "build(no-event-scheduler)" => {
                // The binary cannot batch at all, so no board may look like it did.
                assert_eq!(
                    (interval, per_batch),
                    (1, 1.0),
                    "{board}: tick_cap blamed the build, but this run batched \
                     {per_batch} instructions at interval {interval} — the cap \
                     came from somewhere else and the field is now the lie it \
                     was added to remove"
                );
                build_capped.push(board);
            }
            other => panic!("{board}: unknown tick_cap {other:?}"),
        }
        if interval > 1 {
            assert_eq!(
                cap, "bus",
                "{board}: a relaxed interval can only have come from the bus"
            );
        }
    }
    // A build-level cap is not a per-board property: it applies to every board
    // or to none. A split verdict means the field is reporting something else.
    assert!(
        build_capped.is_empty() || bus_capped.is_empty(),
        "tick_cap says the BUILD capped {build_capped:?} but the BUS capped \
         {bus_capped:?} in the same binary"
    );
}
