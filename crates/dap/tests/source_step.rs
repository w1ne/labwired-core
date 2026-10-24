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
    // Out: back in main. Its next statement is the call to probe_tmp117,
    // which -Os inlined (it has no symbol of its own): the stop shows the
    // line in main that makes the call, not the inlined body.
    let (stop, _) = step(&adapter, StepKind::Out);
    assert_eq!(stop, StepStop::LineChanged);
    assert_eq!(here(&adapter), ("main.c".into(), 225, "main".into()));
    // Into enters the inlined probe_tmp117 without running anything.
    let (stop, _) = step(&adapter, StepKind::Into);
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

/// Nokia 5110 starter (STM32L476, Rust release build): most of `main`'s loop
/// is inlined and has no line-table row of its own. Step over walks the
/// loop's lines (each inlined call is shown on the line that makes it),
/// into enters an inlined function, out returns to its caller, and a
/// breakpoint on an inlined call's line stops on that line.
#[test]
fn cortex_m_rust_steps_through_inlined_calls() {
    let adapter = load(
        "examples/nokia5110-invaders-lab/system.yaml",
        "tests/fixtures/nokia5110-invaders-lab.elf",
    );
    set_line_breakpoint(&adapter, "src/main.rs", 508);
    run_to_breakpoint(&adapter, 50_000_000);
    let main = here(&adapter).2;
    assert!(main.ends_with("__cortex_m_rt_main"), "{main}");
    assert_eq!(here(&adapter).1, 508);

    // 508 calls render; 509 (lcd_frame) and 511 (delay) are inlined.
    for line in [509, 511, 491] {
        assert_eq!(step(&adapter, StepKind::Over).0, StepStop::LineChanged);
        assert_eq!(here(&adapter), ("main.rs".into(), line, main.clone()));
    }
    // 491 is `let count = measure();`: into enters the inlined measure.
    assert_eq!(
        step(&adapter, StepKind::Into),
        (StepStop::LineChanged, 0x0800_0f6a)
    );
    let (file, line, function) = here(&adapter);
    assert_eq!(file, "main.rs");
    assert!(function.ends_with("::measure"), "{function}");
    assert!((223..=248).contains(&line), "measure body, got {line}");
    // Out finishes measure and is back in main, past the call.
    assert_eq!(step(&adapter, StepKind::Out).0, StepStop::LineChanged);
    assert_eq!(here(&adapter), ("main.rs".into(), 493, main.clone()));

    // A breakpoint on an inlined call's line stops there, shown on that line.
    set_line_breakpoint(&adapter, "src/main.rs", 509);
    run_to_breakpoint(&adapter, 50_000_000);
    assert_eq!(here(&adapter), ("main.rs".into(), 509, main));
}

/// The field names of `struct Game` in the Nokia example's source.
fn game_fields() -> Vec<String> {
    let src = std::fs::read_to_string(root().join("examples/nokia5110-invaders-lab/src/main.rs"))
        .unwrap();
    let body = src.split("struct Game {").nth(1).unwrap();
    let body = &body[..body.find('}').unwrap()];
    body.lines()
        .filter_map(|l| l.trim().split_once(':').map(|(n, _)| n.trim().to_string()))
        .filter(|n| !n.is_empty() && !n.starts_with("//"))
        .collect()
}

fn var<'a>(
    vars: &'a [labwired_loader::VariableValue],
    name: &str,
) -> &'a labwired_loader::VariableValue {
    vars.iter()
        .find(|v| v.name == name)
        .unwrap_or_else(|| panic!("no variable {name} in {vars:#?}"))
}

/// (function, file name, line, inlined) per call-stack frame.
fn stack(adapter: &LabwiredAdapter) -> Vec<(String, String, Option<u32>, bool)> {
    adapter
        .call_stack(32)
        .unwrap()
        .into_iter()
        .map(|f| {
            (
                f.function.unwrap_or_default(),
                f.file
                    .map(|p| p.rsplit('/').next().unwrap().to_string())
                    .unwrap_or_default(),
                f.line,
                f.inlined,
            )
        })
        .collect()
}

/// Checks `game` (a `Game` in main's stack frame) against the source.
fn assert_game(game: &labwired_loader::VariableValue) {
    assert!(game.available, "{game:#?}");
    assert_eq!(game.type_name.as_deref(), Some("Game"));
    assert!(
        game.value.starts_with("Game { paddle_x: "),
        "{}",
        game.value
    );
    let names: Vec<&str> = game.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, game_fields());
    let paddle: i32 = var(&game.children, "paddle_x").value.parse().unwrap();
    assert!((0..84).contains(&paddle), "paddle_x {paddle}");
    let vx: i32 = var(&game.children, "vx").value.parse().unwrap();
    assert!(vx == 1 || vx == -1, "vx {vx}");
    // bricks: [[bool; BCOLS]; BROWS], each brick true or false.
    let bricks = var(&game.children, "bricks");
    assert_eq!(bricks.value, "[[bool; 6]; 3]");
    assert_eq!(bricks.children.len(), 3);
    for row in &bricks.children {
        assert_eq!(row.children.len(), 6);
        for b in &row.children {
            assert!(b.value == "true" || b.value == "false", "{b:?}");
        }
    }
    let addr = game.address.expect("game lives on the stack");
    assert!((0x2000_0000..0x2002_0000).contains(&addr), "{addr:#x}");
}

/// Nokia 5110 starter (Rust release build): typed variables and the call
/// stack at `game.render(&mut FB)` (main.rs:508), in `render` after a step
/// into that real call, and in the inlined `measure`.
#[test]
fn cortex_m_rust_variables_and_call_stack() {
    let adapter = load(
        "examples/nokia5110-invaders-lab/system.yaml",
        "tests/fixtures/nokia5110-invaders-lab.elf",
    );
    set_line_breakpoint(&adapter, "src/main.rs", 508);
    run_to_breakpoint(&adapter, 50_000_000);

    // `game` is DW_OP_breg13 (sp) + offset: a stack slot, not optimized out.
    let vars = adapter.frame_variables(0).unwrap();
    assert_game(var(&vars, "game"));
    for name in ["c1", "c2"] {
        let v = var(&vars, name);
        assert!(v.available, "{v:?}");
        assert_eq!(v.type_name.as_deref(), Some("u32"));
    }
    // `paddle` is in a register here.
    let paddle = var(&vars, "paddle");
    assert!(paddle.available && paddle.address.is_none(), "{paddle:?}");

    let frames = stack(&adapter);
    assert!(frames[0].0.ends_with("::__cortex_m_rt_main"), "{frames:?}");
    assert_eq!((frames[0].2, frames[0].3), (Some(508), false));
    // The cortex-m-rt trampoline `main` calls it.
    assert_eq!(frames[1].0, "main", "{frames:?}");

    // Into the real call `render`: the stack is render <- main, the caller
    // shown on the line of the call, its variables still readable.
    assert_eq!(step(&adapter, StepKind::Into).0, StepStop::LineChanged);
    let frames = stack(&adapter);
    assert!(frames[0].0.ends_with("Game::render"), "{frames:?}");
    assert!(!frames[0].3);
    assert!(frames[1].0.ends_with("::__cortex_m_rt_main"), "{frames:?}");
    assert_eq!(
        (frames[1].1.as_str(), frames[1].2, frames[1].3),
        ("main.rs", Some(508), false)
    );
    assert_eq!(frames[2].0, "main");
    let caller = adapter.frame_variables(1).unwrap();
    assert_game(var(&caller, "game"));
    // `self` is `&Game`: the pointer, with the Game it points at below.
    let this = var(&adapter.frame_variables(0).unwrap(), "self").clone();
    assert_eq!(
        this.type_name.as_deref(),
        Some("&nokia5110_invaders_lab::Game")
    );
    assert!(this.value.starts_with("0x2"), "{}", this.value);
    let names: Vec<&str> = this.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, game_fields());

    // The inlined `measure` (main.rs:491): measure (inlined) above main.
    let adapter = load(
        "examples/nokia5110-invaders-lab/system.yaml",
        "tests/fixtures/nokia5110-invaders-lab.elf",
    );
    set_line_breakpoint(&adapter, "src/main.rs", 508);
    run_to_breakpoint(&adapter, 50_000_000);
    for _ in [509, 511, 491] {
        assert_eq!(step(&adapter, StepKind::Over).0, StepStop::LineChanged);
    }
    assert_eq!(step(&adapter, StepKind::Into).0, StepStop::LineChanged);
    let frames = stack(&adapter);
    assert!(frames[0].0.ends_with("::measure"), "{frames:?}");
    assert!(frames[0].3, "measure is inlined");
    assert!(frames[1].0.ends_with("::__cortex_m_rt_main"), "{frames:?}");
    assert_eq!((frames[1].2, frames[1].3), (Some(491), false));
    // Frame 0's variables are measure's own; main's are one frame down.
    let vars = adapter.frame_variables(1).unwrap();
    assert_game(var(&vars, "game"));
    let filt = var(&vars, "filt");
    assert!(filt.available, "{filt:?}");
    assert!(!adapter
        .frame_variables(0)
        .unwrap()
        .iter()
        .any(|v| v.name == "game"));
}

/// nRF54L15 smart-ring (C, gcc -Os): step into `probe8` from main.c:223 and
/// read its parameters, which -Os keeps in registers and location lists.
#[test]
fn cortex_m_c_parameters_and_call_stack() {
    let adapter = load(
        "examples/nrf54l15-smart-ring/system.yaml",
        "tests/fixtures/nrf54l15-smart-ring.elf",
    );
    set_line_breakpoint(&adapter, "src/main.c", 223);
    run_to_breakpoint(&adapter, 5_000_000);
    assert_eq!(step(&adapter, StepKind::Into).0, StepStop::LineChanged);
    assert_eq!(here(&adapter), ("main.c".into(), 177, "probe8".into()));

    // main.c:223 is `probe8("imu    BMI270   ", 0x68, 0x00, 0x24);`.
    let vars = adapter.frame_variables(0).unwrap();
    let label = var(&vars, "label");
    assert_eq!(label.type_name.as_deref(), Some("const char *"));
    assert!(
        label.value.ends_with(r#""imu    BMI270   ""#),
        "{}",
        label.value
    );
    for (name, value) in [("addr", "104"), ("reg", "0"), ("expect", "36")] {
        let v = var(&vars, name);
        assert_eq!((v.value.as_str(), v.available), (value, true), "{name}");
        assert_eq!(v.type_name.as_deref(), Some("uint8_t"));
    }

    let frames = stack(&adapter);
    assert_eq!(
        frames[0],
        ("probe8".into(), "main.c".into(), Some(177), false)
    );
    assert_eq!(
        frames[1],
        ("main".into(), "main.c".into(), Some(223), false)
    );

    // Over twim_read_reg: `acked` holds its result.
    assert_eq!(step(&adapter, StepKind::Over).0, StepStop::LineChanged);
    let vars = adapter.frame_variables(0).unwrap();
    let acked = var(&vars, "acked");
    assert_eq!((acked.value.as_str(), acked.available), ("1", true));
    assert_eq!(var(&vars, "addr").value, "104");
}
