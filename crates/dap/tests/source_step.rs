// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
//
// This software is released under the MIT License.
// See the LICENSE file in the project root for full license information.

//! Source-level debugging through the native adapter, on the same committed
//! DWARF fixtures and the same expected stops as the browser's source-debug
//! tests: a breakpoint on a source line, run to it, step over / into / out by
//! line, read the stop's file:line + function and its locals.
//!
//! Every step over here must land on a DIFFERENT line than it started on: a
//! step that returns without moving would pass a "reason is line_changed"
//! check on its own and still be useless to a user.

use labwired_dap::adapter::LabwiredAdapter;
use labwired_loader::source_step::{StepKind, StepStop};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(system: &str, firmware: &str) -> LabwiredAdapter {
    let adapter = LabwiredAdapter::new();
    adapter
        .load_firmware(root().join(firmware), Some(root().join(system)))
        .unwrap_or_else(|e| panic!("load {firmware} on {system}: {e:#}"));
    assert!(adapter.has_source(), "{firmware} carries a line table");
    adapter
}

/// (file name, line, function) where the debugged core stopped.
fn here(adapter: &LabwiredAdapter) -> (String, u32, String) {
    let pc = adapter.get_pc().unwrap();
    let loc = adapter
        .source_location(pc)
        .unwrap_or_else(|| panic!("{pc:#x} has no source location"));
    (
        loc.file.rsplit('/').next().unwrap().to_string(),
        loc.line,
        loc.function.unwrap_or_default(),
    )
}

/// Continue in engine-sized chunks until a breakpoint stops the run.
fn run_to_breakpoint(adapter: &LabwiredAdapter, max_cycles: u64) -> u32 {
    let mut ran = 0u64;
    loop {
        let reason = adapter.continue_execution_chunk(1_000_000).unwrap();
        if let labwired_core::StopReason::Breakpoint(_) = reason {
            return adapter.get_pc().unwrap();
        }
        ran += 1_000_000;
        assert!(ran < max_cycles, "no breakpoint within {ran} steps");
    }
}

fn step(adapter: &LabwiredAdapter, kind: StepKind) -> (StepStop, u32) {
    let out = adapter
        .step_source(kind, 2_000_000)
        .unwrap()
        .expect("source step");
    (out.stop, out.pc)
}

fn set_line_breakpoint(adapter: &LabwiredAdapter, file: &str, line: i64) {
    let res = adapter.set_breakpoints(file.into(), vec![line]).unwrap();
    assert!(
        res[0].verified,
        "{file}:{line} resolves: {:?}",
        res[0].message
    );
    assert_eq!(res[0].resolved_line, Some(line as u32));
}

/// Classic ESP32 Arduino sketch (`tests/fixtures/source-debug/main.ino`),
/// dual-core: the adapter follows whichever core the breakpoint stopped.
#[test]
fn esp32_arduino_breakpoint_step_over_into_out_and_locals() {
    let adapter = load(
        "configs/systems/esp32-wroom-32.yaml",
        "tests/fixtures/source-debug/esp32-arduino.elf",
    );
    set_line_breakpoint(&adapter, "src/main.ino", 27);
    assert_eq!(run_to_breakpoint(&adapter, 400_000_000), 0x400d_147a);
    assert_eq!(here(&adapter), ("main.ino".into(), 27, "loop()".into()));
    // a1 is read from the core that stopped: a DRAM stack.
    let sp = adapter.get_register(1).unwrap();
    assert!((0x3ff0_0000..0x4000_0000).contains(&sp), "a1 {sp:#x}");

    // 27 `int n = bump(ticks);` -> 28: bump and twice run inside the step.
    assert_eq!(
        step(&adapter, StepKind::Over),
        (StepStop::LineChanged, 0x400d_1485)
    );
    assert_eq!(here(&adapter), ("main.ino".into(), 28, "loop()".into()));
    // `n`, bump's result, is live in a register on line 28.
    let locals = adapter.source_locals().unwrap();
    let n = locals.iter().find(|l| l.name == "n").expect("local n");
    assert_eq!(n.kind, "register");
    assert!(n.value.is_some());
    // 28 Serial.println -> 29 digitalWrite: one step each.
    assert_eq!(step(&adapter, StepKind::Over).0, StepStop::LineChanged);
    assert_eq!(here(&adapter).1, 29);

    // Continue: the next loop() iteration stops on 27 again.
    assert_eq!(run_to_breakpoint(&adapter, 400_000_000), 0x400d_147a);
    assert_eq!(here(&adapter).1, 27);
    // Into bump, whose parameter `v` is in a2 at its first statement.
    let (stop, pc) = step(&adapter, StepKind::Into);
    assert_eq!(
        (stop, pc),
        (StepStop::LineChanged, 0x400d_1428),
        "bump's entry"
    );
    assert_eq!(here(&adapter).2, "bump(int)");
    let locals = adapter.source_locals().unwrap();
    let v = locals.iter().find(|l| l.name == "v").expect("parameter v");
    assert_eq!((v.kind, v.register), ("register", Some(2)));
    assert!(v.value.is_some());
    // Out: back in loop(), past the call, on 28.
    assert_eq!(step(&adapter, StepKind::Out).0, StepStop::LineChanged);
    assert_eq!(here(&adapter), ("main.ino".into(), 28, "loop()".into()));
}

/// nRF54L15 smart-ring probe (Cortex-M33, gcc -Os -g3).
#[test]
fn cortex_m_step_over_into_and_out() {
    let adapter = load(
        "examples/nrf54l15-smart-ring/system.yaml",
        "tests/fixtures/nrf54l15-smart-ring.elf",
    );
    set_line_breakpoint(&adapter, "src/main.c", 220);
    run_to_breakpoint(&adapter, 5_000_000);
    assert_eq!(here(&adapter), ("main.c".into(), 220, "main".into()));

    // 220 calls uart_puts: over runs it through to 223.
    assert_eq!(
        step(&adapter, StepKind::Over),
        (StepStop::LineChanged, 0x2ca)
    );
    assert_eq!(here(&adapter).1, 223);
    // 223 calls probe8 (an I2C transaction and a UART line): one step to 224.
    assert_eq!(
        step(&adapter, StepKind::Over),
        (StepStop::LineChanged, 0x2d6)
    );
    assert_eq!(here(&adapter), ("main.c".into(), 224, "main".into()));
    assert!(String::from_utf8_lossy(&adapter.poll_uart()).contains("BMI270"));

    // Into: a breakpoint on probe8's first line, then out to its caller.
    set_line_breakpoint(&adapter, "src/main.c", 177);
    run_to_breakpoint(&adapter, 5_000_000);
    assert_eq!(here(&adapter), ("main.c".into(), 177, "probe8".into()));
    let locals = adapter.source_locals().unwrap();
    let p = locals.iter().find(|l| l.name == "p").expect("local p");
    assert_eq!(p.kind, "frame_offset");
    // Out: back in main, past the call on 224. The next statement main
    // reaches is probe_tmp117's first line, which -Os inlined into main (it
    // has no symbol of its own), so DWARF names the inlined function.
    let (stop, _) = step(&adapter, StepKind::Out);
    assert_eq!(stop, StepStop::LineChanged);
    let (file, line, function) = here(&adapter);
    assert_eq!(
        (file.as_str(), function.as_str()),
        ("main.c", "probe_tmp117")
    );
    assert!((195..=210).contains(&line), "probe_tmp117 body, got {line}");
}

/// Arduino Uno (AVR): a breakpoint on loop()'s first line and a step over a
/// 4-byte CALL.
#[test]
fn avr_step_over_a_call() {
    let adapter = load(
        "configs/systems/arduino-uno.yaml",
        "tests/fixtures/source-debug/uno-arduino.elf",
    );
    set_line_breakpoint(&adapter, "src/main.ino", 26);
    assert_eq!(run_to_breakpoint(&adapter, 50_000_000), 0x692);
    assert_eq!(
        step(&adapter, StepKind::Over),
        (StepStop::LineChanged, 0x698)
    );
    assert_eq!(here(&adapter).1, 27);
    // Into enters digitalWrite.
    assert_eq!(run_to_breakpoint(&adapter, 50_000_000), 0x692);
    assert_eq!(
        step(&adapter, StepKind::Into),
        (StepStop::LineChanged, 0x10e)
    );
}

/// A step and the run-through inside it are journaled: a snapshot restore
/// replays them to the same instruction.
#[test]
fn source_steps_replay_from_a_snapshot() {
    let adapter = load(
        "examples/nrf54l15-smart-ring/system.yaml",
        "tests/fixtures/nrf54l15-smart-ring.elf",
    );
    set_line_breakpoint(&adapter, "src/main.c", 220);
    run_to_breakpoint(&adapter, 5_000_000);
    assert_eq!(step(&adapter, StepKind::Over).1, 0x2ca);
    let saved = adapter.snapshot_save(None).unwrap();
    assert_eq!(step(&adapter, StepKind::Over).1, 0x2d6);
    adapter.snapshot_restore(saved.id).unwrap();
    assert_eq!(adapter.get_pc().unwrap(), 0x2ca);
}

/// The server runs `continue` on a worker thread and steps on its request
/// thread. An Arduino-ESP32 boot leaves hooks in thread-locals; the run must
/// reach loop() from a thread other than the one that loaded it.
#[test]
fn esp32_arduino_runs_to_loop_from_another_thread() {
    let adapter = load(
        "configs/systems/esp32-wroom-32.yaml",
        "tests/fixtures/source-debug/esp32-arduino.elf",
    );
    set_line_breakpoint(&adapter, "src/main.ino", 27);
    let worker = adapter.clone();
    let pc = std::thread::spawn(move || run_to_breakpoint(&worker, 400_000_000))
        .join()
        .unwrap();
    assert_eq!(pc, 0x400d_147a);
    // And steps on this thread from there.
    assert_eq!(step(&adapter, StepKind::Over).0, StepStop::LineChanged);
    assert_eq!(here(&adapter).1, 28);
}

/// RISC-V (riscv-rt, Rust release build): step over runs the six
/// `write_volatile`s inlined from core in one step and lands on `loop {}`;
/// step into stops in the inlined `ptr/mod.rs`.
#[test]
fn riscv_step_over_runs_through_code_inlined_from_another_file() {
    let adapter = load(
        "configs/systems/ci-fixture-riscv-uart1.yaml",
        "tests/fixtures/riscv-ci-fixture.elf",
    );
    set_line_breakpoint(&adapter, "riscv-ci-fixture/src/main.rs", 12);
    assert_eq!(run_to_breakpoint(&adapter, 5_000_000), 0x8000_02ec);
    assert_eq!(
        step(&adapter, StepKind::Over),
        (StepStop::LineChanged, 0x8000_0320)
    );
    assert_eq!(here(&adapter).1, 24);

    let adapter = load(
        "configs/systems/ci-fixture-riscv-uart1.yaml",
        "tests/fixtures/riscv-ci-fixture.elf",
    );
    set_line_breakpoint(&adapter, "riscv-ci-fixture/src/main.rs", 12);
    run_to_breakpoint(&adapter, 5_000_000);
    assert_eq!(step(&adapter, StepKind::Into).0, StepStop::LineChanged);
    assert_eq!(here(&adapter).0, "mod.rs");
}
