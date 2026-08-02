// LabWired - Firmware Simulation Platform
// Copyright (C) 2026 Andrii Shylenko
// SPDX-License-Identifier: MIT

//! Timing harness for logic-analyzer capture overhead.
//!
//! Not a pass/fail benchmark — it prints wall-clock numbers for capture
//! disarmed vs armed (4 and 8 watched channels), in BOTH capture modes
//! (event-driven push — the default for instrumented peripherals — and the
//! forced per-cycle poll fallback via `Machine::logic_force_poll_capture`),
//! so the cost of probing stays measured, not guessed. Two scenarios:
//!
//! 1. A real firmware fixture (`stm32f103-blinky.elf`) driven through
//!    `Machine::run` in wasm-worker-style batches at the default
//!    `peripheral_tick_interval = 1` — the configuration every playground
//!    run uses.
//! 2. A synthetic NOP loop on a bare bus at `peripheral_tick_interval = 64`
//!    — the maximum-batching case, where arming POLL capture clamps the
//!    run-loop batch and the clamp cost is at its worst; push capture keeps
//!    the full batch width and should track the unarmed time.
//!
//! Run manually with:
//!
//! ```sh
//! cargo test --release -p labwired-core --test logic_capture_bench -- --ignored --nocapture
//! ```

use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::bus::SystemBus;
use labwired_core::peripherals::gpio::{GpioPort, GpioRegisterLayout};
use labwired_core::system::cortex_m::configure_cortex_m;
use labwired_core::Bus;
use labwired_core::{cpu::CortexM, DebugControl, Machine};
use std::path::PathBuf;
use std::time::Instant;

const FIRMWARE_STEPS: u64 = 2_000_000;
const SYNTHETIC_STEPS: u64 = 20_000_000;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Real firmware on the real board model, default (cycle-accurate) config.
fn build_firmware_machine() -> Machine<CortexM> {
    let root = workspace_root();
    let chip =
        ChipDescriptor::from_file(root.join("configs/chips/stm32f103.yaml")).expect("chip config");
    let sys_path = root.join("configs/systems/stm32f103-bare.yaml");
    let mut manifest = SystemManifest::from_file(&sys_path).expect("system config");
    manifest.chip = sys_path
        .parent()
        .unwrap()
        .join(&manifest.chip)
        .to_str()
        .unwrap()
        .to_string();

    let mut bus = SystemBus::from_config(&chip, &manifest).expect("bus from config");
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    let mut machine = Machine::new(cpu, bus);

    let image = labwired_loader::load_elf(&root.join("tests/fixtures/stm32f103-blinky.elf"))
        .expect("load fixture ELF");
    machine.load_firmware(&image).expect("load firmware");
    machine
}

/// Bare bus + one GPIO port + a 1001-instruction NOP loop in RAM, ticked every
/// 64 cycles so `Machine::run` batches as widely as it ever does.
fn build_synthetic_machine() -> Machine<CortexM> {
    const GPIO_BASE: u64 = 0x5000_0000;
    const RAM_BASE: u64 = 0x2000_0000;

    let mut bus = SystemBus::new();
    let (cpu, _nvic) = configure_cortex_m(&mut bus);
    bus.add_peripheral(
        "gpio_bench",
        GPIO_BASE,
        0x400,
        None,
        Box::new(GpioPort::new_with_layout(GpioRegisterLayout::Stm32V2)),
    );
    let mut machine = Machine::new(cpu, bus);
    machine.config.peripheral_tick_interval = 64;

    // MODER pin0..7 = output so read_gpio_pad reflects ODR.
    machine
        .bus
        .write_u32(GPIO_BASE, 0x0000_5555) // V2 MODER @ 0x00
        .unwrap();

    // 1000 x `movs r0, #0` (0x2000) then `b` back to the start (imm11 = -1002
    // halfwords), i.e. a 1001-instruction idle loop.
    for i in 0..1000u64 {
        machine
            .bus
            .write_u16(RAM_BASE + i * 2, 0x2000)
            .expect("write nop");
    }
    machine
        .bus
        .write_u16(
            RAM_BASE + 2000,
            0xE000 | ((-1002i32 as u32 as u16) & 0x07FF),
        )
        .expect("write branch");
    machine.cpu.pc = RAM_BASE as u32;
    machine
}

/// First `n` readable GPIO pads on the bus, as `(peripheral_index, pin)`.
fn readable_pads(machine: &Machine<CortexM>, n: usize) -> Vec<Option<(usize, u8)>> {
    let mut out = Vec::new();
    for (idx, p) in machine.bus.peripherals.iter().enumerate() {
        for pin in 0..16u8 {
            if p.dev.read_gpio_pad(pin).is_some() {
                out.push(Some((idx, pin)));
                if out.len() == n {
                    return out;
                }
            }
        }
    }
    panic!("only found {} readable pads, wanted {}", out.len(), n);
}

fn timed_run(
    mut machine: Machine<CortexM>,
    channels: usize,
    steps: u64,
    force_poll: bool,
) -> (f64, usize, u64) {
    if channels > 0 {
        machine.logic_force_poll_capture(force_poll);
        let pads = readable_pads(&machine, channels);
        machine.logic_watch(&pads);
    }
    // Drive the machine the way the wasm worker does: repeated `run` batches
    // until the target cycle count is reached (early `StepDone` returns are
    // normal at branch/exception boundaries).
    let start = Instant::now();
    while machine.get_cycle_count() < steps {
        let before = machine.get_cycle_count();
        let remaining = (steps - before).min(u32::MAX as u64) as u32;
        machine.run(Some(remaining)).expect("run");
        assert!(
            machine.get_cycle_count() > before,
            "no forward progress at cycle {before}"
        );
    }
    let secs = start.elapsed().as_secs_f64();
    let batch = machine.logic_read_edges(0);
    (secs, batch.edges.len(), batch.dropped)
}

type BuildMachine = fn() -> Machine<CortexM>;

#[test]
#[ignore = "manual timing harness, run with --release --nocapture"]
fn bench_logic_capture_overhead() {
    let scenarios: [(&str, BuildMachine, u64); 2] = [
        ("firmware tick=1 ", build_firmware_machine, FIRMWARE_STEPS),
        (
            "synthetic tick=64",
            build_synthetic_machine,
            SYNTHETIC_STEPS,
        ),
    ];
    for (name, build, steps) in scenarios {
        let (base, edges, dropped) = timed_run(build(), 0, steps, false);
        println!(
            "{name} channels=0 (unarmed)   : {base:.3}s \
             ({:.2} Minstr/s) edges={edges} dropped={dropped}",
            steps as f64 / base / 1e6,
        );
        for channels in [4usize, 8] {
            for (mode, force_poll) in [("push", false), ("poll", true)] {
                let (secs, edges, dropped) = timed_run(build(), channels, steps, force_poll);
                println!(
                    "{name} channels={channels} mode={mode} : {secs:.3}s \
                     ({:.2} Minstr/s, {:.2}x vs unarmed) edges={edges} dropped={dropped}",
                    steps as f64 / secs / 1e6,
                    secs / base,
                );
            }
        }
    }
}
