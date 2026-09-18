// LabWired - Firmware Simulation Platform
// SPDX-License-Identifier: MIT

//! Exact-cycle reference for the browser board-demo firmware. WFE is modeled
//! identically with acceleration on/off; only empty sleep cycles are coalesced.
#![cfg(feature = "event-scheduler")]
use labwired_config::{ChipDescriptor, SystemManifest};
use labwired_core::{
    bus::SystemBus, cpu::CortexM, logic_capture::LogicSource, system::cortex_m::configure_cortex_m,
    AdvanceRequest, Cpu, Machine,
};
use std::{path::PathBuf, time::Instant};

fn machine(ff: bool, fixture: &str) -> Machine<CortexM> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let chip = ChipDescriptor::from_file(root.join("configs/chips/nrf54l15.yaml")).unwrap();
    let manifest = SystemManifest::from_file(root.join("configs/systems/nrf54l15dk.yaml")).unwrap();
    let mut bus = SystemBus::from_config(&chip, &manifest).unwrap();
    let (cpu, _) = configure_cortex_m(&mut bus);
    let mut m = Machine::new(cpu, bus);
    m.config.idle_fast_forward_enabled = ff;
    let image = labwired_loader::load_elf(&root.join("tests/fixtures").join(fixture)).unwrap();
    m.load_firmware(&image).unwrap();
    let gpio = m
        .bus
        .peripherals
        .iter()
        .position(|p| p.name == "gpio2")
        .unwrap();
    m.logic_watch(&[Some(LogicSource::pad(gpio, 6))]);
    m
}

#[test]
fn embassy_sleep_acceleration_preserves_gpio_edges_and_cpu_state() {
    check_fidelity("nrf54l15-embassy-blinky.elf", false);
    check_fidelity("nrf54l15-embassy-grtc.elf", true);
}

fn check_fidelity(fixture: &str, correct_clock: bool) {
    let mut reference = machine(false, fixture);
    let mut fast = machine(true, fixture);
    let cycles = 65_000_000; // > two 250 ms sleep windows at 128 MHz
    let mut elapsed = Vec::new();
    let mut edges = Vec::new();
    for m in [&mut reference, &mut fast] {
        let start = Instant::now();
        m.advance(AdvanceRequest::run(Some(cycles)).with_cycle_limit(cycles))
            .unwrap();
        elapsed.push(start.elapsed());
        let captured = m.logic_read_edges(0);
        assert_eq!(captured.dropped, 0);
        edges.push(
            captured
                .edges
                .iter()
                .map(|e| (e.ch, e.cycle, e.value))
                .collect::<Vec<_>>(),
        );
    }
    eprintln!(
        "reference={:?} accelerated={:?} skipped={} edges={:?}",
        elapsed[0], elapsed[1], fast.idle_fast_forward_cycles_skipped, edges[1]
    );
    assert_eq!(fast.total_cycles, reference.total_cycles);
    assert_eq!(
        edges[0], edges[1],
        "GPIO edges must match at exact simulated cycles"
    );
    assert!(edges[1].len() >= 3, "must actually toggle twice");
    if correct_clock {
        assert_eq!(edges[1].len(), 3);
        for pair in edges[1].windows(2) {
            let interval = pair[1].1 - pair[0].1;
            assert!(
                (32_000_000..32_010_000).contains(&interval),
                "250 ms at 128 MHz plus ISR/executor latency, got {interval}"
            );
        }
    }
    for name in ["grtc", "gpio1", "gpio2", "nvic", "scb"] {
        let peripheral = |m: &Machine<CortexM>| {
            m.bus
                .peripherals
                .iter()
                .find(|p| p.name == name)
                .unwrap()
                .dev
                .snapshot()
        };
        assert_eq!(peripheral(&reference), peripheral(&fast), "{name}");
    }
    assert_eq!(
        serde_json::to_value(reference.cpu.snapshot()).unwrap(),
        serde_json::to_value(fast.cpu.snapshot()).unwrap()
    );
    assert_eq!(reference.cpu.basepri, fast.cpu.basepri);
    assert_eq!(reference.cpu.faultmask, fast.cpu.faultmask);
    assert_eq!(reference.cpu.fpu_s, fast.cpu.fpu_s);
    assert_eq!(reference.bus.ram.data, fast.bus.ram.data, "SRAM must match");
    assert!(fast.idle_fast_forward_cycles_skipped > 60_000_000);
}
